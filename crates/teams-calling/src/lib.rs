//! Teams calling: call history, voicemail, meeting links and NGC signalling; lenient parsing, unknown fields kept.

pub mod callbacks;
pub mod client;
pub mod diagnostics;
pub mod driver;
pub mod history;
mod http;
pub mod meeting_link;
pub mod payload;
pub mod sdp;
pub mod session;
pub mod trap;
pub mod voicemail;

pub use history::{CallDirection, CallLog, CallParticipant, CallState};
pub use meeting_link::{MeetingCoordinates, MeetingLink};
pub use voicemail::{Voicemail, VoicemailClient};

use teams_auth::AuthError;
use teams_i18n::{t, tf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("{}", tf!("calling.lib.couldn_t_reach_the_calling_service", detail = .0))]
    Transport(String),
    #[error("{}", tf!("calling.lib.the_calling_service_responded_with_http", status = .0))]
    Http(u16),
    #[error("{}", tf!("calling.lib.too_many_requests_http_429_you", detail = .0))]
    RateLimited(u64),
    #[error(
        "{}",
        t("calling.lib.unreadable_response_from_the_calling_service")
    )]
    Decode,
    #[error("{0}")]
    Invalid(String),
    #[error(
        "{}",
        t("calling.lib.the_service_didn_t_confirm_the")
    )]
    Uncertain,
}

#[cfg(feature = "fake")]
pub mod fake;
