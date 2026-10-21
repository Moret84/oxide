use http::StatusCode;
use teams_i18n::tf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}", tf!("chat.error.too_many_requests_to_teams_http", retry_after_secs = .retry_after_secs))]
    RateLimited { retry_after_secs: u64 },
    #[error("{0}")]
    Media(&'static str),
    #[error(transparent)]
    Auth(#[from] teams_auth::AuthError),
    /// Never carries the request URL, which may hold secrets (long-poll cursors).
    #[error("{}", tf!("chat.error.request_failed", detail = .0))]
    Http(reqwest::Error),
    #[error("{}", tf!("chat.error.invalid_url", detail = .0))]
    Url(#[from] url::ParseError),
    #[error("{}", tf!("chat.error.invalid_long_poll_url", detail = .0))]
    InvalidLongPollUrl(String),
    #[error("{}", tf!("chat.error.from", status = .status, endpoint = .endpoint, body = .body))]
    Status {
        endpoint: &'static str,
        status: StatusCode,
        body: String,
    },
    #[error("{}", tf!("chat.error.cannot_decode_field", path = .path, endpoint = .endpoint, source = .source))]
    DecodeField {
        endpoint: &'static str,
        path: String,
        source: serde_json::Error,
    },
    #[error("{}", tf!("chat.error.cannot_decode_response", endpoint = .endpoint, source = .source))]
    Decode {
        endpoint: &'static str,
        source: serde_json::Error,
    },
}

impl From<teams_auth::net::Failure> for Error {
    fn from(failure: teams_auth::net::Failure) -> Self {
        match failure {
            teams_auth::net::Failure::Auth(e) => Self::Auth(e),
            teams_auth::net::Failure::RateLimited(retry_after_secs) => {
                Self::RateLimited { retry_after_secs }
            }
            teams_auth::net::Failure::Transport(e) => e.into(),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.without_url())
    }
}
