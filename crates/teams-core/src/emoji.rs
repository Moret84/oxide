//! Teams emoticons from the CDN manifest; reaction keys are catalog (`like`) or Unicode (`1f44d_thumbsup`) ids, maybe `-toneN`.
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

/// Teams' quick-bar reactions in its order; glyphs and names come from the catalog.
const QUICK_KEYS: [&str; 6] = ["like", "heart", "laugh", "surprised", "sad", "angry"];

/// The web client's `staticsPath`.
pub(crate) const STATICS: &str = "https://statics.teams.cdn.office.net";
/// `emoticonAssetVersion` and `emojiMetadataCdnPath` when the web client's configuration was never read.
const VERSION: &str = "0f52465a47bf42f299c74a639443f33e";
const METADATA_PATH: &str = "/evergreen-assets/personal-expressions/v1/metadata/";
/// The web client downloads the manifest again after a week.
const MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ComposerEmoji {
    pub id: String,
    pub name: String,
    pub text: String,
    pub keywords: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct EmojiCategory {
    pub id: String,
    pub name: String,
    pub items: Vec<ComposerEmoji>,
}

#[derive(Deserialize)]
struct Manifest {
    categories: Vec<ManifestCategory>,
}
#[derive(Deserialize)]
struct ManifestCategory {
    id: String,
    title: String,
    emoticons: Vec<Emoticon>,
}
#[derive(Deserialize)]
struct Emoticon {
    id: String,
    description: String,
    unicode: String,
    #[serde(default)]
    shortcuts: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
}

/// One locale's catalog.
pub(crate) struct Catalog {
    categories: Vec<EmojiCategory>,
    /// Ids, then `(shortcut)` aliases, to the glyph and name.
    keys: HashMap<String, (String, String)>,
}
impl Catalog {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let manifest: Manifest = serde_json::from_slice(bytes).ok()?;
        let mut keys = HashMap::new();
        let mut aliases = Vec::new();
        let categories: Vec<EmojiCategory> = manifest
            .categories
            .into_iter()
            .map(|category| EmojiCategory {
                id: category.id,
                name: category.title,
                items: category
                    .emoticons
                    .into_iter()
                    .map(|e| {
                        let entry = (e.unicode.clone(), e.description.clone());
                        for shortcut in &e.shortcuts {
                            if let Some(alias) =
                                shortcut.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
                            {
                                aliases.push((alias.to_owned(), entry.clone()));
                            }
                        }
                        keys.insert(e.id.clone(), entry);
                        ComposerEmoji {
                            keywords: [e.keywords.join(" "), e.shortcuts.join(" ")].join(" "),
                            id: e.id,
                            name: e.description,
                            text: e.unicode,
                        }
                    })
                    .collect(),
            })
            .collect();
        for (alias, entry) in aliases {
            keys.entry(alias).or_insert(entry);
        }
        (!categories.is_empty()).then_some(Self { categories, keys })
    }
}

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
static CATALOGS: ByLocale<Catalog> = ByLocale::new();

/// Downloaded catalogs by locale.
pub(crate) struct ByLocale<T>(RwLock<Vec<(&'static str, Arc<T>)>>);
impl<T> ByLocale<T> {
    pub(crate) const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }
    /// The catalog of the app's language, or of the other one while it downloads.
    pub(crate) fn current(&self) -> Option<Arc<T>> {
        let catalogs = self.0.read().unwrap_or_else(|e| e.into_inner());
        let locale = locale();
        catalogs
            .iter()
            .find(|(l, _)| *l == locale)
            .or(catalogs.first())
            .map(|(_, c)| c.clone())
    }
    pub(crate) fn has(&self, locale: &str) -> bool {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(l, _)| *l == locale)
    }
    pub(crate) fn install(&self, locale: &'static str, catalog: T) {
        let mut catalogs = self.0.write().unwrap_or_else(|e| e.into_inner());
        catalogs.retain(|(l, _)| *l != locale);
        catalogs.push((locale, Arc::new(catalog)));
    }
}

/// Where downloaded manifests are kept; without it they are downloaded at every launch.
pub fn set_cache_dir(path: PathBuf) {
    let _ = CACHE_DIR.set(path);
}
/// The manifest of the app's language.
pub(crate) fn locale() -> &'static str {
    if teams_i18n::english() {
        "en-us"
    } else {
        "fr-fr"
    }
}
fn current() -> Option<Arc<Catalog>> {
    #[cfg(test)]
    tests::install_fixture();
    CATALOGS.current()
}
pub(crate) fn has(locale: &str) -> bool {
    CATALOGS.has(locale)
}
/// The picker's categories; empty until the catalog is downloaded.
pub fn categories() -> Vec<EmojiCategory> {
    current().map_or_else(Vec::new, |c| c.categories.clone())
}
/// Whether `id` is a catalog emoticon drawn as `text`.
pub(crate) fn is_emoticon(id: &str, text: &str) -> bool {
    current().is_some_and(|c| {
        c.categories
            .iter()
            .flat_map(|c| &c.items)
            .any(|e| e.id == id && e.text == text)
    })
}

