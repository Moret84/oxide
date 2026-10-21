//! Teams stickers, read from the web client's lazy bundles (located via its webpack runtime) and cached 7 days per locale.
use serde::{Deserialize, Serialize};
use crate::emoji::{ByLocale, STATICS, fresh, save};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// `stickerUrlConfig.baseUrl` under the statics CDN.
const STICKERS: &str = "/evergreen-assets/stickerassets";
/// `stickerUrlConfig.cdnVersion` when the configuration was never read.
const CDN_VERSION: &str = "5";
/// `pG`×`IZ`: the size stickers are drawn at.
pub(crate) const SIZE: u32 = 250;
const CATALOG_BUNDLE: &str = "components-compose-extensions";

/// One sticker of the catalog, in one locale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sticker {
    pub id: String,
    pub category: String,
    pub name: String,
    pub keywords: String,
    pub url: String,
}

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static CATALOGS: ByLocale<Vec<Sticker>> = ByLocale::new();

/// Shares the emoticons' cache directory.
pub fn set_cache_dir(path: PathBuf) {
    let _ = CACHE_DIR.set(path);
}
/// The catalog of the app's language, or of the other one while it downloads; empty until one is.
pub(crate) fn current() -> Arc<Vec<Sticker>> {
    CATALOGS.current().unwrap_or_default()
}
pub(crate) fn has(locale: &str) -> bool {
    CATALOGS.has(locale)
}

/// Installs the fresh cached catalog of `locale`, else a new download, else a stale copy; false if none.
pub(crate) async fn load(locale: &'static str) -> bool {
    let path = CACHE_DIR
        .get()
        .map(|dir| dir.join("stickers").join(format!("{locale}.json")));
    let cached: Option<Vec<Sticker>> = match &path {
        Some(path) => tokio::fs::read(path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|s: &Vec<Sticker>| !s.is_empty()),
        None => None,
    };
    if path.as_deref().is_some_and(fresh)
        && let Some(stickers) = cached.clone()
    {
        CATALOGS.install(locale, stickers);
        return true;
    }
    if let Some(stickers) = download(locale).await {
        if let Some(path) = &path
            && let Ok(bytes) = serde_json::to_vec(&stickers)
        {
            save(path, &bytes, "sticker catalog").await;
        }
        CATALOGS.install(locale, stickers);
        return true;
    }
    match cached {
        Some(stickers) => {
            CATALOGS.install(locale, stickers);
            true
        }
        None => false,
    }
}
async fn download(locale: &str) -> Option<Vec<Sticker>> {
    let config = teams_auth::web_config::refresh().await.ok()?;
    let runtime_url = url::Url::parse(config.runtime_url.as_deref()?).ok()?;
    let version = config
        .sticker_cdn_version
        .unwrap_or_else(|| CDN_VERSION.to_owned());
    let http = teams_auth::net::client();
    let get = |url: url::Url| {
        let http = http.clone();
        async move {
            let response = http.get(url).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            response.text().await.ok()
        }
    };
    let runtime = get(runtime_url.clone()).await?;
    // Lazy bundles sit beside the runtime.
    let bundle =
        |name: &str| -> Option<url::Url> { runtime_url.join(&bundle_file(&runtime, name)?).ok() };
    let catalog = get(bundle(CATALOG_BUNDLE)?).await?;
    let locale_bundle = |locale: &str| {
        let url = bundle(&format!("funstuff-locale-{locale}"));
        async move {
            match url {
                Some(url) => get(url).await.unwrap_or_default(),
                None => String::new(),
            }
        }
    };
    let captions = locale_bundle(locale).await;
    let english = if locale == "en-us" {
        String::new()
    } else {
        locale_bundle("en-us").await
    };
    let stickers = parse(&catalog, &captions, &english, &version);
    (!stickers.is_empty()).then_some(stickers)
}
/// `{name}-{hash}.js` from the runtime's two maps, `id:"name"` and `id:"hash"`.
fn bundle_file(runtime: &str, name: &str) -> Option<String> {
    let quoted = format!(":\"{name}\"");
    let id = runtime.match_indices(&quoted).find_map(|(at, _)| {
        let before = &runtime[..at];
        let start = before.rfind([',', '{'])? + 1;
        let id = &before[start..];
        (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then_some(id)
    })?;
    let key = format!("{id}:\"");
    let hash = runtime.match_indices(&key).find_map(|(at, _)| {
        if !runtime[..at].ends_with([',', '{']) {
            return None;
        }
        let rest = &runtime[at + key.len()..];
        let value = &rest[..rest.find('"')?];
        (value.len() >= 8 && value.chars().all(|c| c.is_ascii_hexdigit())).then_some(value)
    })?;
    Some(format!("{name}-{hash}.js"))
}
/// A string literal's content after `key:"`, unescaped for the escapes captions use.
fn literal<'a>(text: &'a str, key: &str) -> Option<std::borrow::Cow<'a, str>> {
    let rest = text.split_once(&format!("{key}:\""))?.1;
    let mut end = 0;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'"' => break,
            _ => end += 1,
        }
    }
    let raw = rest.get(..end)?;
    Some(if raw.contains('\\') {
        unescape(raw).into()
    } else {
        raw.into()
    })
}
/// A JavaScript string literal's escapes: `\xHH`, `\uHHHH`, `\u{…}` and the usual ones.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let hex = |digits: &str| u32::from_str_radix(digits, 16).ok();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('x') => {
                let digits: String = chars.by_ref().take(2).collect();
                out.extend(hex(&digits).and_then(char::from_u32));
            }
            Some('u') if chars.peek() == Some(&'{') => {
                chars.next();
                let digits: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.extend(hex(&digits).and_then(char::from_u32));
            }
            Some('u') => {
                let digits: String = chars.by_ref().take(4).collect();
                let Some(unit) = hex(&digits) else { continue };
                // A surrogate pair is two escapes.
                if (0xD800..0xDC00).contains(&unit) && chars.peek() == Some(&'\\') {
                    let mut lookahead = chars.clone();
                    lookahead.next();
                    if lookahead.next() == Some('u') {
                        let low: String = lookahead.by_ref().take(4).collect();
                        if let Some(low) = hex(&low).filter(|l| (0xDC00..0xE000).contains(l)) {
                            chars = lookahead;
                            out.extend(char::from_u32(
                                0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00),
                            ));
                            continue;
                        }
                    }
                }
                out.extend(char::from_u32(unit));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
