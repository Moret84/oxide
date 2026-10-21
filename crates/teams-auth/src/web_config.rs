//! Settings of the web client's `config-prod` bundle, cached for a week in `~/Library/Caches/Oxide/web-config.json` (stale copy kept offline).
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use teams_i18n::t;
use url::Url;

use crate::AuthError;

const PAGE: &str = "https://teams.microsoft.com/v2/";
/// Teams sends other browsers to an "unsupported browser" page.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Bundle of the production configuration, listed by the page.
const BUNDLE: &str = "config-prod-";
const RUNTIME: &str = "runtime-";
const MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebConfig {
    /// `auth.clientId`.
    pub client_id: Option<String>,
    /// `teamsEchoBotMri`.
    pub echo_bot_mri: Option<String>,
    /// `emoticonAssetVersion`.
    pub emoticon_asset_version: Option<String>,
    /// `emojiMetadataCdnPath`, a path on the Teams statics CDN.
    pub emoji_metadata_path: Option<String>,
    /// The page's webpack runtime, which names every lazy bundle (the sticker catalog among them) with its hash.
    pub runtime_url: Option<String>,
    /// `stickerUrlConfig.cdnVersion`, the `v=` of sticker images.
    pub sticker_cdn_version: Option<String>,
    /// Unix seconds of the download.
    fetched_at: u64,
}
impl WebConfig {
    /// Less than a week old, and read by a version that also looks for the runtime.
    fn fresh(&self) -> bool {
        now().saturating_sub(self.fetched_at) < MAX_AGE.as_secs() && self.runtime_url.is_some()
    }
}

static CURRENT: RwLock<Option<WebConfig>> = RwLock::new(None);
/// One download at a time: the others then find it fresh.
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The configuration last read, in this run or from the cache, however old.
pub fn current() -> Option<WebConfig> {
    let memory = CURRENT
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    memory.or_else(|| {
        let cached = read_cache()?;
        set(cached.clone());
        Some(cached)
    })
}

/// [`current`] while less than a week old, else downloaded again; the old copy when the download fails.
pub async fn refresh() -> Result<WebConfig, AuthError> {
    let _one = REFRESH.lock().await;
    let known = current();
    if let Some(config) = known.as_ref().filter(|c| c.fresh()) {
        return Ok(config.clone());
    }
    let page = Url::parse(PAGE).unwrap();
    match download(&crate::net::client(), &page).await {
        Ok(config) => {
            write_cache(&config);
            set(config.clone());
            Ok(config)
        }
        Err(error) => known.ok_or(error),
    }
}

fn set(config: WebConfig) {
    *CURRENT
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(config);
}

/// Sets the application ID for the rest of the run, keeping the other settings.
pub(crate) fn set_client_id(id: String) {
    let mut current = CURRENT
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    current.get_or_insert_with(Default::default).client_id = Some(id);
}

fn unreachable() -> AuthError {
    AuthError::Failed(t("auth.web_config.unreadable").into())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches/Oxide/web-config.json"))
}

fn read_cache() -> Option<WebConfig> {
    let config: WebConfig = serde_json::from_slice(&std::fs::read(cache_path()?).ok()?).ok()?;
    // Checked again: the file may have been edited.
    Some(WebConfig {
        fetched_at: config.fetched_at,
        ..sanitized(config)
    })
}

/// Written beside, then renamed: a crash never leaves half a file.
fn write_cache(config: &WebConfig) {
    let Some(path) = cache_path() else { return };
    let Ok(bytes) = serde_json::to_vec_pretty(config) else {
        return;
    };
    let partial = path.with_extension("partial");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&partial, bytes))
        .and_then(|()| std::fs::rename(&partial, &path));
    if let Err(e) = written {
        eprintln!("teams-auth: web configuration not cached: {e}");
    }
}

async fn get(http: &reqwest::Client, url: Url) -> Result<String, AuthError> {
    let response = http
        .get(url)
        .header("user-agent", USER_AGENT)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| unreachable())?;
    if !response.status().is_success() {
        return Err(unreachable());
    }
    response.text().await.map_err(|_| unreachable())
}

/// The page lists the configuration bundle, which holds every setting as `key:[{value:…}]`.
async fn download(http: &reqwest::Client, page: &Url) -> Result<WebConfig, AuthError> {
    let html = get(http, page.clone()).await?;
    let bundle = bundle_url(page, &html, BUNDLE).ok_or_else(unreachable)?;
    let config = parse(&get(http, bundle).await?);
    if config == WebConfig::default() {
        return Err(unreachable());
    }
    Ok(WebConfig {
        fetched_at: now(),
        runtime_url: bundle_url(page, &html, RUNTIME).map(String::from),
        ..config
    })
}

