//! Calls screen: call history (CSA, `48:calllogs`) and voicemail (Outlook); request ids discard stale responses.
use super::*;
use serde_json::json;
use teams_calling::history::{self, CALL_LOGS};
use teams_calling::{CallDirection, CallLog, CallState, Voicemail, VoicemailClient};
use teams_i18n::t;
use teams_realtime::{RealtimeConfig, RealtimeEvent};

use super::push::{Incoming, PushChannel, PushTexts, Transition};

const HISTORY_PAGE: u32 = 50;
const VOICEMAIL_PAGE: u32 = 30;
const REFRESH: Duration = Duration::from_secs(60);

const UNKNOWN_CALLER: &str = "core.call.unknown_caller";
fn unknown_caller() -> &'static str {
    t(UNKNOWN_CALLER)
}
const PUSH_TEXTS: PushTexts = PushTexts {
    connecting: "core.calls.connecting_to_the_calls_channel",
    registering: "core.calls.connected_registering",
    ready: "core.calls.ready_to_receive_calls",
    refused: "core.calls.registration_refused",
    retrying: "core.calls.retrying_in_s",
    stopped: "core.calls.calls_channel_stopped",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CallDirectionView {
    Incoming,
    Outgoing,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum CallOutcome {
    Missed,
    Answered,
    Declined,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CallLogView {
    pub id: String,
    /// The other side: a person, a number, or the group chat.
    pub name: String,
    /// MRI to call back; empty when unknown.
    pub peer: String,
    pub direction: CallDirectionView,
    pub outcome: CallOutcome,
    /// ISO-8601 start time.
    pub started_at: String,
    pub duration_secs: Option<i64>,
    pub group: bool,
    /// The person or chat can be called back natively.
    pub can_call_back: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct VoicemailView {
    pub id: String,
    pub sender: String,
    pub address: String,
    pub received: String,
    pub read: bool,
    pub duration_secs: Option<u32>,
    pub transcription: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CallsView {
    pub history: Vec<CallLogView>,
    pub voicemails: Vec<VoicemailView>,
    pub loading_history: bool,
    pub loading_voicemails: bool,
    pub has_more: bool,
    pub error: Option<String>,
    pub voicemail_error: Option<String>,
    /// Voicemail whose audio is ready for `OxideCore::take_voicemail_audio`.
    pub audio_ready: Option<String>,
    /// Voicemail whose audio is downloading.
    pub audio_loading: Option<String>,
    /// State of the push channel, for diagnostics; empty when stopped.
    pub realtime: String,
}
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CallsAction {
    Refresh,
    More,
    DeleteLog {
        id: String,
    },
    /// Calls the other side of a logged call again: the person, or the group chat.
    CallBack {
        id: String,
        video: bool,
    },
    PlayVoicemail {
        id: String,
    },
    MarkVoicemail {
        id: String,
        read: bool,
    },
    DeleteVoicemail {
        id: String,
    },
}
#[derive(Debug, Clone, uniffi::Record)]
pub struct VoicemailAudio {
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryPage {
    logs: Vec<CallLog>,
    /// `originalArrivalTime` of the oldest message, the cursor of the next page.
    oldest: Option<String>,
    full: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum CallsMsg {
    Action(CallsAction),
    History(u64, u64, bool, Result<HistoryPage, String>),
    Voicemails(u64, u64, Result<Vec<Voicemail>, String>),
    Audio(u64, String, Result<(String, Vec<u8>), String>),
    Mutated(u64, Result<(), String>),
    Realtime(u64, RealtimeEvent),
}
#[derive(Default)]
pub(crate) struct CallsState {
    voicemail: Option<VoicemailClient<AppAuthenticator>>,
    pub view: CallsView,
    logs: Vec<CallLog>,
    oldest: Option<String>,
    history_request: u64,
    voicemail_request: u64,
    pub(crate) refreshed: Option<std::time::Instant>,
    pub(crate) push: PushChannel,
    /// `cor_id` of the push channel, also the callback session segment.
    pub(crate) session: String,
    /// Public URL of the registered push session; calls need it for callbacks.
    pub(crate) surl: Option<String>,
    /// Shared with the push channel: faster pings while a call runs.
    pub(crate) in_call: Arc<teams_realtime::CallFlag>,
}

async fn fetch_history(client: Client, before: Option<String>) -> Result<HistoryPage, String> {
    let messages = match client.call_history(before.as_deref(), HISTORY_PAGE).await {
        Ok(page) => page.messages,
        // The CSA route is unverified: the chat service stream is the documented store.
        Err(csa) if before.is_none() => {
            let query = MessagesQuery {
                page_size: Some(HISTORY_PAGE),
                ..Default::default()
            };
            client
                .messages(CALL_LOGS, &query)
                .await
                .map_err(|_| csa.to_string())?
                .messages
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok(HistoryPage {
        full: messages.len() as u32 >= HISTORY_PAGE,
        oldest: messages
            .iter()
            .filter_map(|m| m.originalarrivaltime.clone())
            .min(),
        logs: history::visible(messages.iter().map(log_message)),
    })
}

/// A message of `48:calllogs`, as the calling crate reads it.
pub(crate) fn log_message(message: &Message) -> history::LogMessage<'_> {
    history::LogMessage {
        id: &message.id,
        original_arrival_time: message.originalarrivaltime.as_deref(),
        call_log: message.properties.get("call-log"),
        is_deleted: message.is_deleted(),
    }
}

impl App {
    fn voicemail_client(&mut self) -> Option<VoicemailClient<AppAuthenticator>> {
        if self.calls.voicemail.is_none() {
            self.calls.voicemail = self
                .client
                .as_ref()
                .map(|c| VoicemailClient::new(c.authenticator()));
        }
        self.calls.voicemail.clone()
    }

    pub(crate) fn load_calls(&mut self, force: bool) -> Task<Msg> {
        if *self.nav != Nav::Calls
            || (!force
                && (self.calls.view.loading_history
                    || self.calls.refreshed.is_some_and(|t| t.elapsed() < REFRESH)))
        {
            return Task::none();
        }
        let (Some(client), Some(voicemail)) = (self.client.clone(), self.voicemail_client()) else {
            return Task::none();
        };
        self.calls.refreshed = Some(std::time::Instant::now());
        self.calls.history_request += 1;
        self.calls.voicemail_request += 1;
        let (g, h, v) = (
            self.generation,
            self.calls.history_request,
            self.calls.voicemail_request,
        );
        self.calls.view.loading_history = true;
        self.calls.view.loading_voicemails = true;
        Task::batch([
            Task::perform(fetch_history(client, None), move |r| {
                Msg::Calls(CallsMsg::History(g, h, false, r))
            }),
            Task::perform(
                async move {
                    voicemail
                        .list(VOICEMAIL_PAGE)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |r| Msg::Calls(CallsMsg::Voicemails(g, v, r)),
            ),
        ])
    }

    fn log_view(&self, log: &CallLog) -> CallLogView {
        let peer = log.counterpart(&self.me);
        let group = log.call_type.as_deref() == Some("MultiParty")
            || log.group_chat_thread_id.is_some() && log.participants.len() > 2;
        let name = peer
            .and_then(|p| {
                p.display_name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .or_else(|| self.known_name(&p.id))
                    .or_else(|| p.id.strip_prefix("4:").map(str::to_owned))
            })
            .or_else(|| {
                log.group_chat_thread_id
                    .as_ref()
                    .and_then(|id| self.chats.iter().find(|c| &c.id == id))
                    .map(|c| chat_title(c, &self.me))
            })
            .unwrap_or_else(|| {
                if group {
                    t("core.calls.group_call").into()
                } else {
                    unknown_caller().into()
                }
            });
        CallLogView {
            id: log.call_id.clone(),
            name,
            peer: if group {
                String::new()
            } else {
                peer.map(|p| p.id.clone()).unwrap_or_default()
            },
            direction: match log.direction {
                CallDirection::Incoming => CallDirectionView::Incoming,
                CallDirection::Outgoing => CallDirectionView::Outgoing,
                CallDirection::Unknown => CallDirectionView::Unknown,
            },
            outcome: match log.state {
                CallState::Missed => CallOutcome::Missed,
                CallState::Accepted => CallOutcome::Answered,
                CallState::Declined => CallOutcome::Declined,
                CallState::Other(_) => CallOutcome::Other,
            },
            started_at: log.start_time.clone().unwrap_or_default(),
            duration_secs: log.duration_secs(),
            group,
            can_call_back: !group
                && peer.is_some_and(|p| p.id.starts_with("8:") || p.id.starts_with("4:"))
                || log.group_chat_thread_id.is_some(),
        }
    }

    fn refresh_history_view(&mut self) {
        self.calls.view.history = self.calls.logs.iter().map(|l| self.log_view(l)).collect();
    }

    /// Rebuilds the cached view texts in the current language.
    pub(crate) fn relocalize_calls(&mut self) {
        self.refresh_history_view();
        for v in &mut self.calls.view.voicemails {
            if let Some(text) = relocalized(&v.sender, &[UNKNOWN_CALLER]) {
                v.sender = text.into();
            }
        }
        if let Some(text) = PUSH_TEXTS.relocalized(&self.calls.view.realtime) {
            self.calls.view.realtime = text.into();
        }
    }

    /// Drops the push channel and connects a new one (same endpoint and session).
    pub(crate) fn restart_realtime(&mut self) -> Task<Msg> {
        if !self.calls.push.halt() {
            return Task::none();
        }
        let surl = self.calls.surl.clone();
        let session = self.calls.session.clone();
        let task = self.sync_realtime();
        // Keep the session id: callbacks only change if Trouter hands out another URL.
        self.calls.session = session;
        self.calls.surl = surl;
        task
    }

    pub(crate) fn sync_realtime(&mut self) -> Task<Msg> {
        let Some(client) = self.client.as_ref() else {
            if self.calls.push.halt() {
                self.calls.view.realtime.clear();
                self.calls.surl = None;
            }
            return Task::none();
        };
        if self.calls.push.running() {
            return Task::none();
        }
        let auth = client.authenticator();
        let g = self.generation;
        let calls = &mut *self.calls;
        calls.session = uuid::Uuid::new_v4().to_string();
        calls.surl = None;
        let session = calls.session.clone();
        let in_call = calls.in_call.clone();
        let task = calls.push.start(
            auth,
            |endpoint| {
                let mut config = RealtimeConfig::calling(endpoint, session);
                config.in_call = in_call;
                config
            },
            move |event| Msg::Calls(CallsMsg::Realtime(g, event)),
        );
        calls.view.realtime = t(PUSH_TEXTS.connecting).into();
        task
    }

    fn realtime_event(&mut self, event: RealtimeEvent) -> Task<Msg> {
        let change = match Incoming::from(event) {
            Incoming::Change(change) => change,
            Incoming::Request(request) => return self.call_pushed(&request),
            Incoming::MessageLoss => return Task::none(),
            Incoming::Diagnostic(message) => {
                teams_calling::diagnostics::record(&format!("Trouter {message}"));
                return Task::none();
            }
        };
        let status = match self.calls.push.apply(&PUSH_TEXTS, change) {
            Transition::Connected { surl, status } => {
                let moved = self.calls.surl.as_ref().is_some_and(|old| *old != surl);
                self.calls.surl = Some(surl.clone());
                self.calls.view.realtime = status;
                // A new session URL: the active call's callbacks must follow it.
                return if moved {
                    self.call_moved(surl)
                } else {
                    Task::none()
                };
            }
            Transition::Registered { status, .. } | Transition::Down(status) => status,
            Transition::Stopped(status) => {
                self.calls.surl = None;
                status
            }
        };
        teams_calling::diagnostics::record(&format!("Trouter {status}"));
        self.calls.view.realtime = status;
        Task::none()
    }

    pub(crate) fn update_calls(&mut self, msg: CallsMsg) -> Task<Msg> {
        match msg {
            CallsMsg::History(g, r, more, result) => {
                if g != self.generation || r != self.calls.history_request {
                    return Task::none();
                }
                self.calls.view.loading_history = false;
                match result {
                    Ok(page) => {
                        if more {
                            let known: HashSet<String> =
                                self.calls.logs.iter().map(|l| l.call_id.clone()).collect();
                            self.calls.logs.extend(
                                page.logs
                                    .into_iter()
                                    .filter(|l| !known.contains(&l.call_id)),
                            );
                        } else {
                            self.calls.logs = page.logs;
                        }
                        self.calls.oldest = page.oldest.or(self.calls.oldest.take());
                        self.calls.view.has_more = page.full;
                        self.calls.view.error = None;
                        self.refresh_history_view();
                    }
                    Err(e) => self.calls.view.error = Some(e),
                }
            }
            CallsMsg::Voicemails(g, r, result) => {
                if g != self.generation || r != self.calls.voicemail_request {
                    return Task::none();
                }
                self.calls.view.loading_voicemails = false;
                match result {
                    Ok(list) => {
                        self.calls.view.voicemail_error = None;
                        self.calls.view.voicemails = list
                            .into_iter()
                            .map(|v| VoicemailView {
                                sender: v
                                    .sender_name
                                    .clone()
                                    .or_else(|| v.sender_address.clone())
                                    .unwrap_or_else(|| unknown_caller().into()),
                                address: v.sender_address.unwrap_or_default(),
                                received: v.received.unwrap_or_default(),
                                read: v.is_read,
                                duration_secs: v.duration_secs,
                                transcription: v.transcription,
                                id: v.id,
                            })
                            .collect();
                    }
                    Err(e) => self.calls.view.voicemail_error = Some(e),
                }
            }
            CallsMsg::Audio(g, id, result) => {
                if g != self.generation || self.calls.view.audio_loading.as_deref() != Some(&id) {
                    return Task::none();
                }
                self.calls.view.audio_loading = None;
                match result {
                    Ok((kind, data)) => {
                        self.calls.view.audio_ready = Some(id.clone());
                        self.handoff().offer_voicemail(id.clone(), kind, data);
                        // Listening marks the message read, as in Teams.
                        return self.update_calls(CallsMsg::Action(CallsAction::MarkVoicemail {
                            id,
                            read: true,
                        }));
                    }
                    Err(e) => self.calls.view.voicemail_error = Some(e),
                }
            }
            CallsMsg::Mutated(g, result) => {
                if g != self.generation {
                    return Task::none();
                }
                if let Err(e) = result {
                    self.calls.view.error = Some(e);
                    return self.load_calls(true);
                }
            }
            CallsMsg::Realtime(g, event) => {
                if g != self.generation || !self.calls.push.running() {
                    return Task::none();
                }
                return self.realtime_event(event);
            }
            CallsMsg::Action(action) => {
                if self.client.is_none() {
                    return Task::none();
                }
                let g = self.generation;
                match action {
                    CallsAction::Refresh => {
                        self.calls.view.error = None;
                        self.calls.view.voicemail_error = None;
                        return self.load_calls(true);
                    }
                    CallsAction::More => {
                        if self.calls.view.loading_history || !self.calls.view.has_more {
                            return Task::none();
                        }
                        let Some(client) = self.client.clone() else {
                            return Task::none();
                        };
                        self.calls.history_request += 1;
                        let r = self.calls.history_request;
                        self.calls.view.loading_history = true;
                        let before = self.calls.oldest.clone();
                        return Task::perform(fetch_history(client, before), move |v| {
                            Msg::Calls(CallsMsg::History(g, r, true, v))
                        });
                    }
                    CallsAction::CallBack { id, video } => {
                        let Some(log) = self.calls.logs.iter().find(|l| l.call_id == id) else {
                            return Task::none();
                        };
                        let group = log.call_type.as_deref() == Some("MultiParty");
                        let chat = log.group_chat_thread_id.clone();
                        let peer = log.counterpart(&self.me).map(|p| p.id.clone());
                        let action = match (group, chat, peer) {
                            (true, Some(chat), _) | (_, Some(chat), None) => {
                                call::CallAction::CallChat {
                                    chat_id: Some(chat),
                                    video,
                                }
                            }
                            (_, _, Some(mri)) => call::CallAction::Start { mri, video },
                            _ => return Task::none(),
                        };
                        return self.update_call(call::CallMsg::Action(action));
                    }
                    CallsAction::DeleteLog { id } => {
                        let Some(index) = self.calls.logs.iter().position(|l| l.call_id == id)
                        else {
                            return Task::none();
                        };
                        let log = self.calls.logs.remove(index);
                        self.refresh_history_view();
                        let client = self.client.clone().unwrap();
                        return Task::perform(
                            async move {
                                let body = json!({ "call-log": log.deleted_property() });
                                for message in log.message_ids() {
                                    client
                                        .set_message_property(
                                            CALL_LOGS, message, "call-log", &body, true,
                                        )
                                        .await
                                        .map_err(|e| e.to_string())?;
                                }
                                Ok(())
                            },
                            move |r| Msg::Calls(CallsMsg::Mutated(g, r)),
                        );
                    }
                    CallsAction::PlayVoicemail { id } => {
                        let Some(client) = self.voicemail_client() else {
                            return Task::none();
                        };
                        self.calls.view.audio_loading = Some(id.clone());
                        self.calls.view.audio_ready = None;
                        self.handoff().withdraw_voicemail();
                        return Task::perform(
                            {
                                let id = id.clone();
                                async move { client.audio(&id).await.map_err(|e| e.to_string()) }
                            },
                            move |r| Msg::Calls(CallsMsg::Audio(g, id, r)),
                        );
                    }
                    CallsAction::MarkVoicemail { id, read } => {
                        let Some(v) = self.calls.view.voicemails.iter_mut().find(|v| v.id == id)
                        else {
                            return Task::none();
                        };
                        if v.read == read {
                            return Task::none();
                        }
                        v.read = read;
                        let client = self.voicemail_client().unwrap();
                        return Task::perform(
                            async move { client.set_read(&id, read).await.map_err(|e| e.to_string()) },
                            move |r| Msg::Calls(CallsMsg::Mutated(g, r)),
                        );
                    }
                    CallsAction::DeleteVoicemail { id } => {
                        let before = self.calls.view.voicemails.len();
                        self.calls.view.voicemails.retain(|v| v.id != id);
                        if self.calls.view.voicemails.len() == before {
                            return Task::none();
                        }
                        if self.calls.view.audio_ready.as_deref() == Some(id.as_str()) {
                            self.handoff().withdraw_voicemail();
                            self.calls.view.audio_ready = None;
                        }
                        let client = self.voicemail_client().unwrap();
                        return Task::perform(
                            async move { client.delete(&id).await.map_err(|e| e.to_string()) },
                            move |r| Msg::Calls(CallsMsg::Mutated(g, r)),
                        );
                    }
                }
            }
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl App {
        pub(crate) fn calls_logs_for_test(&mut self, logs: Vec<CallLog>) {
            self.calls.logs = logs;
            self.refresh_history_view();
        }

        fn take_voicemail_audio(&self, id: &str) -> Option<(String, Vec<u8>)> {
            self.handoff().take_voicemail(id)
        }
    }

    fn log(id: &str, extra: serde_json::Value) -> CallLog {
        let mut raw = json!({"callId": id, "callDirection": "incoming", "callState": "missed",
            "startTime": "2026-10-13T09:00:00Z",
            "originatorParticipant": {"id": "8:orgid:bob"}});
        raw.as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap_or_default());
        let message: Message = serde_json::from_value(
            json!({"id": format!("m-{id}"), "properties": {"call-log": raw.to_string()}}),
        )
        .unwrap();
        CallLog::from_message(log_message(&message)).unwrap()
    }

    fn signed_in() -> App {
        let mut app = App::empty();
        *app.client = Some(ChatClient::new(Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new(),
        ))));
        *app.me = "8:orgid:me".into();
        *app.nav = Nav::Calls;
        app
    }

    #[tokio::test]
    async fn demo_server_serves_history_voicemail_and_deletions() {
        let base = Url::parse(&format!("http://{}/", mock::spawn())).unwrap();
        let auth = Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new()
                .with_service(Service::Csa, base.clone())
                .with_service(Service::ChatService, base.clone())
                .with_service(Service::Outlook, base),
        ));
        let client = ChatClient::new(auth.clone());
        let page = fetch_history(client.clone(), None).await.unwrap();
        assert_eq!(page.logs.len(), 5);
        assert_eq!(page.logs[0].call_id, "demo-call-1");
        let body = json!({"call-log": page.logs[0].deleted_property()});
        client
            .set_message_property(CALL_LOGS, "demo-call-1", "call-log", &body, true)
            .await
            .unwrap();
        let page = fetch_history(client, None).await.unwrap();
        assert!(page.logs.iter().all(|l| l.call_id != "demo-call-1"));

        let voicemail = VoicemailClient::new(auth);
        let list = voicemail.list(30).await.unwrap();
        assert_eq!(list[0].id, "demo-voicemail-1");
        let (kind, audio) = voicemail.audio("demo-voicemail-1").await.unwrap();
        assert_eq!(kind, "audio/wav");
        assert!(audio.starts_with(b"RIFF"));
    }

    #[tokio::test]
    async fn the_push_channel_runs_once_signed_in() {
        let fake = teams_realtime::fake::FakeTrouter::start().await;
        let mut app = signed_in();
        *app.client = Some(ChatClient::new(Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new()
                .with_service(Service::Trouter, fake.trouter_url())
                .with_service(Service::Registrar, fake.registrar_url()),
        ))));
        let task = app.sync_realtime();
        let Some(crate::task::Job::Subscribe(start)) = task.0.into_iter().next() else {
            panic!("no push channel started");
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(start(crate::task::Sink::new(move |msg| {
            let _ = tx.send(msg);
        })));
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.calls.view.realtime != "Prêt à recevoir les appels" {
                let Some(Msg::Calls(msg)) = rx.recv().await else {
                    panic!("channel ended")
                };
                let _ = app.update_calls(msg);
            }
        })
        .await
        .expect("push channel never became ready");
        assert!(app.sync_realtime().0.is_empty(), "started only once");
    }

    #[test]
    fn stale_history_responses_are_ignored_and_names_come_from_the_directory() {
        let mut app = signed_in();
        Arc::make_mut(&mut app.profiles).insert(
            "8:orgid:bob".into(),
            serde_json::from_value(json!({"mri": "8:orgid:bob", "displayName": "Bruno"})).unwrap(),
        );
        let _ = app.load_calls(true);
        let current = app.calls.history_request;
        let page = |id: &str| HistoryPage {
            logs: vec![log(id, json!({}))],
            oldest: None,
            full: false,
        };
        let _ = app.update_calls(CallsMsg::History(
            app.generation,
            current - 1,
            false,
            Ok(page("old")),
        ));
        assert!(app.calls.view.history.is_empty());
        let _ = app.update_calls(CallsMsg::History(
            app.generation,
            current,
            false,
            Ok(page("new")),
        ));
        assert_eq!(app.calls.view.history[0].id, "new");
        assert_eq!(app.calls.view.history[0].name, "Bruno");
        assert_eq!(app.calls.view.history[0].peer, "8:orgid:bob");
        assert_eq!(app.calls.view.history[0].outcome, CallOutcome::Missed);
        assert!(!app.calls.view.loading_history);
    }

    #[test]
    fn deleting_a_log_hides_it_at_once_and_voicemail_audio_is_handed_over_once() {
        let mut app = signed_in();
        app.calls.logs = vec![log("a", json!({})), log("b", json!({}))];
        let task = app.update_calls(CallsMsg::Action(CallsAction::DeleteLog { id: "a".into() }));
        assert_eq!(task.0.len(), 1);
        assert_eq!(app.calls.view.history.len(), 1);

        app.calls.view.voicemails = vec![VoicemailView {
            id: "v".into(),
            sender: "Alice".into(),
            address: String::new(),
            received: String::new(),
            read: false,
            duration_secs: None,
            transcription: None,
        }];
        let _ = app.update_calls(CallsMsg::Action(CallsAction::PlayVoicemail {
            id: "v".into(),
        }));
        let task = app.update_calls(CallsMsg::Audio(
            app.generation,
            "v".into(),
            Ok(("audio/mpeg".into(), vec![1, 2])),
        ));
        assert_eq!(task.0.len(), 1, "listening marks the voicemail read");
        assert!(app.calls.view.voicemails[0].read);
        assert_eq!(app.calls.view.audio_ready.as_deref(), Some("v"));
        assert_eq!(app.take_voicemail_audio("v").unwrap().1, vec![1, 2]);
        assert!(app.take_voicemail_audio("v").is_none());
        assert_eq!(
            app.calls.view.audio_ready.as_deref(),
            Some("v"),
            "nothing left to take"
        );
    }
}