/// The hidden key of `namespace:key` (`MSTEAMS_I18N_OBFUSCATE_KEYS`): `k` + base-36 hash, keeping a `_mf` suffix.
fn obfuscated(namespace: &str, key: &str) -> String {
    const MODULUS: u64 = 1 << 40;
    let hash = format!("{namespace}:{key}")
        .encode_utf16()
        .fold(0u64, |w, c| (w * 131 + u64::from(c)) % MODULUS);
    let mut digits = Vec::new();
    let mut n = hash;
    loop {
        digits.push(char::from_digit((n % 36) as u32, 36).unwrap());
        n /= 36;
        if n == 0 {
            break;
        }
    }
    let suffix = if key.ends_with("_mf") { "_mf" } else { "" };
    format!("k{}{suffix}", digits.iter().rev().collect::<String>())
}
/// A caption written `key:"…"` or `"key":"…"`, under its hidden key (current builds) or plain one (older).
fn caption(bundle: &str, key: &str) -> Option<String> {
    let hidden = obfuscated("funstuff", key);
    [hidden, format!("\"{key}\""), key.to_owned()]
        .iter()
        .find_map(|k| {
            bundle
                .match_indices(&format!("{k}:\""))
                .find(|(at, _)| {
                    k.starts_with('"') || bundle[..*at].ends_with([',', '{', '\n', ' '])
                })
                .and_then(|(at, _)| literal(&bundle[at..], k))
        })
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
/// The catalog's `["id",{id:"…",link:"…",…}]` entries, minus EDU/meme (license or caption needed); captioned ones use their ready-made `link`.
pub(crate) fn parse(catalog: &str, captions: &str, english: &str, version: &str) -> Vec<Sticker> {
    let mut stickers: Vec<Sticker> = Vec::new();
    for (at, _) in catalog.match_indices("\",{id:\"") {
        let entry = &catalog[at + 2..];
        let Some(end) = entry.find('}') else { continue };
        let entry = &entry[..end];
        let field = |key: &str| literal(entry, key).map(|s| s.into_owned());
        let (Some(id), Some(link), Some(category)) =
            (field("id"), field("link"), field("category"))
        else {
            continue;
        };
        if category.starts_with("EDU_")
            || category == "Meme"
            || stickers.iter().any(|s| s.id == id)
            || !link
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            continue;
        }
        let text = |key: Option<String>| {
            key.and_then(|key| caption(captions, &key).or_else(|| caption(english, &key)))
        };
        let name = text(field("name")).unwrap_or_else(|| {
            id.split_once('_')
                .map_or(id.as_str(), |(_, n)| n)
                .replace('_', " ")
        });
        stickers.push(Sticker {
            url: format!(
                "{STATICS}{STICKERS}/{}-{SIZE}x{SIZE}/{link}?v={version}",
                category.to_lowercase()
            ),
            keywords: text(field("keywords")).unwrap_or_default(),
            id,
            category,
            name,
        });
    }
    stickers
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Against the real Teams page and CDN: `cargo test -p teams-core -- --ignored live`.
    #[tokio::test]
    #[ignore]
    async fn live_catalog_downloads_in_both_languages() {
        for locale in ["fr-fr", "en-us"] {
            let stickers = download(locale).await.expect("catalog");
            assert!(stickers.len() > 300, "{locale}: {}", stickers.len());
            let named = stickers.iter().filter(|s| !s.name.contains('_')).count();
            println!(
                "{locale}: {} stickers, {named} named, first {:?}",
                stickers.len(),
                stickers[0]
            );
        }
    }
    #[test]
    fn javascript_escapes_are_decoded() {
        assert_eq!(
            unescape(r#"\xE9crasant \u2019 \u{1F600} \ud83d\ude00 l\'ami \"x\""#),
            "écrasant ’ 😀 😀 l'ami \"x\""
        );
    }
    #[test]
    fn hidden_keys_match_the_web_client() {
        assert_eq!(
            obfuscated("funstuff", "collage_image_overlay_text"),
            "kd6lc012p"
        );
        assert_eq!(
            obfuscated("funstuff", "customize_reaction_button"),
            "khphw1cj"
        );
        assert!(obfuscated("funstuff", "x_mf").ends_with("_mf"));
        let bundle = format!(
            "{{{}:\"J’aime\"}}",
            obfuscated("funstuff", "stickers_bmac_like_name")
        );
        assert_eq!(
            caption(&bundle, "stickers_bmac_like_name").as_deref(),
            Some("J’aime")
        );
    }
    #[test]
    fn the_runtime_names_each_bundle_with_its_hash() {
        let runtime = r#"o.u=e=>(({906:"funstuff-locale-en-us",53931:"components-compose-extensions",153931:"other"}[e]||e)+"-"+{906:"5c224033126d9b4f",53931:"05d98065d7782b82"}[e]+".js")"#;
        assert_eq!(
            bundle_file(runtime, "components-compose-extensions").as_deref(),
            Some("components-compose-extensions-05d98065d7782b82.js")
        );
        assert_eq!(
            bundle_file(runtime, "funstuff-locale-en-us").as_deref(),
            Some("funstuff-locale-en-us-5c224033126d9b4f.js")
        );
        assert_eq!(bundle_file(runtime, "funstuff-locale-fr-fr"), None);
    }
    #[test]
    fn catalog_entries_take_their_captions_from_the_locale_bundle() {
        let catalog = r#"new Map([["stickers_BMac_like",{id:"stickers_BMac_like",name:"stickers_BMac_like_name",link:"stickers_BMac_like.png",category:"BMac",keywords:"stickers_BMac_like_keywords"}],["Clippy_Yeah",{id:"Clippy_Yeah",name:"stickers_clippy_yeah_name",link:"Clippy_Yeah.gif",previewLink:"Clippy_Yeah.png",category:"Clippy",keywords:"k"}],["EDU_x",{id:"EDU_x",name:"n",link:"x.png",category:"EDU_Robots"}],["Meme_a",{id:"Meme_a",name:"n",link:"a.png",category:"Meme"}]])"#;
        let fr = r#"{stickers_BMac_like_name:"J’aime",stickers_BMac_like_keywords:"aimer, pouce"}"#;
        let en = r#"{"stickers_clippy_yeah_name":"Clippy says \"yeah\""}"#;
        let stickers = parse(catalog, fr, en, "5");
        assert_eq!(stickers.len(), 2);
        assert_eq!(stickers[0].name, "J’aime");
        assert_eq!(stickers[0].keywords, "aimer, pouce");
        assert_eq!(
            stickers[0].url,
            "https://statics.teams.cdn.office.net/evergreen-assets/stickerassets/bmac-250x250/stickers_BMac_like.png?v=5"
        );
        assert_eq!(stickers[1].name, "Clippy says \"yeah\"");
        assert_eq!(stickers[1].category, "Clippy");
    }
}
