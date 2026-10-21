//! `SharepointShareService.shareFiles`: grants the chat members access and gets the link the message carries.
use crate::upload::guid;
use crate::{Auth, Body, Error, FilesClient, api, origin, share_id, str_at};
use http::Method;
use serde_json::{Value, json};
use teams_auth::Authenticator;
use url::Url;

/// `inviteApiUserPayloadLimit`.
const MAX_RECIPIENTS: usize = 100;
/// `deferRedeemHeaderLowerLimitOfRecipientCount`.
const DEFER_FROM: usize = 10;

/// Result of an invite: the link the message carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shared {
    pub share_url: Option<String>,
    pub share_id: Option<String>,
    pub permission_scope: Option<String>,
    /// `failedRecipientResults[].error.code`, when only some members got access.
    pub partial_error: Option<String>,
}

fn read(value: &Value, shared: &mut Shared) {
    for item in value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(link) = item.get("link").or_else(|| item.get("mainLink")) else {
            continue;
        };
        if shared.share_url.is_none() {
            shared.share_url = str_at(link, &["/webUrl"]).map(str::to_owned);
            shared.share_id = str_at(item, &["/id"]).map(str::to_owned);
            shared.permission_scope = str_at(link, &["/scope"]).map(str::to_owned);
        }
    }
    if shared.partial_error.is_none() {
        shared.partial_error = value
            .get("failedRecipientResults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|f| str_at(f, &["/error/code"]))
            .map(str::to_owned);
    }
}

impl<A: Authenticator> FilesClient<A> {
    /// Invites by file id when known, else through the `shareId` of its URL; recipients are AAD object ids.
    pub async fn invite(
        &self,
        site: &Url,
        file_id: Option<&str>,
        object_url: &str,
        recipients: &[String],
    ) -> Result<Shared, Error> {
        let url = match file_id.filter(|id| !id.is_empty()) {
            Some(id) => api(
                site,
                &format!("_api/v2.0/sites/root/items/{}/driveItem/invite", guid(id)?),
            )?,
            None => {
                let file = Url::parse(object_url).map_err(|_| Error::Decode)?;
                api(
                    &origin(&file)?,
                    &format!("_api/v2.0/shares/{}/driveItem/invite", share_id(object_url)),
                )?
            }
        };
        let mut shared = Shared::default();
        let batches: Vec<&[String]> = if recipients.is_empty() {
            vec![&[]]
        } else {
            recipients.chunks(MAX_RECIPIENTS).collect()
        };
        for batch in batches {
            let prefer = if batch.len() >= DEFER_FROM {
                "ReturnSpecificErrorCode,DeferRedeem,getDefaultLink"
            } else {
                "ReturnSpecificErrorCode,getDefaultLink"
            };
            let body = json!({
                "recipients": batch.iter().map(|oid| json!({"objectId": oid})).collect::<Vec<_>>()
            });
            let value = self
                .json(
                    Auth::Resource,
                    Method::POST,
                    url.clone(),
                    &[("prefer", prefer), ("scenario", "TeamsFiles")],
                    Body::Json(body),
                )
                .await?;
            read(&value, &mut shared);
        }
        Ok(shared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_first_link_and_partial_failures() {
        let mut shared = Shared::default();
        read(
            &json!({
                "value": [
                    {"id": "perm1", "grantedToIdentitiesV2": []},
                    {"id": "perm2", "link": {"webUrl": "https://c-my.sharepoint.com/:w:/g/personal/a/E1", "scope": "users"}}
                ],
                "failedRecipientResults": [{"error": {"code": "invalidRecipient"}}]
            }),
            &mut shared,
        );
        assert_eq!(shared.share_id.as_deref(), Some("perm2"));
        assert_eq!(shared.permission_scope.as_deref(), Some("users"));
        assert_eq!(shared.partial_error.as_deref(), Some("invalidRecipient"));
    }
}
