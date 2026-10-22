//! Authenticated AMS downloads. No bearer/Skype headers are sent to external images.
use crate::{ChatClient, Error};
use http::HeaderMap;
use teams_auth::{Authenticator, Service};
use teams_i18n::t;
use url::Url;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
fn object_path(url: &Url) -> Option<&str> {
    let start = url.path().find("/v1/")?;
    let tail = &url.path()[start + 1..];
    let parts: Vec<_> = tail.split('/').collect();
    ((parts.get(1) == Some(&"objects") && parts.len() >= 4)
        || (parts.get(2) == Some(&"objects") && parts.len() >= 5))
        .then_some(tail)
}
/// The URL preview proxy that relays external images of cards (`…/urlp/v1/url/content?url=…`); it wants AMS credentials too.
fn url_preview_path(url: &Url) -> bool {
    url.path().ends_with("/v1/url/content")
}
fn ams_host(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    host.ends_with(".asm.skype.com")
        || host.ends_with(".asyncgw.teams.microsoft.com")
        || [
            "api.ams.gcc.teams.microsoft.com",
            "api.ams.gov.teams.microsoft.us",
            "api.ams.dod.teams.microsoft.us",
            "api.asyncgw.teams.microsoft.scloud",
            "api.asyncgw.teams.microsoftonline.cn",
            "api.asyncgw.teams.eaglex.ic.gov",
            "api.asyncgw.communications.svc.sovcloud.fr",
            "api.asyncgw.communications.svc.sovcloud.de",
        ]
        .contains(&host)
        || (host == "teams.microsoft.com" && url.path().starts_with("/asm/"))
}
fn is_sharepoint(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        [
            ".sharepoint.com",
            ".sharepoint.us",
            ".sharepoint.cn",
            ".sharepoint.de",
        ]
        .iter()
        .any(|s| host.ends_with(s))
    })
}
fn safe_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}
impl<A: Authenticator> ChatClient<A> {
    pub async fn download_image(&self, source: &str) -> Result<Vec<u8>, Error> {
        let source = Url::parse(source)
            .map_err(|_| Error::Media(t("chat.media.invalid_image_address")))?;
        if !safe_url(&source) {
            return Err(Error::Media(t("chat.media.image_address_isn_t_https_or")));
        }
        let ams = self.auth.base_url(Service::Ams).ok();
        let is_ams =
            ams_host(&source) || ams.as_ref().is_some_and(|b| b.origin() == source.origin());
        let mut target = if is_ams && let Some(tail) = object_path(&source) {
            if let Some(base) = &ams {
                let mut url =
                    Url::parse(&format!("{}/{}", base.as_str().trim_end_matches('/'), tail))?;
                url.set_query(source.query());
                if !url.query_pairs().any(|(k, _)| k == "v") {
                    url.query_pairs_mut().append_pair("v", "1");
                }
                url
            } else {
                return Err(Error::Media(t("chat.media.the_session_s_media_service_is")));
            }
        } else {
            source
        };
        let mut auth_retries = 0;
        for _ in 0..6 {
            let configured = ams.as_ref().is_some_and(|b| b.origin() == target.origin());
            // Only object and URL preview URLs of the discovered AMS or a Microsoft AMS host receive IC3 credentials.
            let authenticated_ams = (object_path(&target).is_some() || url_preview_path(&target))
                && (configured || (safe_url(&target) && ams_host(&target)));
            if !safe_url(&target) && !configured {
                return Err(Error::Media(t("chat.media.non_https_image_redirect_refused")));
            }
            let mut headers = HeaderMap::new();
            if authenticated_ams {
                self.auth.authorize(Service::Ams, &mut headers).await?;
            } else if is_sharepoint(&target) {
                self.auth.authorize_resource(&target, &mut headers).await?;
            }
            let response = self
                .http
                .get(target.clone())
                .headers(headers)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| {
                    Error::Media(t("chat.media.couldn_t_download_the_image"))
                })?;
            if response.status() == http::StatusCode::UNAUTHORIZED
                && authenticated_ams
                && auth_retries < 2
                && self
                    .auth
                    .on_challenge(Service::Ams, response.headers())
                    .await
            {
                auth_retries += 1;
                continue;
            }
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or(Error::Media(t("chat.media.invalid_image_redirect")))?;
                target = target.join(location).map_err(|_| {
                    Error::Media(t("chat.media.invalid_image_redirect"))
                })?;
                // Rebuild auth headers from scratch for the next origin, including CDN hops.
                continue;
            }
            if !response.status().is_success() {
                return Err(Error::Status {
                    endpoint: "image",
                    status: response.status(),
                    body: String::new(),
                });
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_IMAGE_BYTES as u64)
            {
                return Err(Error::Media(t("chat.media.image_is_too_large_to_preview")));
            }
            let mut response = response;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                Error::Media(t("chat.media.image_read_interrupted"))
            })? {
                if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                    return Err(Error::Media(t("chat.media.image_is_too_large_to_preview")));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(bytes);
        }
        Err(Error::Media(t("chat.media.too_many_redirects_for_this_image")))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_only_microsoft_ams_objects() {
        let a = Url::parse("https://eu-api.asm.skype.com/v1/objects/abc/views/imgo").unwrap();
        assert!(ams_host(&a));
        assert_eq!(object_path(&a), Some("v1/objects/abc/views/imgo"));
        assert!(!ams_host(
            &Url::parse("https://eu-api.asm.skype.com.attacker.test/v1/objects/abc/views/imgo")
                .unwrap()
        ));
        let preview = Url::parse(
            "https://eu-prod.asyncgw.teams.microsoft.com/urlp/v1/url/content?url=https%3a%2f%2fteams.github.com%2fpr.png",
        )
        .unwrap();
        assert!(ams_host(&preview) && url_preview_path(&preview));
        assert!(!url_preview_path(&a));
        assert!(!safe_url(
            &Url::parse("https://user:pass@example.test/image.png").unwrap()
        ));
    }
}
