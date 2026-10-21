//! Automatic replies ("Out of office") through the middle tier, as the web settings page does.
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use teams_auth::Authenticator;

use crate::client::ChatClient;
use crate::error::Error;
use crate::types::null_default;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepliesStatus {
    #[default]
    Disabled,
    AlwaysEnabled,
    Scheduled,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalAudience {
    #[default]
    None,
    ContactsOnly,
    All,
}
/// The flat object both calls exchange; messages are plain text, dates UTC ISO 8601.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticReplies {
    #[serde(default)]
    pub status: RepliesStatus,
    #[serde(default, deserialize_with = "null_default")]
    pub internal_reply_message: String,
    #[serde(default, deserialize_with = "null_default")]
    pub external_reply_message: String,
    #[serde(default, deserialize_with = "null_default")]
    pub external_audience: ExternalAudience,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_start_date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_end_date_time: Option<String>,
}

impl<A: Authenticator> ChatClient<A> {
    pub async fn automatic_replies(&self) -> Result<AutomaticReplies, Error> {
        let v = self
            .mt(
                "getOofSettings",
                Method::GET,
                "beta/users/automaticRepliesSetting",
                None,
            )
            .await?;
        let v = match v {
            Value::Object(mut o) if o.get("value").is_some_and(Value::is_object) => {
                o.remove("value").unwrap_or_default()
            }
            other => other,
        };
        serde_json::from_value(v).map_err(|source| Error::Decode {
            endpoint: "getOofSettings",
            source,
        })
    }
    pub async fn set_automatic_replies(&self, replies: &AutomaticReplies) -> Result<(), Error> {
        let body = serde_json::to_value(replies).expect("replies serialize");
        self.mt(
            "setOofMessage",
            Method::POST,
            "beta/users/updateAutomaticRepliesSetting",
            Some(body),
        )
        .await
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn replies_round_trip_the_web_client_shape() {
        let replies: AutomaticReplies = serde_json::from_value(json!({
            "status": "scheduled", "internalReplyMessage": "Back Monday",
            "externalReplyMessage": null, "externalAudience": null,
            "scheduledStartDateTime": "2026-10-24T07:00:00.000Z",
            "scheduledEndDateTime": "2026-10-27T08:00:00.000Z"
        }))
        .unwrap();
        assert_eq!(replies.status, RepliesStatus::Scheduled);
        assert_eq!(replies.external_audience, ExternalAudience::None);
        let back = serde_json::to_value(&replies).unwrap();
        assert_eq!(back["status"], "scheduled");
        assert_eq!(back["externalAudience"], "none");
        assert_eq!(back["externalReplyMessage"], "");
    }
}
