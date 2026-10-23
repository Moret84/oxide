//! Link previews: the card under a message with a link, and the one the compose box prepares for the draft's first link.
//! Read as the web client's `Chat Helper` reads `properties.links`, sent as its compose box does (`283115.Xh`).
use super::view::ImageView;
use super::*;
use crate::html::{self, InlineImage};
use serde_json::{Value, json};
use std::collections::HashSet;
use teams_chat::UrlInfo;

/// The web client's `composeLinkChanged` debounce, for a link the user has finished (a space after it, or a formatted link).
const SETTLED: Duration = Duration::from_millis(125);
/// A link at the end of the draft may still be being typed: asked for only once typing pauses.
const TYPING: Duration = Duration::from_millis(800);

/// A message's link preview: the first entry of its links, as the web client shows only one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<InlineImage>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct LinkPreviewView {
    pub url: String,
    /// The page's host, without `www.`.
    pub site: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<ImageView>,
    /// The compose box's preview while the service is asked.
    pub loading: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LinkPreviewAction {
    /// Sends the draft without its preview; the same link is not offered again in this draft.
    Dismiss,
}
#[derive(Debug, Clone)]
pub(crate) enum LinkPreviewMsg {
    Action(LinkPreviewAction),
    /// The debounce of `(key, url)` is over.
    Due(u64, String, String),
    Loaded(u64, String, String, Result<UrlInfo, String>),
}
#[derive(Debug, Clone, PartialEq)]
enum Status {
    Waiting,
    Loading,
    Ready(UrlInfo),
    Failed,
}
#[derive(Debug, Clone, PartialEq)]
struct DraftLink {
    /// The draft it belongs to ([`App::draft_key`]).
    key: String,
    url: String,
    status: Status,
}
#[derive(Default)]
pub(crate) struct LinkPreviewState {
    draft: Option<DraftLink>,
    /// Links whose preview was closed, by draft key; forgotten once that draft has no link.
    dismissed: HashSet<(String, String)>,
}

/// From `properties.links` (a JSON string, or an array), else `properties.urlpreviews`.
pub(crate) fn from_message(message: &Message) -> Option<LinkPreview> {
    let decode = |value: &Value| {
        let mut v = value.clone();
        for _ in 0..2 {
            match v.as_str() {
                Some(s) => v = serde_json::from_str(s).unwrap_or(Value::Null),
                None => break,
            }
        }
        v
    };
    let text = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(|s| html::to_text(s).trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    let size = |meta: Option<&Value>| {
        let meta = decode(meta?);
        let side = |k| meta.get(k)?.as_u64().filter(|n| *n > 0).map(|n| n as u32);
        Some((side("width")?, side("height")?))
    };
    let (entry, preview, enabled) = if let Some(links) = message.properties.get("links") {
        let entry = decode(links).as_array()?.first()?.clone();
        let preview = entry.get("preview").cloned().unwrap_or(Value::Null);
        let enabled = entry.get("previewenabled").and_then(Value::as_bool) != Some(false);
        (entry, preview, enabled)
    } else {
        let list = decode(message.properties.get("urlpreviews")?);
        let value = list.as_array()?.first()?.get("value")?.clone();
        let enabled = text(&value, "thumbnail").is_some();
        let preview = json!({
            "previewurl": value.get("thumbnail"),
            "previewmeta": value.get("thumbnail_meta"),
            "title": value.get("title"),
            "description": value.get("description"),
        });
        (value, preview, enabled)
    };
    let url = html::safe_link(entry.get("url")?.as_str()?)?;
    if !enabled || url.starts_with("mailto:") {
        return None;
    }
    let title = text(&preview, "title");
    let thumbnail = text(&preview, "previewurl")
        .filter(|src| src.starts_with("https://"))
        .map(|src| {
            let declared = size(preview.get("previewmeta"));
            InlineImage {
                src,
                alt: title.clone().unwrap_or_default(),
                width: declared.map(|d| d.0),
                height: declared.map(|d| d.1),
                ..Default::default()
            }
        });
    (title.is_some() || thumbnail.is_some()).then(|| LinkPreview {
        url,
        title,
        description: text(&preview, "description"),
        thumbnail,
    })
}

fn site(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned()))
        .unwrap_or_default()
}

/// SharePoint and OneDrive links are files: the web client shows no preview for them.
fn previewable(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some_and(|h| {
                ![
                    ".sharepoint.com",
                    ".sharepoint.us",
                    ".sharepoint.cn",
                    ".sharepoint.de",
                ]
                .iter()
                .any(|s| h.ends_with(s))
            })
    })
}

