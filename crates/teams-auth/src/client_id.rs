//! Sign-in application ID, none built in: `OXIDE_CLIENT_ID`, then `client_id` in Oxide's `config.json`, then [`crate::web_config`].
use std::path::PathBuf;

use serde_json::Value;
use teams_i18n::t;
use uuid::Uuid;

use crate::{AuthError, web_config};

const ENV: &str = "OXIDE_CLIENT_ID";

/// The ID known without the network: configured, or read from the web client before.
pub fn known() -> Option<String> {
    configured().or_else(|| web_config::current()?.client_id)
}

/// The configured ID, or the one of the web client's configuration.
pub async fn resolve() -> Result<String, AuthError> {
    if let Some(id) = configured() {
        return Ok(id);
    }
    web_config::refresh()
        .await
        .ok()
        .and_then(|config| config.client_id)
        .ok_or_else(not_found)
}

/// Uses `id` for the rest of the run, as if it had been read; false if it is no GUID.
pub fn remember(id: &str) -> bool {
    let Some(id) = valid(id) else {
        return false;
    };
    web_config::set_client_id(id);
    true
}

pub(crate) fn not_found() -> AuthError {
    AuthError::Failed(t("auth.client_id.not_found").into())
}

fn configured() -> Option<String> {
    if let Some(id) = std::env::var(ENV).ok().and_then(|id| valid(&id)) {
        return Some(id);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let text =
        std::fs::read_to_string(home.join("Library/Application Support/Oxide/config.json")).ok()?;
    from_config(&text)
}

fn from_config(text: &str) -> Option<String> {
    let config: Value = serde_json::from_str(text).ok()?;
    valid(config.get("client_id")?.as_str()?)
}

/// A GUID, in the form Microsoft prints it.
pub(crate) fn valid(id: &str) -> Option<String> {
    Uuid::parse_str(id.trim())
        .ok()
        .map(|id| id.hyphenated().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0c6e1f4a-3b1d-4c8e-9f2a-7d5b6a4c3e21";

    #[test]
    fn the_config_file_must_hold_a_guid() {
        assert_eq!(
            from_config(&format!(r#"{{"client_id": " {} "}}"#, ID.to_uppercase())).as_deref(),
            Some(ID)
        );
        assert_eq!(from_config(r#"{"client_id": "teams"}"#), None);
        assert_eq!(from_config(r#"{"other": 1}"#), None);
        assert_eq!(from_config("not json"), None);
    }

    #[test]
    fn the_message_tells_where_to_set_the_id() {
        assert!(not_found().to_string().contains(ENV));
    }
}
