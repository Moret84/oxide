//! Chat "Partagé" tab: files from the AllFiles `SharedInChat[Fast]` index, links from `WorkingSetFiles`.
use crate::{Auth, Body, Error, FilesClient, request_id, str_at};
use http::Method;
use serde_json::Value;
use teams_auth::{Authenticator, Service};
use teams_i18n::t;
use url::Url;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedFile {
    /// `sharepointIds.listItemUniqueId`.
    pub id: String,
    pub name: String,
    /// `sharepointIds.fileUrl`.
    pub object_url: String,
    pub extension: String,
    /// `callerSharingInfo.sharedDateTime` (ISO 8601).
    pub shared_at: String,
    pub site_url: String,
    /// `callerSharingInfo.webUrl`: the sharing link.
    pub share_url: String,
    pub sender_name: String,
    pub sender_mri: String,
    pub message_id: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedLink {
    pub id: String,
    pub url: String,
    pub title: String,
    pub modified: String,
    pub sender_name: String,
    pub sender_mri: String,
    pub preview_image: Option<String>,
}

const LINKS_SELECT: &str = "Id,FileSize,FileCreatedTime,FileModifiedTime,StorageProviderContext,FileItemDefaultFolderType,Visualization,UserRelationship,ItemProperties/Shared/InternetMessageId,WorkingSetId,ItemProperties/Shared/SubjectProperty,ItemProperties/Shared/LastSharedWithMailboxOwnerBySmtp,ItemProperties/Shared/FileItemPreview,ItemProperties/Shared/FileItemConversationId,ItemProperties/Shared/LastSharedWithMailboxOwnerByDisplayName,ItemProperties/Shared/LastSharedWithMailboxOwnerDateTime,ItemProperties/Shared/TeamsMessageThreadId,ItemProperties/Shared/ImmutableFileItemReferenceId,ItemProperties/WebLink/WebUrl,ItemProperties/SharedFile/ContainerType,ItemProperties/SharedFile/ExternalResourceId,ItemProperties/Visualization/StaticTeaser,ItemProperties/WebLink/Title,ItemProperties/Shared/StorageProviderContext,ItemProperties/Shared/UserAtMentioned,ItemProperties/Shared/Importance,ItemProperties/Shared/MailboxOwnerInTo,ItemProperties/Shared/MailboxOwnerInCc,ItemProperties/Shared/FileIsFromMe,ItemProperties/Shared/SharedWithMailboxOwnerBySmtpHistory,ItemProperties/Shared/AnchorText,ItemProperties/Shared/ToParticipants,ItemProperties/Shared/AllStorageProviderContextCounts";

/// Thread ids go inside an OData string literal: refuse anything else than their alphabet.
fn thread(thread_id: &str) -> Result<&str, Error> {
    let ok = !thread_id.is_empty()
        && thread_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":@._-".contains(c));
    ok.then_some(thread_id)
        .ok_or_else(|| Error::Invalid(t("files.substrate.invalid_conversation").into()))
}
fn host(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}
fn web_url(value: &str) -> Option<String> {
    let url = Url::parse(value.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

pub(crate) fn files_of(value: &Value) -> Vec<SharedFile> {
    value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let name = str_at(x, &["/name"])?;
            let s = |p: &str| str_at(x, &[p]).unwrap_or_default().to_owned();
            Some(SharedFile {
                id: s("/sharepointIds/listItemUniqueId"),
                name: name.to_owned(),
                object_url: s("/sharepointIds/fileUrl"),
                extension: s("/file/extension").trim_start_matches('.').to_owned(),
                shared_at: s("/callerSharingInfo/sharedDateTime"),
                site_url: s("/sharepointIds/siteUrl"),
                share_url: s("/callerSharingInfo/webUrl"),
                sender_name: s("/callerSharingInfo/sharedBy/user/displayName"),
                sender_mri: s("/callerSharingInfo/sharedBy/user/mriId"),
                message_id: s("/callerSharingInfo/teamsMessageId"),
            })
        })
        .collect()
}
pub(crate) fn links_of(value: &Value) -> Vec<SharedLink> {
    value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let url = web_url(str_at(
                x,
                &["/ItemProperties/WebLink/WebUrl", "/Visualization/AccessUrl"],
            )?)?;
            Some(SharedLink {
                id: str_at(x, &["/Id"]).unwrap_or(&url).to_owned(),
                title: str_at(
                    x,
                    &[
                        "/ItemProperties/WebLink/Title",
                        "/Visualization/Title",
                        "/ItemProperties/Shared/AnchorText",
                    ],
                )
                .map(str::to_owned)
                .unwrap_or_else(|| host(&url)),
                modified: str_at(x, &["/FileModifiedTime"]).unwrap_or_default().to_owned(),
                sender_name: str_at(
                    x,
                    &["/ItemProperties/Shared/LastSharedWithMailboxOwnerByDisplayName"],
                )
                .unwrap_or_default()
                .to_owned(),
                sender_mri: str_at(x, &["/UserRelationship/LastSharedById"])
                    .unwrap_or_default()
                    .to_owned(),
                preview_image: str_at(x, &["/Visualization/PreviewImageUrl"]).map(str::to_owned),
                url,
            })
        })
        .collect()
}

