//! Who is typing in a chat, and whether the others have read my messages (consumption horizons).
use super::*;
use std::time::Instant;
use teams_chat::receipts::{self as wire, MemberHorizon};
use teams_i18n::tf;

/// `typingIndicator.notificationFrequency` (config-prod).
const TYPING_SEND_EVERY: Duration = Duration::from_secs(20);
/// `typingIndicator.userTimeout` (config-prod).
const TYPING_TIMEOUT: Duration = Duration::from_secs(22);
/// `maxRosterSizeInGroupChatChatServiceLimit` (config-prod).
const RECEIPTS_MAX_MEMBERS: usize = 20;
/// Horizons of the open chat are read again at most this often without a push.
const HORIZONS_REFRESH: Duration = Duration::from_secs(30);

/// Status icon of one of my messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ReceiptView {
    /// Delivered, not yet read by everyone.
    Sent,
    /// Read by the other member (1:1).
    Seen,
    /// Read by every other member of the group.
    SeenByEveryone,
}

#[derive(Debug, Clone)]
struct Typer {
    mri: String,
    name: String,
    until: Instant,
}

#[derive(Default)]
pub(crate) struct ReceiptsState {
    typing: Tracked<HashMap<String, Vec<Typer>>, { feeds::TYPING }>,
    /// Last typing sent per conversation.
    sent: HashMap<String, Instant>,
    horizons: Tracked<HashMap<String, Vec<MemberHorizon>>, { feeds::MESSAGES }>,
    /// When each conversation's horizons were last asked for.
    asked: HashMap<String, Instant>,
    /// `readReceiptsEnabled` user property; receipts are shown unless it is `false`.
    disabled: Tracked<bool, { feeds::MESSAGES }>,
}
impl ReceiptsState {
    /// The parts changed since the previous call.
    pub(crate) fn take_changes(&mut self) -> Parts {
        let Self {
            typing,
            sent: _,
            horizons,
            asked: _,
            disabled,
        } = self;
        typing.take_changes() | horizons.take_changes() | disabled.take_changes()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ReceiptsMsg {
    /// A lost typing notification is not worth an error banner.
    TypingSent,
    HorizonsLoaded(u64, String, Result<Vec<MemberHorizon>, String>),
    /// A typer may have timed out in this conversation.
    Expire(String),
    /// `readReceiptsEnabled` written; the previous value comes back on failure.
    Saved(u64, bool, Result<(), String>),
}

impl App {
    pub(crate) fn update_receipts(&mut self, msg: ReceiptsMsg) -> Task<Msg> {
        match msg {
            ReceiptsMsg::TypingSent => Task::none(),
            ReceiptsMsg::HorizonsLoaded(g, _, _) | ReceiptsMsg::Saved(g, _, _)
                if g != self.generation =>
            {
                Task::none()
            }
            ReceiptsMsg::Saved(_, previous, result) => {
                if let Err(error) = result {
                    *self.receipts.disabled = previous;
                    *self.error = Some(tf!(
                        "core.receipts.read_receipts_not_changed",
                        error = error
                    ));
                }
                Task::none()
            }
            ReceiptsMsg::HorizonsLoaded(_, conv, result) => {
                if let Ok(horizons) = result {
                    self.receipts.horizons.insert(conv, horizons);
                }
                Task::none()
            }
            ReceiptsMsg::Expire(conv) => {
                let now = Instant::now();
                self.retain_typers(&conv, |t| t.until > now);
                Task::none()
            }
        }
    }

    /// `readReceiptsEnabled` from `GET v1/users/ME/properties` (a string or a boolean).
    pub(crate) fn receipts_preferences(&mut self, properties: &serde_json::Value) {
        *self.receipts.disabled = match properties.get("readReceiptsEnabled") {
            Some(serde_json::Value::Bool(on)) => !on,
            Some(serde_json::Value::String(s)) => s.eq_ignore_ascii_case("false"),
            _ => false,
        };
    }

    pub(crate) fn read_receipts_on(&self) -> bool {
        !*self.receipts.disabled
    }

    /// `PUT v1/users/ME/properties?name=readReceiptsEnabled` as a string; off, others' receipts are neither read nor shown.
    pub(crate) fn set_read_receipts(&mut self, on: bool) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let previous = *self.receipts.disabled;
        *self.receipts.disabled = !on;
        if on {
            // Read the open chat's horizons again at once.
            self.receipts.asked.clear();
        }
        let generation = self.generation;
        Task::perform(
            async move {
                client
                    .set_user_property("readReceiptsEnabled", serde_json::json!(on.to_string()))
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| Msg::Receipts(ReceiptsMsg::Saved(generation, previous, r)),
        )
    }

    /// Typing and read positions from the chat push channel; `None` for any other message.
    pub(crate) fn control_message(&mut self, conv: &str, message: &Message) -> Option<Task<Msg>> {
        let sender = message.sender_mri().to_owned();
        match message.messagetype.as_str() {
            wire::TYPING => {
                if sender.eq_ignore_ascii_case(&self.me) || notifications::is_channel_id(conv) {
                    return Some(Task::none());
                }
                let name = message
                    .imdisplayname
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .or_else(|| self.known_name(&sender))
                    // Unknown: named at display time, in the current language.
                    .unwrap_or_default();
                let typers = self.receipts.typing.entry(conv.to_owned()).or_default();
                typers.retain(|t| t.mri != sender);
                typers.push(Typer {
                    mri: sender,
                    name,
                    until: Instant::now() + TYPING_TIMEOUT,
                });
                let conv = conv.to_owned();
                Some(Task::perform(
                    async move {
                        tokio::time::sleep(TYPING_TIMEOUT + Duration::from_millis(50)).await;
                        conv
                    },
                    |conv| Msg::Receipts(ReceiptsMsg::Expire(conv)),
                ))
            }
            wire::CLEAR_TYPING => {
                self.stop_typing(conv, &sender);
                Some(Task::none())
            }
            wire::HORIZON_UPDATE => {
                // Any other message from a typer ends their typing.
                self.stop_typing(conv, &sender);
                if let Some(update) = wire::parse_horizon_update(&message.content) {
                    let list = self.receipts.horizons.entry(conv.to_owned()).or_default();
                    list.retain(|h| h.id != update.id);
                    list.push(update);
                }
                Some(Task::none())
            }
            _ => None,
        }
    }

    /// A real message from a typer ends their typing.
    pub(crate) fn stop_typing(&mut self, conv: &str, sender: &str) {
        self.retain_typers(conv, |t| !t.mri.eq_ignore_ascii_case(sender));
    }

    fn retain_typers(&mut self, conv: &str, keep: impl FnMut(&Typer) -> bool) {
        if let Some(typers) = self.receipts.typing.get_mut(conv) {
            typers.retain(keep);
            if typers.is_empty() {
                self.receipts.typing.remove(conv);
            }
        }
    }

    /// The draft changed: tells the chat, at most every 20 s, that the user is typing.
    pub(crate) fn typed(&mut self) -> Task<Msg> {
        let (Some(client), Some(Selection::Chat(conv))) = (self.client.clone(), &*self.selected)
        else {
            return Task::none();
        };
        // Not while editing a message, nor alone in the chat or in a large group.
        if self.conv.draft().trim().is_empty()
            || self.conv.editing().is_some()
            || !self.typing_supported(conv)
        {
            return Task::none();
        }
        let now = Instant::now();
        if self
            .receipts
            .sent
            .get(conv)
            .is_some_and(|at| now.duration_since(*at) < TYPING_SEND_EVERY)
        {
            return Task::none();
        }
        self.receipts.sent.insert(conv.clone(), now);
        let conv = conv.clone();
        Task::perform(async move { client.send_typing(&conv).await.ok() }, |_| {
            Msg::Receipts(ReceiptsMsg::TypingSent)
        })
    }

    /// After sending, the next keystroke says "typing" again at once.
    pub(crate) fn typing_sent_message(&mut self, conv: &str) {
        self.receipts.sent.remove(conv);
    }

    /// Reads the open chat's horizons when they are unknown or stale. Idempotent.
    pub(crate) fn load_receipts(&mut self) -> Task<Msg> {
        let (Some(client), Some(Selection::Chat(conv))) = (self.client.clone(), &*self.selected)
        else {
            return Task::none();
        };
        if *self.receipts.disabled || !self.receipts_supported(conv) {
            return Task::none();
        }
        let now = Instant::now();
        if self
            .receipts
            .asked
            .get(conv)
            .is_some_and(|at| now.duration_since(*at) < HORIZONS_REFRESH)
        {
            return Task::none();
        }
        self.receipts.asked.insert(conv.clone(), now);
        let (conv, generation) = (conv.clone(), self.generation);
        Task::perform(
            async move {
                let result = client
                    .member_horizons(&conv)
                    .await
                    .map_err(|e| e.to_string());
                (conv, result)
            },
            move |(conv, r)| Msg::Receipts(ReceiptsMsg::HorizonsLoaded(generation, conv, r)),
        )
    }

    fn typing_supported(&self, conv: &str) -> bool {
        self.receipts_supported(conv)
            && self
                .chats
                .iter()
                .find(|c| c.id == conv)
                .is_some_and(|c| c.members.len() != 1)
    }

    fn receipts_supported(&self, conv: &str) -> bool {
        if conv == NOTES || conv.starts_with("48:") || notifications::is_channel_id(conv) {
            return false;
        }
        self.chats
            .iter()
            .find(|c| c.id == conv)
            .is_some_and(|c| !c.is_group() || c.members.len() <= RECEIPTS_MAX_MEMBERS)
    }

    /// "Alice est en train d’écrire…" for the open chat.
    pub(crate) fn typing_text(&self) -> Option<String> {
        let Some(Selection::Chat(conv)) = &*self.selected else {
            return None;
        };
        let now = Instant::now();
        let names: Vec<&str> = self
            .receipts
            .typing
            .get(conv)?
            .iter()
            .filter(|t| t.until > now)
            .map(|t| match t.name.as_str() {
                "" => teams_i18n::t("core.receipts.someone"),
                name => name,
            })
            .collect();
        typing_sentence(&names)
    }

    /// Icons of my messages (oldest first): only the latest one everyone read is seen, and my latest newer one is sent.
    pub(crate) fn receipts_for(&self, messages: &[Message]) -> HashMap<String, ReceiptView> {
        let mut icons = HashMap::new();
        let Some(Selection::Chat(conv)) = &*self.selected else {
            return icons;
        };
        if *self.receipts.disabled || !self.receipts_supported(conv) {
            return icons;
        }
        let Some(horizons) = self.receipts.horizons.get(conv) else {
            return icons;
        };
        let chat = self.chats.iter().find(|c| &c.id == conv);
        let one_on_one = chat.is_none_or(|c| !c.is_group());
        let others: Vec<&str> = chat
            .map(|c| {
                c.members
                    .iter()
                    .map(|m| m.id.as_str())
                    .filter(|id| !id.eq_ignore_ascii_case(&self.me))
                    .collect()
            })
            .filter(|ids: &Vec<&str>| !ids.is_empty())
            .unwrap_or_else(|| {
                horizons
                    .iter()
                    .map(|h| h.id.as_str())
                    .filter(|id| !id.eq_ignore_ascii_case(&self.me))
                    .collect()
            });
        if others.is_empty() {
            return icons;
        }
        // Every other member needs a horizon; messages before a late joiner's visibility were never seen by them.
        let theirs: Vec<&MemberHorizon> = others
            .iter()
            .filter_map(|member| horizons.iter().find(|h| h.id.eq_ignore_ascii_case(member)))
            .collect();
        if theirs.len() != others.len() {
            return icons;
        }
        let earliest = theirs
            .iter()
            .map(|h| h.read_up_to)
            .min()
            .unwrap_or_default();
        let visible = theirs
            .iter()
            .map(|h| h.visible_from)
            .max()
            .unwrap_or_default();
        let seen = |time: i64| time <= earliest && time >= visible;
        let mine: Vec<(&Message, i64)> = messages
            .iter()
            .filter(|m| m.sender_mri().eq_ignore_ascii_case(&self.me) && !m.is_deleted())
            .filter_map(|m| conversations::message_time(m).map(|t| (m, t)))
            .collect();
        let last_seen = mine.iter().rev().find(|(_, t)| seen(*t));
        if let Some((m, _)) = last_seen {
            icons.insert(
                m.id.clone(),
                if one_on_one {
                    ReceiptView::Seen
                } else {
                    ReceiptView::SeenByEveryone
                },
            );
        }
        if let Some((m, t)) = mine.last()
            && last_seen.is_none_or(|(_, seen_at)| t > seen_at)
        {
            icons.insert(m.id.clone(), ReceiptView::Sent);
        }
        icons
    }
}

/// Up to three names, then the first two and a count (`TypingUsersPublisher` labels).
pub(crate) fn typing_sentence(names: &[&str]) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(tf!("core.receipts.is_typing", name = one)),
        [a, b] => Some(tf!("core.receipts.and_are_typing", first = a, second = b)),
        [a, b, c] => Some(tf!(
            "core.receipts.and_are_typing_2",
            first = a,
            second = b,
            third = c
        )),
        [a, b, rest @ ..] => Some(tf!(
            "core.receipts.and_others_are_typing",
            first = a,
            second = b,
            rest = rest.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, from: &str, time: &str, kind: &str, content: &str) -> Message {
        serde_json::from_value(json!({
            "id": id, "messagetype": kind, "content": content,
            "from": format!("https://x/contacts/{from}"), "imdisplayname": "Alice",
            "originalarrivaltime": time,
            "conversationLink": "https://x/conversations/19:chat@thread.v2",
        }))
        .unwrap()
    }
    fn chat(members: &[&str]) -> Conversation {
        serde_json::from_value(json!({
            "id": "19:chat@thread.v2",
            "members": members.iter().map(|m| json!({"id": m})).collect::<Vec<_>>(),
        }))
        .unwrap()
    }
    fn app(members: &[&str]) -> App {
        let mut app = App::empty();
        *app.me = "8:orgid:me".into();
        *app.chats = vec![chat(members)];
        *app.selected = Some(Selection::Chat("19:chat@thread.v2".into()));
        app
    }

    #[test]
    fn typing_shows_names_and_ends_with_a_message_or_clear() {
        let mut app = app(&["8:orgid:me", "8:orgid:a"]);
        let typing = message("1", "8:orgid:a", "2026-10-16T10:00:00Z", wire::TYPING, "");
        let task = app.control_message("19:chat@thread.v2", &typing).unwrap();
        assert_eq!(task.0.len(), 1, "an expiry timer");
        assert_eq!(
            app.typing_text().as_deref(),
            Some("Alice est en train d’écrire…")
        );
        let mine = message("2", "8:orgid:me", "2026-10-16T10:00:00Z", wire::TYPING, "");
        let _ = app.control_message("19:chat@thread.v2", &mine);
        assert_eq!(
            app.typing_text().as_deref(),
            Some("Alice est en train d’écrire…")
        );
        let clear = message(
            "3",
            "8:orgid:a",
            "2026-10-16T10:00:00Z",
            wire::CLEAR_TYPING,
            "",
        );
        let _ = app.control_message("19:chat@thread.v2", &clear);
        assert_eq!(app.typing_text(), None);
        let _ = app.control_message("19:chat@thread.v2", &typing);
        app.stop_typing("19:chat@thread.v2", "8:orgid:a");
        assert_eq!(app.typing_text(), None);
        let text = message(
            "4",
            "8:orgid:a",
            "2026-10-16T10:00:00Z",
            "RichText/Html",
            "x",
        );
        assert!(app.control_message("19:chat@thread.v2", &text).is_none());
        assert_eq!(
            typing_sentence(&["A", "B"]).unwrap(),
            "A et B sont en train d’écrire…"
        );
        assert_eq!(
            typing_sentence(&["A", "B", "C"]).unwrap(),
            "A, B et C sont en train d’écrire…"
        );
        assert_eq!(
            typing_sentence(&["A", "B", "C", "D", "E"]).unwrap(),
            "A, B et 3 autres sont en train d’écrire…"
        );
    }

    #[test]
    fn only_the_latest_seen_and_the_latest_sent_messages_carry_an_icon() {
        let mut app = app(&["8:orgid:me", "8:orgid:a", "8:orgid:b"]);
        let messages = vec![
            message(
                "1",
                "8:orgid:me",
                "2026-10-16T10:00:00Z",
                "RichText/Html",
                "a",
            ),
            message(
                "2",
                "8:orgid:me",
                "2026-10-16T10:01:00Z",
                "RichText/Html",
                "b",
            ),
            message(
                "3",
                "8:orgid:a",
                "2026-10-16T10:02:00Z",
                "RichText/Html",
                "c",
            ),
            message(
                "4",
                "8:orgid:me",
                "2026-10-16T10:03:00Z",
                "RichText/Html",
                "d",
            ),
        ];
        let t = |m: &Message| conversations::message_time(m).unwrap();
        assert!(app.receipts_for(&messages).is_empty(), "no horizons yet");
        let update = |who: &str, at: i64| {
            message(
                "h",
                who,
                "2026-10-16T10:05:00Z",
                wire::HORIZON_UPDATE,
                &json!({"user": who, "consumptionhorizon": format!("{at};0;0")}).to_string(),
            )
        };
        let _ = app.control_message("19:chat@thread.v2", &update("8:orgid:a", t(&messages[3])));
        assert!(
            app.receipts_for(&messages).is_empty(),
            "b has no horizon yet: no status at all"
        );
        let _ = app.control_message("19:chat@thread.v2", &update("8:orgid:b", t(&messages[1])));
        let icons = app.receipts_for(&messages);
        assert_eq!(icons["2"], ReceiptView::SeenByEveryone);
        assert_eq!(icons["4"], ReceiptView::Sent);
        assert!(!icons.contains_key("1"));
        let _ = app.control_message("19:chat@thread.v2", &update("8:orgid:b", t(&messages[3])));
        let icons = app.receipts_for(&messages);
        assert_eq!(icons.len(), 1);
        assert_eq!(icons["4"], ReceiptView::SeenByEveryone);

        app.receipts_preferences(&json!({"readReceiptsEnabled": "false"}));
        assert!(
            app.receipts_for(&messages).is_empty(),
            "turned off by the user"
        );
        assert!(!app.read_receipts_on());
        assert!(app.load_receipts().0.is_empty(), "nor read while off");
    }

    #[test]
    fn one_on_one_chats_say_seen_and_large_groups_have_no_receipts() {
        let mut app = app(&["8:orgid:me", "8:orgid:a"]);
        app.receipts.horizons.insert(
            "19:chat@thread.v2".into(),
            vec![MemberHorizon {
                id: "8:orgid:a".into(),
                read_up_to: i64::MAX,
                visible_from: 0,
            }],
        );
        let messages = vec![message(
            "1",
            "8:orgid:me",
            "2026-10-16T10:00:00Z",
            "RichText/Html",
            "a",
        )];
        assert_eq!(app.receipts_for(&messages)["1"], ReceiptView::Seen);
        let members: Vec<String> = (0..21).map(|i| format!("8:orgid:{i}")).collect();
        let refs: Vec<&str> = members.iter().map(String::as_str).collect();
        *app.chats = vec![chat(&refs)];
        assert!(app.receipts_for(&messages).is_empty());
    }
}
