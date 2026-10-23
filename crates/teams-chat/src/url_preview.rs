//! Link previews from the URL preview service (`urlPreviewService`, web module 87166).
use crate::{ChatClient, Error};
use http::{HeaderMap, HeaderValue, Method};
use serde_json::Value;
use teams_auth::{Authenticator, Service};

/// What `v1/url/info` says of a page; the service fills only what the page declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlInfo {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Through the preview proxy (`…/v1/url/content?url=…`), which wants AMS credentials.
    pub thumbnail: Option<String>,
    /// `thumbnail_meta`, kept as sent into `previewmeta`.
    pub thumbnail_meta: Option<Value>,
}

impl<A: Authenticator> ChatClient<A> {
    /// The web client's `getUrlPreview`: `&amp;` decoded, then the URL encoded as `encodeURIComponent` does.
    pub async fn url_preview(&self, url: &str) -> Result<UrlInfo, Error> {
        let url = url.replace("&amp;", "&");
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let id = uuid::Uuid::new_v4().to_string();
        for name in ["requestid", "correlationid", "transactionid"] {
            headers.insert(name, HeaderValue::from_str(&id).unwrap());
        }
        let answer: Value = self
            .service_json(
                Service::UrlPreview,
                "getUrlPreview",
                Method::GET,
                &format!("v1/url/info?url={}", encode_component(&url)),
                None,
                headers,
            )
            .await?;
        Ok(info(&answer, &url))
    }
}

fn info(answer: &Value, asked: &str) -> UrlInfo {
    let text = |key: &str| {
        answer
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    UrlInfo {
        url: text("url").unwrap_or_else(|| asked.to_owned()),
        title: text("title"),
        description: text("description"),
        thumbnail: text("thumbnail"),
        thumbnail_meta: answer
            .get("thumbnail_meta")
            .filter(|v| v.is_object())
            .cloned(),
    }
}

/// `encodeURIComponent`: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )` as UTF-8 `%XX`.
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn urls_are_encoded_as_encode_uri_component() {
        assert_eq!(
            encode_component("https://a.test/p?q=1&r=é ~"),
            "https%3A%2F%2Fa.test%2Fp%3Fq%3D1%26r%3D%C3%A9%20~"
        );
    }
    #[test]
    fn blank_fields_are_left_out_and_the_url_defaults_to_the_one_asked() {
        let answer = json!({"title": "  Rust  ", "description": "", "thumbnail_meta": {"width": 1200, "height": 630}});
        let info = info(&answer, "https://www.rust-lang.org/");
        assert_eq!(info.url, "https://www.rust-lang.org/");
        assert_eq!(info.title.as_deref(), Some("Rust"));
        assert_eq!(info.description, None);
        assert_eq!(info.thumbnail_meta, Some(json!({"width": 1200, "height": 630})));
    }
}