fn bundle_url(page: &Url, html: &str, prefix: &str) -> Option<Url> {
    let at = html.find(prefix)?;
    let start = html[..at].rfind(['"', '\''])? + 1;
    let end = at + html[at..].find(['"', '\''])?;
    let url = page.join(&html[start..end]).ok()?;
    let name = url.path_segments()?.next_back()?;
    (name.starts_with(prefix)
        && name.ends_with(".js")
        && (url.scheme() == "https" || url.origin() == page.origin()))
    .then_some(url)
}

fn parse(script: &str) -> WebConfig {
    let text = |key| value(script, key).map(str::to_owned);
    sanitized(WebConfig {
        client_id: text("auth:{clientId"),
        echo_bot_mri: text("teamsEchoBotMri"),
        emoticon_asset_version: text("emoticonAssetVersion"),
        emoji_metadata_path: text("emojiMetadataCdnPath"),
        runtime_url: None,
        sticker_cdn_version: number_in(script, "stickerUrlConfig", "cdnVersion"),
        fetched_at: 0,
    })
}

/// What follows `key:[{value:<opener>` for a setting with a single, unconditional value.
fn setting<'a>(script: &'a str, key: &str, opener: &str) -> Option<&'a str> {
    let pattern = format!("{key}:[{{value:{opener}");
    let at = script
        .match_indices(&pattern)
        .map(|(at, _)| at)
        .find(|&at| script[..at].ends_with([',', '{']))?;
    Some(&script[at + pattern.len()..])
}

/// A number inside an object setting, such as `stickerUrlConfig:[{value:{…,cdnVersion:5}}]`.
fn number_in(script: &str, key: &str, field: &str) -> Option<String> {
    let rest = setting(script, key, "{")?;
    let object = &rest[..rest.find('}')?];
    let value = object.split(',').find_map(|pair| pair.strip_prefix(&format!("{field}:")))?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 9 && value.chars().all(|c| c.is_ascii_digit()))
        .then(|| value.to_owned())
}

/// The string of a setting with a single, unconditional value.
fn value<'a>(script: &'a str, key: &str) -> Option<&'a str> {
    let rest = setting(script, key, "\"")?;
    Some(&rest[..rest.find('"')?])
}

