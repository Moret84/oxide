//! Where files live: the user's OneDrive for chats, the team site for channels.
use crate::{Auth, Body, Error, FilesClient, site_root, str_at};
use http::Method;
use serde_json::Value;
use teams_auth::{Authenticator, Service};
use teams_i18n::t;
use url::Url;

/// The user's OneDrive, from `beta/me/files/availability` or the `personalFileSite` user property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSite {
    /// `personalSiteUrl`, with a trailing `/`.
    pub site_url: Url,
    /// `personalRootFolderUrl`: the document library (`…/Documents`).
    pub root_folder_url: Url,
}
impl PersonalSite {
    pub fn parse(value: &Value) -> Option<Self> {
        // The property may itself be a JSON string.
        let value = match value.as_str() {
            Some(s) => serde_json::from_str(s).ok()?,
            None => value.clone(),
        };
        let site = str_at(&value, &["/personalSiteUrl", "/siteUrl"])?;
        let site_url = site_root(site).ok()?;
        let root = str_at(&value, &["/personalRootFolderUrl", "/listUrl"])
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}Documents", site_url));
        let root_folder_url = Url::parse(root.trim_end_matches('/')).ok()?;
        (root_folder_url.origin() == site_url.origin()).then_some(Self {
            site_url,
            root_folder_url,
        })
    }
    /// Server-relative path of the document library (`/personal/x/Documents`).
    pub fn library_path(&self) -> String {
        decoded(self.root_folder_url.path())
    }
    /// Server-relative folder of chat attachments: `pathname(listUrl + "/" + T("files_chatFileShareFolder"))`.
    pub fn chat_folder(&self, folder_name: &str) -> String {
        format!("{}/{}", self.library_path().trim_end_matches('/'), folder_name)
    }
}
pub(crate) fn decoded(path: &str) -> String {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned()
}

/// Localized name of the chat attachments folder (`files_chatFileShareFolder`).
pub fn chat_folder_name(locale: &str) -> &'static str {
    if locale.to_ascii_lowercase().starts_with("fr") {
        "Fichiers de conversation Microsoft Teams"
    } else {
        "Microsoft Teams Chat Files"
    }
}

impl<A: Authenticator> FilesClient<A> {
    /// `POST {middleTier}/beta/me/files/availability` (no body).
    pub async fn personal_site(&self) -> Result<PersonalSite, Error> {
        let url = self
            .auth
            .base_url(Service::MiddleTier)?
            .join("beta/me/files/availability")
            .map_err(|_| Error::Invalid(t("files.site.invalid_middle_tier_url").into()))?;
        let value = self
            .json(
                Auth::Service(Service::MiddleTier),
                Method::POST,
                url,
                &[("x-ms-client-caller", "PersonalFileDetailsService")],
                Body::Empty,
            )
            .await?;
        PersonalSite::parse(&value).ok_or(Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_availability_and_the_chat_folder() {
        let site = PersonalSite::parse(&json!({
            "personalSiteUrl": "https://contoso-my.sharepoint.com/personal/jdoe_contoso_com",
            "personalRootFolderUrl": "https://contoso-my.sharepoint.com/personal/jdoe_contoso_com/Documents"
        }))
        .unwrap();
        assert_eq!(
            site.site_url.as_str(),
            "https://contoso-my.sharepoint.com/personal/jdoe_contoso_com/"
        );
        assert_eq!(
            site.chat_folder(chat_folder_name("en-US")),
            "/personal/jdoe_contoso_com/Documents/Microsoft Teams Chat Files"
        );
        assert_eq!(
            chat_folder_name("fr_FR.UTF-8"),
            "Fichiers de conversation Microsoft Teams"
        );
    }

    #[test]
    fn accepts_the_property_as_a_json_string_and_rejects_foreign_roots() {
        let property = json!(
            r#"{"personalSiteUrl":"https://c-my.sharepoint.com/personal/a","personalRootFolderUrl":"https://c-my.sharepoint.com/personal/a/Documents"}"#
        );
        assert!(PersonalSite::parse(&property).is_some());
        assert!(
            PersonalSite::parse(&json!({
                "personalSiteUrl": "https://c-my.sharepoint.com/personal/a",
                "personalRootFolderUrl": "https://evil.example/Documents"
            }))
            .is_none()
        );
    }
}
