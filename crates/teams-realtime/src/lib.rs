//! Trouter push channel (v4a allocate, socket.io 0.9, websocket) and registrar: [`run`] hands pushed requests to the caller and answers them.

pub mod sio;
mod transport;
mod trouter;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use trouter::run;

/// Static description of this endpoint, sent at allocation and registration.
#[derive(Debug, Clone)]
pub struct RealtimeConfig {
    /// Push application: `SkypeSpacesWeb` for calls.
    pub app_id: String,
    /// Notification template: `SkypeSpacesWeb_2.6` for calls.
    pub template_key: String,
    /// Endpoint id (`epid`, registration id): one UUID per app install or session.
    pub endpoint_id: String,
    /// `cor_id`: one UUID per app session.
    pub session_id: String,
    /// `<platformId>/<build>`, e.g. [`teams_auth::CLIENT_VERSION`].
    pub client_version: String,
    /// `ua` of the `tc` client info.
    pub user_agent: String,
    /// `clientDescription.platform` of the registration.
    pub platform: String,
    /// Path prefixes this client handles; other pushed requests get a 404.
    pub paths: Vec<String>,
    pub timing: Timing,
    /// Set while a call is active: pings every 5 s so a dead socket is noticed quickly.
    pub in_call: Arc<CallFlag>,
    /// Report every pushed packet's shape as [`RealtimeEvent::ProtocolDiagnostic`].
    pub diagnostics: bool,
}

/// Whether a call is active, waking the push loop only when it flips (no periodic re-check).
#[derive(Debug, Default)]
pub struct CallFlag {
    on: AtomicBool,
    changed: tokio::sync::Notify,
}

impl CallFlag {
    pub fn get(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
    pub fn set(&self, on: bool) {
        if self.on.swap(on, Ordering::Relaxed) != on {
            // A stored permit covers a flip between the loop's read and its wait.
            self.changed.notify_one();
        }
    }
    pub(crate) async fn changed(&self) {
        self.changed.notified().await
    }
}

impl RealtimeConfig {
    /// Settings of the web calling stack (`buildTrouterConfigWith`).
    pub fn calling(endpoint_id: String, session_id: String) -> Self {
        Self {
            app_id: "SkypeSpacesWeb".into(),
            template_key: "SkypeSpacesWeb_2.6".into(),
            endpoint_id,
            session_id,
            client_version: teams_auth::CLIENT_VERSION.into(),
            user_agent: "TeamsCDL".into(),
            platform: "Chrome".into(),
            // Callbacks arrive under `/callAgent/`; the incoming-call path is unverified, so accept everything.
            paths: vec!["/".into()],
            timing: Timing::default(),
            in_call: Arc::default(),
            diagnostics: true,
        }
    }

    /// Settings of the data (CDL) stack: chat messages and user properties arrive as `EventMessage`s on `/messaging`.
    pub fn messaging(endpoint_id: String, session_id: String) -> Self {
        Self {
            app_id: "TeamsCDLWebWorker".into(),
            template_key: "TeamsCDLWebWorker_2.6".into(),
            endpoint_id,
            session_id,
            client_version: teams_auth::CLIENT_VERSION.into(),
            user_agent: "TeamsCDL".into(),
            platform: "Chrome".into(),
            paths: vec!["/messaging".into()],
            timing: Timing {
                // `maxBackoffMs` when `productName == "TeamsCDL"`.
                max_backoff: Duration::from_secs(300),
                ..Timing::default()
            },
            in_call: Arc::default(),
            diagnostics: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Timing {
    pub connect_timeout: Duration,
    pub fetch_timeout: Duration,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
    pub max_backoff: Duration,
    /// Requested registration lifetime; renewed 30 s before it ends.
    pub registration_ttl: u64,
    /// Delay before retrying a failed registration.
    pub registration_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(20),
            fetch_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(40),
            pong_timeout: Duration::from_secs(5),
            max_backoff: Duration::from_secs(30),
            registration_ttl: 3600,
            registration_retry: Duration::from_secs(123),
        }
    }
}

impl Timing {
    /// Tighter settings while a call is active.
    pub fn in_call() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            fetch_timeout: Duration::from_secs(5),
            ping_interval: Duration::from_secs(5),
            pong_timeout: Duration::from_secs(2),
            max_backoff: Duration::from_secs(10),
            ..Self::default()
        }
    }
}

/// An HTTP request a service pushed to this endpoint through Trouter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedRequest {
    pub method: String,
    /// Request target as sent: absolute URL or relative path under the session URL.
    pub url: String,
    /// Path relative to the session URL, starting with `/`.
    pub path: String,
    /// Header names in lower case.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealtimeEvent {
    /// Socket open; `surl` is the public base URL of this session, without a final `/`.
    Connected {
        surl: String,
    },
    /// The registrar accepted the session for `ttl` seconds.
    Registered {
        ttl: u64,
    },
    RegistrationFailed {
        reason: String,
    },
    Request(PushedRequest),
    /// Protocol shape only, for local diagnostics; contains no pushed body or URL.
    ProtocolDiagnostic(String),
    /// Trouter dropped notifications for these tags: resynchronise them.
    MessageLoss {
        tags: Vec<String>,
    },
    Disconnected {
        reason: String,
        retry_in: Duration,
    },
    /// Unrecoverable (deprecated token, repeated authorization failures): the loop stopped.
    Fatal(String),
}

pub type Sink = Arc<dyn Fn(RealtimeEvent) + Send + Sync>;

#[cfg(feature = "fake")]
pub mod fake;