/// Installs the fresh cached manifest of `locale`, else a new download, else a stale copy; false if none.
pub(crate) async fn load(locale: &'static str) -> bool {
    // The defaults serve when the configuration cannot be read.
    let config = teams_auth::web_config::refresh().await.unwrap_or_default();
    let version = config
        .emoticon_asset_version
        .unwrap_or_else(|| VERSION.to_owned());
    let metadata = format!(
        "{STATICS}/{}/{version}",
        config
            .emoji_metadata_path
            .as_deref()
            .unwrap_or(METADATA_PATH)
            .trim_matches('/')
    );
    let path = CACHE_DIR.get().map(|dir| {
        dir.join("emoticons")
            .join(&version)
            .join(format!("{locale}.json"))
    });
    let cached = match &path {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };
    if path.as_deref().is_some_and(fresh)
        && let Some(catalog) = cached.as_deref().and_then(Catalog::parse)
    {
        CATALOGS.install(locale, catalog);
        return true;
    }
    if let Some((bytes, catalog)) = download(&metadata, locale).await {
        if let Some(path) = &path {
            save(path, &bytes, "emoticon catalog").await;
        }
        CATALOGS.install(locale, catalog);
        return true;
    }
    match cached.as_deref().and_then(Catalog::parse) {
        Some(catalog) => {
            CATALOGS.install(locale, catalog);
            true
        }
        None => false,
    }
}
/// The locale's manifest, or `default.json` when the CDN has none, as the web client does.
async fn download(metadata: &str, locale: &str) -> Option<(Vec<u8>, Catalog)> {
    for name in [locale, "default"] {
        let url = format!("{metadata}/{name}.json");
        let Ok(response) = teams_auth::net::client().get(url).send().await else {
            return None;
        };
        if !response.status().is_success() {
            continue;
        }
        let bytes = response.bytes().await.ok()?.to_vec();
        return Catalog::parse(&bytes).map(|catalog| (bytes, catalog));
    }
    None
}
/// Whether a cached download is younger than the web client's one-week refresh.
pub(crate) fn fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < MAX_AGE)
}
/// Written beside, then renamed: a crash never leaves half a file.
pub(crate) async fn save(path: &Path, bytes: &[u8], what: &str) {
    let Some(dir) = path.parent() else { return };
    let partial = path.with_extension("partial");
    let saved = async {
        tokio::fs::create_dir_all(dir).await?;
        tokio::fs::write(&partial, bytes).await?;
        tokio::fs::rename(&partial, path).await
    };
    if let Err(e) = saved.await {
        eprintln!("teams-core: {what} not cached: {e}");
    }
}

/// Reactions with a skin tone: the modifier goes right after the hand or person.
fn tone(n: char) -> Option<char> {
    match n {
        '1' => Some('\u{1F3FB}'),
        '2' => Some('\u{1F3FC}'),
        '3' => Some('\u{1F3FD}'),
        '4' => Some('\u{1F3FE}'),
        '5' => Some('\u{1F3FF}'),
        _ => None,
    }
}

fn with_tone(base: &str, modifier: Option<char>) -> String {
    let Some(modifier) = modifier else {
        return base.to_owned();
    };
    // Drop the emoji presentation selector: the tone modifier already implies it.
    let mut chars: Vec<char> = base.chars().filter(|c| *c != '\u{FE0F}').collect();
    let at = chars.len().min(1);
    chars.insert(at, modifier);
    chars.into_iter().collect()
}

/// `1f44d_thumbsup` or `1f468-200d-1f4bb_technologist`: the code points before `_`.
fn unicode(id: &str) -> Option<String> {
    let (points, _) = id.split_once('_')?;
    points
        .split('-')
        .map(|p| u32::from_str_radix(p, 16).ok().and_then(char::from_u32))
        .collect()
}

