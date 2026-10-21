//! Errors the app must tell apart, not only show; each keeps today's text so the banners stay the same.
use teams_auth::AuthError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreError {
    /// Signing in again is the only way forward.
    SessionExpired(String),
    /// Microsoft could not be reached: no network, or a timeout.
    Offline(String),
    RateLimited {
        retry_after_secs: u64,
        message: String,
    },
    Other(String),
}

impl CoreError {
    /// The kind of `error`, shown as `message`.
    pub fn chat(error: &teams_chat::Error, message: String) -> Self {
        use teams_chat::Error;
        match error {
            Error::RateLimited { retry_after_secs } => Self::RateLimited {
                retry_after_secs: *retry_after_secs,
                message,
            },
            Error::Auth(auth) if expired(auth) => Self::SessionExpired(message),
            Error::Http(e) if e.is_connect() || e.is_timeout() => Self::Offline(message),
            _ => Self::Other(message),
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Self::SessionExpired(m) | Self::Offline(m) | Self::Other(m) => m,
            Self::RateLimited { message, .. } => message,
        }
    }
}

fn expired(error: &AuthError) -> bool {
    matches!(error, AuthError::SessionExpired | AuthError::NotSignedIn)
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<teams_chat::Error> for CoreError {
    fn from(error: teams_chat::Error) -> Self {
        let message = error.to_string();
        Self::chat(&error, message)
    }
}

impl From<AuthError> for CoreError {
    fn from(error: AuthError) -> Self {
        let message = error.to_string();
        if expired(&error) {
            Self::SessionExpired(message)
        } else {
            Self::Other(message)
        }
    }
}

impl From<String> for CoreError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for CoreError {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

impl From<CoreError> for String {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::SessionExpired(m) | CoreError::Offline(m) | CoreError::Other(m) => m,
            CoreError::RateLimited { message, .. } => message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_cause_and_the_text_stays_as_shown() {
        let limited = CoreError::from(teams_chat::Error::RateLimited {
            retry_after_secs: 30,
        });
        assert!(matches!(
            limited,
            CoreError::RateLimited {
                retry_after_secs: 30,
                ..
            }
        ));
        assert!(limited.to_string().contains("30 s"));
        let expired = CoreError::from(teams_chat::Error::Auth(AuthError::SessionExpired));
        assert!(matches!(expired, CoreError::SessionExpired(_)));
        assert_eq!(expired.to_string(), AuthError::SessionExpired.to_string());
        let shown = CoreError::chat(
            &teams_chat::Error::Media("x"),
            "Discussions indisponibles".into(),
        );
        assert_eq!(shown, CoreError::Other("Discussions indisponibles".into()));
        assert!(matches!(
            CoreError::from(AuthError::Failed("x".into())),
            CoreError::Other(_)
        ));
    }

    #[tokio::test]
    async fn an_unreachable_service_is_offline() {
        let refused = teams_auth::net::client()
            .get("http://127.0.0.1:1/")
            .send()
            .await
            .unwrap_err();
        let error = CoreError::from(teams_chat::Error::from(refused));
        assert!(matches!(error, CoreError::Offline(_)), "{error:?}");
    }
}
