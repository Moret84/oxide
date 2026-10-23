//! App definitions from the middle tier: the name and icon Teams shows for a bot's messages.
use crate::{ChatClient, Error};
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};

/// An app as the catalog describes it, named in the language asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppDefinition {
    pub id: String,
    pub name: String,
    /// `largeImageUrl`, the picture Teams gives the app's messages.
    pub icon: Option<String>,
    /// Its bots' ids, without the `28:` prefix.
    pub bot_ids: Vec<String>,
}

impl<A: Authenticator> ChatClient<A> {
    /// The web client's `getBatchedAppDefinitions`; the server localizes names per `X-Client-UI-Language` (`fr-fr`).
    pub async fn app_definitions(
        &self,
        ids: &[String],
        language: &str,
    ) -> Result<Vec<AppDefinition>, Error> {
        let mut headers = HeaderMap::new();
        if let Ok(language) = HeaderValue::from_str(language) {
            headers.insert("x-client-ui-language", language);
        }
        let answer: Value = self
            .service_json(
                Service::MiddleTier,
                "appDefinitions",
                Method::POST,
                "beta/users/apps/batchedDefinitions",
                Some(json!(ids)),
                headers,
            )
            .await?;
        Ok(definitions(&answer))
    }
}
/// A bare array as the web client reads it, or one wrapped in `value`.
fn definitions(answer: &Value) -> Vec<AppDefinition> {
    let list = answer
        .as_array()
        .or_else(|| answer.get("value")?.as_array())
        .or_else(|| answer.pointer("/value/appDefinitions")?.as_array());
    let text = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    list.into_iter()
        .flatten()
        .filter_map(|app| {
            Some(AppDefinition {
                id: text(app, "id")?,
                name: text(app, "name")?,
                icon: text(app, "largeImageUrl").filter(|u| u.starts_with("https://")),
                bot_ids: app
                    .get("bots")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|b| text(b, "id"))
                    .map(|id| id.trim_start_matches("28:").to_owned())
                    .collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definitions_keep_localized_name_icon_and_bots() {
        let app = json!({"id": "app-1", "name": "Flux de travail", "largeImageUrl": "https://cdn.test/w.png",
            "smallImageUrl": "https://cdn.test/s.png", "bots": [{"id": "28:BOT-1"}, {"id": "bot-2"}]});
        let expected = vec![AppDefinition {
            id: "app-1".into(),
            name: "Flux de travail".into(),
            icon: Some("https://cdn.test/w.png".into()),
            bot_ids: vec!["BOT-1".into(), "bot-2".into()],
        }];
        assert_eq!(
            definitions(&json!([app.clone(), {"id": "nameless"}])),
            expected
        );
        assert_eq!(definitions(&json!({"value": [app]})), expected);
        assert!(definitions(&json!({"errorCode": "x"})).is_empty());
    }
}