impl<A: Authenticator> FilesClient<A> {
    fn substrate(&self, rest: &str) -> Result<Url, Error> {
        let base = self.auth.base_url(Service::Substrate)?;
        Url::parse(&format!("{}{rest}", base.as_str()))
            .map_err(|_| Error::Invalid(t("files.substrate.invalid_substrate_url").into()))
    }

    /// No `$top`/`$skip`: the web client sends none either.
    pub async fn shared_in_chat(&self, thread_id: &str, fast: bool) -> Result<Vec<SharedFile>, Error> {
        let index = if fast { "SharedInChatFast" } else { "SharedInChat" };
        let url = self.substrate(&format!(
            "allfiles/api/me/interactions('{index}')/items?$filter=callerSharingInfo/chatIds/any(n: n eq '{}')",
            thread(thread_id)?
        ))?;
        let id = request_id();
        let now = chrono::Utc::now().to_rfc3339();
        let value = self
            .json(
                Auth::Service(Service::Substrate),
                Method::GET,
                url,
                &[
                    ("content-type", "application/json;charset=UTF-8"),
                    ("x-client-localtime", &now),
                    ("clientrequestid", &id),
                    ("client-request-id", &id),
                    ("scenariotag", "TeamsSharedInChat_AllFiles"),
                ],
                Body::Empty,
            )
            .await?;
        Ok(files_of(&value))
    }

    /// `top` goes without `$`, as the service expects.
    pub async fn shared_links(&self, thread_id: &str) -> Result<Vec<SharedLink>, Error> {
        let url = self.substrate(&format!(
            "api/beta/me/WorkingSetFiles?$select={LINKS_SELECT}&$filter=(ItemProperties/Shared/StorageProviderContext eq 11) and (Visualization/Type eq 'weblink' or Visualization/Type eq 'sharepointweblink' or ItemProperties/SharedFile/ContainerType eq 'UnenrichedWeb') and (ItemProperties/Shared/TeamsMessageThreadId eq '{}')&top=50",
            thread(thread_id)?
        ))?;
        let id = request_id();
        let now = chrono::Utc::now().to_rfc3339();
        let value = self
            .json(
                Auth::Service(Service::Substrate),
                Method::GET,
                url,
                &[
                    ("x-client-localtime", &now),
                    ("client-request-id", &id),
                    ("x-ms-request-id", &id),
                    ("prefer", "exchange.behavior=\"ApplicationData,SubstrateFiles\""),
                ],
                Body::Empty,
            )
            .await?;
        Ok(links_of(&value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_all_files_items() {
        let files = files_of(&json!({"value": [
            {"name": "plan.pptx", "file": {"extension": ".pptx"},
             "sharepointIds": {"listItemUniqueId": "g1", "fileUrl": "https://c.sharepoint.com/sites/t/Shared Documents/plan.pptx", "siteUrl": "https://c.sharepoint.com/sites/t", "siteId": "s"},
             "callerSharingInfo": {"sharedDateTime": "2026-10-01T10:00:00Z", "webUrl": "https://c.sharepoint.com/:p:/s/t/E1",
               "sharedBy": {"user": {"displayName": "Alice", "mriId": "8:orgid:a"}}, "teamsMessageId": "1700"}},
            {"file": {"extension": "txt"}}
        ]}));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].extension, "pptx");
        assert_eq!(files[0].sender_name, "Alice");
        assert_eq!(files[0].share_url, "https://c.sharepoint.com/:p:/s/t/E1");
    }

    #[test]
    fn maps_working_set_links_and_drops_unsafe_urls() {
        let links = links_of(&json!({"value": [
            {"Id": "l1", "FileModifiedTime": "2026-10-02T08:00:00Z",
             "ItemProperties": {"WebLink": {"WebUrl": "https://github.com/x/y"}, "Shared": {"AnchorText": "le dépôt", "LastSharedWithMailboxOwnerByDisplayName": "Bob"}},
             "UserRelationship": {"LastSharedById": "8:orgid:b"}},
            {"Id": "l2", "ItemProperties": {"WebLink": {"WebUrl": "javascript:alert(1)"}}}
        ]}));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].title, "le dépôt");
        assert_eq!(links[0].sender_name, "Bob");
    }

    #[test]
    fn refuses_thread_ids_that_would_escape_the_filter() {
        assert!(thread("19:abc@thread.v2").is_ok());
        assert!(thread("x') or (1 eq 1").is_err());
    }
}
