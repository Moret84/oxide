//! TURN relay credentials (TRAP) and the ICE servers built from them.
use std::sync::Arc;

use http::{HeaderMap, HeaderValue, Method};
use serde_json::Value;
use teams_auth::{Authenticator, Service};

use crate::Error;
use crate::http::Http;

/// Relay of the embedded `relayConfig` (`Relay.Turn`).
pub const DEFAULT_RELAY: &str = "default.relay.teams.microsoft.com";
const UDP_PORT: u16 = 3478;
const TCP_PORT: u16 = 443;
const TLS_PORT: u16 = 443;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapCredentials {
    pub realm: Option<String>,
    pub username: String,
    pub password: String,
    /// Lifetime in seconds.
    pub expires: u64,
}

impl TrapCredentials {
    /// `{expires, tokens:[{realm,username,password}]}`, or the older flat shape.
    pub fn parse(value: &Value) -> Option<Self> {
        let expires = value.get("expires").and_then(|e| {
            e.as_u64()
                .or_else(|| e.as_str().and_then(|s| s.parse().ok()))
        });
        let token = value
            .get("tokens")
            .and_then(Value::as_array)
            .and_then(|t| t.first())
            .unwrap_or(value);
        let text = |key: &str| {
            token
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            realm: text("realm").map(|r| r.trim_matches('"').to_owned()),
            username: text("username")?,
            password: text("password")?,
            expires: expires
                .or_else(|| token.get("expires").and_then(Value::as_u64))
                .unwrap_or(3600),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// TURN over UDP, TCP and TLS with credentials; plain STUN without, which fails behind a symmetric NAT or proxy.
pub fn ice_servers(credentials: Option<&TrapCredentials>, relay: &str) -> Vec<IceServer> {
    match credentials {
        Some(c) => vec![IceServer {
            urls: vec![
                format!("turn:{relay}:{UDP_PORT}?transport=udp"),
                format!("turn:{relay}:{TCP_PORT}?transport=tcp"),
                format!("turns:{relay}:{TLS_PORT}"),
            ],
            username: Some(c.username.clone()),
            credential: Some(c.password.clone()),
        }],
        None => vec![IceServer {
            urls: vec![format!("stun:{relay}:{UDP_PORT}")],
            username: None,
            credential: None,
        }],
    }
}

pub struct TrapClient<A> {
    http: Http<A>,
}

impl<A: Authenticator> TrapClient<A> {
    pub fn new(auth: Arc<A>) -> Self {
        Self {
            http: Http::new(auth),
        }
    }

    pub async fn credentials(&self) -> Result<TrapCredentials, Error> {
        let url = self.http.auth.base_url(Service::Trap)?;
        let mut headers = HeaderMap::new();
        headers.insert("api-version", HeaderValue::from_static("2"));
        let value = self
            .http
            .json(Service::Trap, Method::GET, url, None, headers)
            .await?;
        TrapCredentials::parse(&value).ok_or(Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_both_token_shapes_and_builds_turn_servers() {
        let new = TrapCredentials::parse(&json!({"expires": 28800,
            "tokens": [{"realm": "\"rtcmedia\"", "username": "u", "password": "p"}]}))
        .unwrap();
        assert_eq!(new.realm.as_deref(), Some("rtcmedia"));
        assert_eq!(new.expires, 28800);
        let old = TrapCredentials::parse(&json!({"realm": "r", "username": "u2",
            "password": "p2", "expires": "60"}))
        .unwrap();
        assert_eq!((old.username.as_str(), old.expires), ("u2", 60));
        assert!(TrapCredentials::parse(&json!({"tokens": []})).is_none());

        let servers = ice_servers(Some(&new), DEFAULT_RELAY);
        assert_eq!(
            servers[0].urls,
            [
                "turn:default.relay.teams.microsoft.com:3478?transport=udp",
                "turn:default.relay.teams.microsoft.com:443?transport=tcp",
                "turns:default.relay.teams.microsoft.com:443",
            ]
        );
        assert_eq!(servers[0].credential.as_deref(), Some("p"));
        assert_eq!(
            ice_servers(None, DEFAULT_RELAY)[0].urls,
            ["stun:default.relay.teams.microsoft.com:3478"]
        );
    }
}