/// Drops what does not have the expected shape, so a changed bundle falls back to defaults.
fn sanitized(config: WebConfig) -> WebConfig {
    WebConfig {
        client_id: config.client_id.and_then(|id| crate::client_id::valid(&id)),
        echo_bot_mri: config.echo_bot_mri.filter(|mri| {
            mri.strip_prefix("28:").is_some_and(|id| {
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        }),
        emoticon_asset_version: config.emoticon_asset_version.filter(|version| {
            (1..=64).contains(&version.len()) && version.chars().all(|c| c.is_ascii_alphanumeric())
        }),
        emoji_metadata_path: config.emoji_metadata_path.filter(|path| {
            path.starts_with('/')
                && !path.contains("..")
                && path
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c))
        }),
        runtime_url: config.runtime_url.filter(|url| {
            Url::parse(url).is_ok_and(|u| {
                u.scheme() == "https"
                    && u.path_segments()
                        .and_then(|mut s| s.next_back())
                        .is_some_and(|name| name.starts_with(RUNTIME) && name.ends_with(".js"))
            })
        }),
        sticker_cdn_version: config.sticker_cdn_version,
        fetched_at: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header_regex, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ID: &str = "0c6e1f4a-3b1d-4c8e-9f2a-7d5b6a4c3e21";
    const SCRIPT: &str = r#"const t={configs:{auth:{clientId:[{value:"0c6e1f4a-3b1d-4c8e-9f2a-7d5b6a4c3e21"}],enableAuthV2:[{value:!0}]},calling:{listedMRIs:[{value:["28:aaaa"]}],teamsEchoBotMri:[{value:"28:cf28171e-0000-47e4-a1d6-79460b0b3ca0"}]},expressions:{emojiMetadataCdnPath:[{value:"/evergreen-assets/personal-expressions/v1/metadata/"}],emoticonAssetVersion:[{value:"0f52465a47bf42f299c74a639443f33e"}]},messaging:{stickerUrlConfig:[{value:{baseUrl:"/evergreen-assets/stickerassets",cdnVersion:5}}]},other:{clientId:[{value:"ffffffff-ffff-ffff-ffff-ffffffffffff"}]}}}"#;

    #[test]
    fn reads_each_setting_from_the_bundle() {
        assert_eq!(
            parse(SCRIPT),
            WebConfig {
                client_id: Some(ID.into()),
                echo_bot_mri: Some("28:cf28171e-0000-47e4-a1d6-79460b0b3ca0".into()),
                emoticon_asset_version: Some("0f52465a47bf42f299c74a639443f33e".into()),
                emoji_metadata_path: Some(
                    "/evergreen-assets/personal-expressions/v1/metadata/".into()
                ),
                runtime_url: None,
                sticker_cdn_version: Some("5".into()),
                fetched_at: 0,
            }
        );
        assert_eq!(parse("const t={}"), WebConfig::default());
    }

    #[test]
    fn a_setting_is_matched_by_its_whole_name() {
        assert_eq!(
            value(r#"{xteamsEchoBotMri:[{value:"28:a"}]}"#, "teamsEchoBotMri"),
            None
        );
        assert_eq!(
            value(
                r#"{xteamsEchoBotMri:[{value:"28:a"}],teamsEchoBotMri:[{value:"28:b"}]}"#,
                "teamsEchoBotMri"
            ),
            Some("28:b")
        );
        // Values that depend on the platform are left to the defaults.
        assert_eq!(
            value(
                r#"{teamsEchoBotMri:[{platform:[1],value:"28:a"}]}"#,
                "teamsEchoBotMri"
            ),
            None
        );
    }

    #[test]
    fn unexpected_shapes_are_dropped() {
        let config = sanitized(WebConfig {
            client_id: Some("not-a-guid".into()),
            echo_bot_mri: Some("8:orgid:someone".into()),
            emoticon_asset_version: Some("../x".into()),
            emoji_metadata_path: Some("https://elsewhere.example/".into()),
            runtime_url: Some("http://cdn.example/runtime-1.js".into()),
            sticker_cdn_version: None,
            fetched_at: 0,
        });
        assert_eq!(config, WebConfig::default());
        assert!(
            sanitized(WebConfig {
                emoji_metadata_path: Some("/a/../b/".into()),
                ..Default::default()
            })
            .emoji_metadata_path
            .is_none()
        );
    }

    #[test]
    fn a_copy_is_fresh_for_a_week() {
        let config = |age: u64| WebConfig {
            fetched_at: now() - age,
            runtime_url: Some("https://cdn.example/runtime-1.js".into()),
            ..Default::default()
        };
        assert!(config(6 * 86_400).fresh());
        assert!(!config(8 * 86_400).fresh());
        // Copies saved before the runtime was read are downloaded again.
        assert!(!WebConfig { runtime_url: None, ..config(60) }.fresh());
    }

    #[test]
    fn finds_the_production_bundle_among_the_page_scripts() {
        let page = Url::parse("https://teams.microsoft.com/v2/").unwrap();
        let html = r#"[{path:"https://cdn.example/a/config-life-1.js"},{path:"https://cdn.example/a/config-prod-80421014aac5422c.js",integrity:"x"}]"#;
        assert_eq!(
            bundle_url(&page, html, BUNDLE).unwrap().as_str(),
            "https://cdn.example/a/config-prod-80421014aac5422c.js"
        );
        assert!(bundle_url(&page, r#"{path:"http://cdn.example/config-prod-1.js"}"#, BUNDLE).is_none());
        assert!(bundle_url(&page, r#"{path:"https://cdn.example/config-prod-1.css"}"#, BUNDLE).is_none());
        assert!(bundle_url(&page, "no bundle", BUNDLE).is_none());
        let html = r#"[{path:"https://cdn.example/a/runtime-4605033e3deea06a.js"},{path:"https://cdn.example/a/config-prod-8.js"}]"#;
        assert_eq!(
            bundle_url(&page, html, RUNTIME).unwrap().as_str(),
            "https://cdn.example/a/runtime-4605033e3deea06a.js"
        );
    }

    #[tokio::test]
    async fn downloads_the_configuration_through_the_page_like_a_browser() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .and(header_regex("user-agent", r"Version/[0-9.]+ Safari/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<script>load([{path:"/assets/runtime-123.js"},{path:"/assets/config-prod-abc.js",isPolyfill:!1}])</script>"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/assets/config-prod-abc.js"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SCRIPT))
            .mount(&server)
            .await;
        let page = Url::parse(&format!("{}/v2/", server.uri())).unwrap();
        let config = download(&crate::net::client(), &page).await.unwrap();
        assert_eq!(config.client_id.as_deref(), Some(ID));
        assert_eq!(
            config.runtime_url,
            Some(format!("{}/assets/runtime-123.js", server.uri()))
        );
        assert!(config.fresh());
    }

    #[tokio::test]
    async fn an_unsupported_browser_redirect_is_no_configuration() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "/v2/unsupported-browser"),
            )
            .mount(&server)
            .await;
        let page = Url::parse(&format!("{}/v2/", server.uri())).unwrap();
        assert!(download(&crate::net::client(), &page).await.is_err());
    }
}
