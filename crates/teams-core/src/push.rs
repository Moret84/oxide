//! One Trouter push channel (calls or chat notifications): start, stop, liveness and status texts.
use super::*;
use teams_i18n::{fill, t};
use teams_realtime::{PushedRequest, RealtimeConfig, RealtimeEvent};

/// Translation keys of a channel's states.
pub(crate) struct PushTexts {
    pub connecting: &'static str,
    pub registering: &'static str,
    pub ready: &'static str,
    /// `{reason}`.
    pub refused: &'static str,
    /// `{reason}`, `{seconds}`.
    pub retrying: &'static str,
    /// `{reason}`.
    pub stopped: &'static str,
}

impl PushTexts {
    /// `status` in the current language when it is a state without details.
    pub(crate) fn relocalized(&self, status: &str) -> Option<&'static str> {
        relocalized(status, &[self.connecting, self.registering, self.ready])
    }
}

/// A Trouter event: a change of the channel's state, or something for its owner.
pub(crate) enum Incoming {
    Change(Change),
    Request(PushedRequest),
    MessageLoss,
    Diagnostic(String),
}
/// An event that changes the channel's state, for [`PushChannel::apply`].
pub(crate) struct Change(RealtimeEvent);

impl From<RealtimeEvent> for Incoming {
    fn from(event: RealtimeEvent) -> Self {
        match event {
            RealtimeEvent::Request(request) => Self::Request(request),
            RealtimeEvent::MessageLoss { .. } => Self::MessageLoss,
            RealtimeEvent::ProtocolDiagnostic(message) => Self::Diagnostic(message),
            change => Self::Change(Change(change)),
        }
    }
}

/// The channel's new state, its status text in the current language.
pub(crate) enum Transition {
    Connected {
        surl: String,
        status: String,
    },
    /// `resumed` when the channel was not live before: pushes sent meanwhile were lost.
    Registered {
        status: String,
        resumed: bool,
    },
    /// Registration refused or connection lost; the channel retries.
    Down(String),
    /// The channel gave up and is stopped.
    Stopped(String),
}

#[derive(Default)]
pub(crate) struct PushChannel {
    /// Dropping it stops the channel.
    pub(crate) stop: Option<tokio::sync::watch::Sender<bool>>,
    /// Endpoint id registered for this app session.
    pub(crate) endpoint: String,
    /// Connected and registered: pushes arrive.
    live: bool,
}

impl PushChannel {
    pub(crate) fn running(&self) -> bool {
        self.stop.is_some()
    }
    pub(crate) fn live(&self) -> bool {
        self.live
    }
    /// Stops the channel; whether it was running.
    pub(crate) fn halt(&mut self) -> bool {
        self.live = false;
        self.stop.take().is_some()
    }

    /// Connects with `config` built for this endpoint; every event goes back through `wrap`.
    pub(crate) fn start(
        &mut self,
        auth: Arc<AppAuthenticator>,
        config: impl FnOnce(String) -> RealtimeConfig,
        wrap: impl Fn(RealtimeEvent) -> Msg + Send + Sync + 'static,
    ) -> Task<Msg> {
        if self.endpoint.is_empty() {
            self.endpoint = uuid::Uuid::new_v4().to_string();
        }
        let config = config(self.endpoint.clone());
        let (stop, stopped) = tokio::sync::watch::channel(false);
        self.stop = Some(stop);
        self.live = false;
        Task::subscribe(move |sink| async move {
            let sink: teams_realtime::Sink = Arc::new(move |event| sink.send(wrap(event)));
            teams_realtime::run(auth, config, sink, stopped).await;
        })
    }

    pub(crate) fn apply(&mut self, texts: &PushTexts, Change(event): Change) -> Transition {
        match event {
            RealtimeEvent::Connected { surl } => {
                self.live = false;
                Transition::Connected {
                    surl,
                    status: t(texts.registering).into(),
                }
            }
            RealtimeEvent::Registered { .. } => {
                let resumed = !self.live;
                self.live = true;
                Transition::Registered {
                    status: t(texts.ready).into(),
                    resumed,
                }
            }
            RealtimeEvent::RegistrationFailed { reason } => {
                self.live = false;
                Transition::Down(fill(t(texts.refused), &[("reason", reason)]))
            }
            RealtimeEvent::Disconnected { reason, retry_in } => {
                self.live = false;
                let seconds = retry_in.as_secs_f32().ceil() as u64;
                Transition::Down(fill(
                    t(texts.retrying),
                    &[("reason", reason), ("seconds", seconds.to_string())],
                ))
            }
            RealtimeEvent::Fatal(reason) => {
                self.halt();
                Transition::Stopped(fill(t(texts.stopped), &[("reason", reason)]))
            }
            RealtimeEvent::Request(_)
            | RealtimeEvent::MessageLoss { .. }
            | RealtimeEvent::ProtocolDiagnostic(_) => unreachable!("not a change: {event:?}"),
        }
    }
}