/// The draft's first link and whether it is finished: a formatted link, or a typed URL followed by a space.
pub(crate) fn first_link(draft: &str, formats: &[formatting::ComposerFormat]) -> Option<(String, bool)> {
    let typed = draft
        .match_indices("http")
        .filter(|(i, _)| {
            draft[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || "([{<\"'".contains(c))
        })
        .find_map(|(i, _)| {
            let rest = &draft[i..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let candidate = rest[..end].trim_end_matches(|c: char| ".,;:!?)]}>\"'".contains(c));
            let lower = candidate.to_ascii_lowercase();
            if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                return None;
            }
            let url = html::safe_link(candidate)?;
            let start = draft[..i].encode_utf16().count() as u32;
            Some((start, url, end < rest.len()))
        });
    let formatted = formats
        .iter()
        .filter_map(|f| Some((f.start, html::safe_link(f.link.as_deref()?)?, true)))
        .filter(|(_, url, _)| !url.starts_with("mailto:"))
        .min_by_key(|(start, ..)| *start);
    [typed, formatted]
        .into_iter()
        .flatten()
        .min_by_key(|(start, ..)| *start)
        .map(|(_, url, settled)| (url, settled))
}

/// `properties.links` as the web client sends it: one HyperLink with its preview.
fn links_property(info: &UrlInfo) -> String {
    let mut preview = json!({
        "previewurl": info.thumbnail.clone().unwrap_or_default(),
        "title": info.title.clone().unwrap_or_default(),
        "description": info.description.clone().unwrap_or_default(),
    });
    if let Some(meta) = &info.thumbnail_meta {
        preview["previewmeta"] = meta.clone();
    }
    json!([{
        "@type": "http://schema.skype.com/HyperLink",
        "itemid": "0",
        "url": info.url,
        "previewenabled": true,
        "preview": preview,
    }])
    .to_string()
}