/// The emoji to show for a reaction key, and a readable name for its tooltip.
pub fn reaction(key: &str) -> (String, String) {
    let (id, modifier) = match key.rsplit_once("-tone") {
        Some((id, n)) if n.len() == 1 => (id, n.chars().next().and_then(tone)),
        _ => (key, None),
    };
    let fallback = id.split_once('_').map_or(id, |(_, name)| name).to_owned();
    let found = current()
        .and_then(|c| c.keys.get(id).cloned())
        .or_else(|| unicode(id).map(|base| (base, fallback.clone())));
    match found {
        Some((base, name)) => (with_tone(&base, modifier), name),
        // Unknown id: its name is still more useful than nothing.
        None => (format!(":{fallback}:"), fallback),
    }
}
/// The quick bar's reactions as `(key, glyph, name)`; empty until the catalog is downloaded.
pub fn quick_reactions() -> Vec<(String, String, String)> {
    let Some(catalog) = current() else {
        return Vec::new();
    };
    QUICK_KEYS
        .iter()
        .filter_map(|key| {
            let (glyph, name) = catalog.keys.get(*key)?.clone();
            Some(((*key).to_owned(), glyph, name))
        })
        .collect()
}
/// The catalog emoticon whose artwork shows this reaction, if it has one.
pub fn artwork(key: &str) -> Option<String> {
    current()
        .is_some_and(|c| {
            c.categories
                .iter()
                .flat_map(|c| &c.items)
                .any(|e| e.id == key)
        })
        .then(|| key.to_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// A few entries in the manifest's shape; the real catalog is never in the repository.
    const FIXTURE: &str = r#"{"categories":[
        {"id":"smilies","title":"Émoticônes","emoticons":[
            {"id":"grinningfacewithsmilingeyes","description":"Visage souriant avec yeux rieurs","unicode":"😄","shortcuts":[":D","(lol)"],"keywords":["sourire","joyeux"]},
            {"id":"laugh","description":"Rire","unicode":"😆","shortcuts":["(laugh)"],"keywords":["rire"]},
            {"id":"heart","description":"Cœur","unicode":"❤️","shortcuts":["(heart)"],"keywords":["amour"]}
        ]},
        {"id":"gestures","title":"Gestes de la main","emoticons":[
            {"id":"like","description":"J’aime","unicode":"👍","shortcuts":["(like)"],"keywords":["pouce"]},
            {"id":"yes","description":"Oui","unicode":"👍","shortcuts":["(y)","(yes)"],"keywords":["pouce"]},
            {"id":"clap","description":"Applaudissements","unicode":"👏","shortcuts":["(clap)"],"keywords":["bravo"]},
            {"id":"victory","description":"Victoire","unicode":"✌️","shortcuts":["(victory)"],"keywords":["paix"]},
            {"id":"1f44d_thumbsup","description":"Pouce levé","unicode":"👍","shortcuts":["(thumbsup)"],"keywords":["pouce"]}
        ]}
    ]}"#;
    pub(crate) fn install_fixture() {
        if !has("fr-fr") {
            CATALOGS.install(
                "fr-fr",
                Catalog::parse(FIXTURE.as_bytes()).expect("fixture"),
            );
        }
    }
    #[test]
    fn manifest_becomes_picker_categories_with_searchable_keywords() {
        let categories = categories();
        assert_eq!(categories.len(), 2);
        assert_eq!(categories[0].name, "Émoticônes");
        let first = &categories[0].items[0];
        assert_eq!(first.text, "😄");
        assert!(first.keywords.contains("sourire") && first.keywords.contains("(lol)"));
        assert!(is_emoticon("clap", "👏"));
        assert!(!is_emoticon("clap", "👍"));
    }
    #[test]
    fn classic_catalog_and_unicode_keys_become_emoji_with_their_skin_tone() {
        assert_eq!(reaction("like"), ("👍".into(), "J’aime".into()));
        assert_eq!(reaction("heart").1, "Cœur");
        assert_eq!(reaction("yes"), ("👍".into(), "Oui".into()));
        assert_eq!(reaction("yes-tone2").0, "👍🏼");
        assert_eq!(reaction("clap-tone5").0, "👏🏿");
        assert_eq!(reaction("victory-tone1").0, "✌🏻");
        assert_eq!(reaction("lol").0, "😄");
        assert_eq!(reaction("1f44d_thumbsup").0, "👍");
        assert_eq!(reaction("1f44d_thumbsup-tone3").0, "👍🏽");
        assert_eq!(reaction("1f468-200d-1f4bb_technologist").0, "👨‍💻");
        assert_eq!(
            reaction("somethingnew"),
            (":somethingnew:".into(), "somethingnew".into())
        );
        assert_eq!(artwork("clap").as_deref(), Some("clap"));
        assert_eq!(artwork("clap-tone2"), None);
    }
    #[test]
    fn quick_bar_keeps_teams_order_and_skips_reactions_missing_from_the_catalog() {
        let keys: Vec<_> = quick_reactions().into_iter().map(|(k, ..)| k).collect();
        assert_eq!(keys, ["like", "heart", "laugh"]);
    }
    #[test]
    fn unreadable_or_empty_manifests_are_rejected() {
        assert!(Catalog::parse(b"<html>").is_none());
        assert!(Catalog::parse(br#"{"categories":[]}"#).is_none());
    }
}