impl App {
    /// Follows the draft's first link after any change: a new link waits for its debounce, then is asked for.
    pub(crate) fn refresh_link_preview(&mut self) -> Task<Msg> {
        let key = self.draft_key();
        let found = if self.selected.is_some() && self.conv.editing().is_none() {
            let formats = self.composer.formats.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            first_link(self.conv.draft(), formats).filter(|(url, _)| previewable(url))
        } else {
            None
        };
        let state = &*self.link_preview;
        let Some((url, settled)) = found else {
            if state.draft.is_some() || state.dismissed.iter().any(|(k, _)| *k == key) {
                let state = &mut *self.link_preview;
                state.draft = None;
                state.dismissed.retain(|(k, _)| *k != key);
            }
            return Task::none();
        };
        if state.dismissed.contains(&(key.clone(), url.clone())) {
            if state.draft.is_some() {
                self.link_preview.draft = None;
            }
            return Task::none();
        }
        if state.draft.as_ref().is_some_and(|d| d.key == key && d.url == url) {
            return Task::none();
        }
        self.link_preview.draft = Some(DraftLink {
            key: key.clone(),
            url: url.clone(),
            status: Status::Waiting,
        });
        let generation = self.generation;
        let wait = if settled { SETTLED } else { TYPING };
        Task::perform(tokio::time::sleep(wait), move |()| {
            Msg::LinkPreview(LinkPreviewMsg::Due(generation, key, url))
        })
    }
    pub(crate) fn update_link_preview(&mut self, msg: LinkPreviewMsg) -> Task<Msg> {
        let current = |state: &LinkPreviewState, key: &str, url: &str| {
            state
                .draft
                .as_ref()
                .is_some_and(|d| d.key == key && d.url == url)
        };
        match msg {
            LinkPreviewMsg::Action(LinkPreviewAction::Dismiss) => {
                let state = &mut *self.link_preview;
                if let Some(draft) = state.draft.take() {
                    state.dismissed.insert((draft.key, draft.url));
                }
                Task::none()
            }
            LinkPreviewMsg::Due(generation, key, url) => {
                if generation != self.generation || !current(&self.link_preview, &key, &url) {
                    return Task::none();
                }
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if let Some(draft) = &mut self.link_preview.draft {
                    draft.status = Status::Loading;
                }
                Task::perform(
                    async move {
                        let result = client.url_preview(&url).await.map_err(|e| e.to_string());
                        (key, url, result)
                    },
                    move |(key, url, result)| {
                        Msg::LinkPreview(LinkPreviewMsg::Loaded(generation, key, url, result))
                    },
                )
            }
            LinkPreviewMsg::Loaded(generation, key, url, result) => {
                if generation != self.generation || !current(&self.link_preview, &key, &url) {
                    return Task::none();
                }
                if let Some(draft) = &mut self.link_preview.draft {
                    // A page with nothing to show gets no card, as in the web client.
                    draft.status = match result {
                        Ok(info) if info.title.is_some() || info.thumbnail.is_some() => {
                            Status::Ready(info)
                        }
                        Ok(_) | Err(_) => Status::Failed,
                    };
                }
                Task::none()
            }
        }
    }
    fn ready_link(&self) -> Option<&UrlInfo> {
        let draft = self.link_preview.draft.as_ref()?;
        match &draft.status {
            Status::Ready(info) if draft.key == self.draft_key() => Some(info),
            _ => None,
        }
    }
    /// `properties.links` for the message being sent, when its preview is ready.
    pub(crate) fn draft_links(&self) -> Option<String> {
        self.ready_link().map(links_property)
    }
    /// The preview image to download for the compose box.
    pub(crate) fn draft_link_image(&self) -> Option<String> {
        self.ready_link()?.thumbnail.clone()
    }
    pub(crate) fn composer_link_preview(&self) -> Option<LinkPreviewView> {
        let draft = self.link_preview.draft.as_ref()?;
        if draft.key != self.draft_key() {
            return None;
        }
        match &draft.status {
            Status::Waiting | Status::Failed => None,
            Status::Loading => Some(LinkPreviewView {
                site: site(&draft.url),
                url: draft.url.clone(),
                title: None,
                description: None,
                image: None,
                loading: true,
            }),
            Status::Ready(info) => {
                let size = |k: &str| {
                    info.thumbnail_meta
                        .as_ref()?
                        .get(k)?
                        .as_u64()
                        .map(|n| n as u32)
                };
                Some(LinkPreviewView {
                    site: site(&info.url),
                    url: info.url.clone(),
                    title: info.title.clone(),
                    description: info.description.clone(),
                    image: info.thumbnail.as_ref().map(|src| {
                        self.image_view(&InlineImage {
                            src: src.clone(),
                            width: size("width"),
                            height: size("height"),
                            ..Default::default()
                        })
                    }),
                    loading: false,
                })
            }
        }
    }
    pub(crate) fn link_preview_view(&self, preview: &LinkPreview) -> LinkPreviewView {
        LinkPreviewView {
            site: site(&preview.url),
            url: preview.url.clone(),
            title: preview.title.clone(),
            description: preview.description.clone(),
            image: preview.thumbnail.as_ref().map(|i| self.image_view(i)),
            loading: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(key: &str, value: Value) -> Message {
        let mut m = Message::default();
        m.properties.insert(key.into(), value);
        m
    }
    #[test]
    fn links_are_read_from_a_json_string_with_their_preview() {
        let m = message(
            "links",
            json!(json!([{
                "@type": "http://schema.skype.com/HyperLink", "itemid": "0",
                "url": "https://www.rust-lang.org/", "previewenabled": true,
                "preview": {"previewurl": "https://urlp.asm.skype.com/v1/url/content?url=x",
                            "previewmeta": {"width": 1200, "height": 630},
                            "title": "Rust", "description": "A language &amp; more"}
            }]).to_string()),
        );
        let preview = from_message(&m).unwrap();
        assert_eq!(preview.url, "https://www.rust-lang.org/");
        assert_eq!(preview.title.as_deref(), Some("Rust"));
        assert_eq!(preview.description.as_deref(), Some("A language & more"));
        let image = preview.thumbnail.unwrap();
        assert_eq!((image.width, image.height), (Some(1200), Some(630)));
    }
    #[test]
    fn disabled_empty_or_unsafe_links_show_nothing() {
        let link = |enabled: bool, url: &str, title: &str| {
            message(
                "links",
                json!([{"url": url, "previewenabled": enabled, "preview": {"title": title}}]),
            )
        };
        assert!(from_message(&link(true, "https://a.test/", "A")).is_some());
        assert!(from_message(&link(false, "https://a.test/", "A")).is_none());
        assert!(from_message(&link(true, "https://a.test/", "")).is_none());
        assert!(from_message(&link(true, "javascript:alert(1)", "A")).is_none());
    }
    #[test]
    fn url_previews_are_read_when_links_are_absent() {
        let m = message(
            "urlpreviews",
            json!([{"value": {"url": "https://a.test/", "title": "A", "thumbnail": "https://t.test/a.png"}}]),
        );
        let preview = from_message(&m).unwrap();
        assert_eq!(preview.thumbnail.unwrap().src, "https://t.test/a.png");
    }
    #[test]
    fn the_first_link_is_typed_or_formatted_whichever_comes_first() {
        assert_eq!(first_link("voir https://a.test/x. merci", &[]), Some(("https://a.test/x".into(), true)));
        assert_eq!(first_link("voir https://a.test", &[]), Some(("https://a.test/".into(), false)));
        assert_eq!(first_link("rien ici, ni xhttps://a.test", &[]), None);
        let format = formatting::ComposerFormat {
            start: 0,
            length: 3,
            link: Some("https://b.test/".into()),
            ..Default::default()
        };
        assert_eq!(
            first_link("ici puis https://a.test ", std::slice::from_ref(&format)),
            Some(("https://b.test/".into(), true))
        );
    }
    #[test]
    fn sent_links_follow_the_web_client() {
        let info = UrlInfo {
            url: "https://a.test/".into(),
            title: Some("A".into()),
            thumbnail: Some("https://t.test/a.png".into()),
            thumbnail_meta: Some(json!({"width": 2, "height": 1})),
            ..Default::default()
        };
        let sent: Value = serde_json::from_str(&links_property(&info)).unwrap();
        assert_eq!(
            sent,
            json!([{"@type": "http://schema.skype.com/HyperLink", "itemid": "0", "url": "https://a.test/",
                "previewenabled": true, "preview": {"previewurl": "https://t.test/a.png",
                "previewmeta": {"width": 2, "height": 1}, "title": "A", "description": ""}}])
        );
        // And read back as a preview.
        let preview = from_message(&message("links", json!(sent.to_string()))).unwrap();
        assert_eq!(preview.title.as_deref(), Some("A"));
    }
    /// Runs the network work of `task`, and of what its answers start, to the end.
    async fn run(app: &mut App, task: Task<Msg>) {
        let mut jobs = task.0;
        while let Some(job) = jobs.pop() {
            if let crate::task::Job::Future(future) = job
                && let Ok(msg) = tokio::time::timeout(Duration::from_secs(15), future).await
            {
                jobs.extend(app.update(msg).0);
            }
        }
    }
    async fn send(app: &mut App, msg: Msg) {
        let task = app.update(msg);
        run(app, task).await;
    }
    fn last_message_link(app: &App) -> Option<LinkPreviewView> {
        app.message_view(app.conv.messages().last().unwrap()).link_preview
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn the_demo_shows_received_previews_and_sends_the_draft_s_one() {
        let (mut app, task) = App::boot(true);
        run(&mut app, task).await;
        let chat = "19:alice_me@unq.gbl.spaces".to_owned();
        send(&mut app, Msg::Select(Selection::Chat(chat))).await;
        let received = app
            .conv
            .messages()
            .iter()
            .find_map(|m| app.message_view(m).link_preview)
            .expect("the seeded preview");
        assert_eq!(received.title.as_deref(), Some("Rust Programming Language"));
        assert_eq!(received.site, "rust-lang.org");
        let image = received.image.expect("its picture");
        assert!(matches!(image.status, view::ImageStatus::Ready { .. }), "{image:?}");

        send(&mut app, Msg::DraftChanged("regarde https://example.org/doc merci".into())).await;
        let preview = app.composer_view().link_preview.expect("the draft's preview");
        assert!(!preview.loading);
        assert_eq!(preview.url, "https://example.org/doc");
        assert_eq!(preview.title.as_deref(), Some("example.org — page de démonstration"));
        let image = preview.image.expect("its picture");
        assert!(matches!(image.status, view::ImageStatus::Ready { .. }), "{image:?}");
        send(&mut app, Msg::Send).await;
        assert_eq!(app.error.clone(), None);
        assert!(app.composer_view().link_preview.is_none(), "the draft is empty");
        let sent = last_message_link(&app).expect("sent with its preview");
        assert_eq!(sent.url, "https://example.org/doc");

        // A closed preview stays closed for that draft, and the message goes without it.
        send(&mut app, Msg::DraftChanged("https://example.org/autre ".into())).await;
        assert!(app.composer_view().link_preview.is_some());
        send(&mut app, Msg::LinkPreview(LinkPreviewMsg::Action(LinkPreviewAction::Dismiss))).await;
        send(&mut app, Msg::DraftChanged("https://example.org/autre ok".into())).await;
        assert!(app.composer_view().link_preview.is_none());
        send(&mut app, Msg::Send).await;
        let last = app.conv.messages().last().unwrap();
        assert!(last.content.contains("autre ok"), "{}", last.content);
        assert!(last_message_link(&app).is_none());
    }
    #[test]
    fn file_links_get_no_preview() {
        assert!(!previewable("https://contoso.sharepoint.com/:w:/r/doc"));
        assert!(previewable("https://www.rust-lang.org/"));
    }
}
