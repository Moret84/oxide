//! The GIF and sticker picker: Giphy search, the downloaded sticker catalog and the messages that send them.
use super::*;
use crate::stickers::{self, SIZE as STICKER_SIZE, Sticker};
use teams_i18n::{t, tf};

/// Height GIFs are capped at in a message, keeping their ratio.
const GIF_MAX_HEIGHT: u32 = 250;
const GIPHY: &str = "http://schema.skype.com/Giphy";
const STICKER: &str = "http://schema.skype.com/Sticker";

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct GifView {
    pub url: String,
    pub preview: String,
    pub width: u32,
    pub height: u32,
    pub title: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct GifsView {
    pub query: String,
    pub results: Vec<GifView>,
    pub loading: bool,
    pub error: Option<String>,
    /// A GIF or sticker is going out.
    pub sending: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StickerView {
    pub id: String,
    pub name: String,
    pub keywords: String,
    pub url: String,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StickerCategory {
    pub id: String,
    pub stickers: Vec<StickerView>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum GifsAction {
    /// Popular GIFs for an empty query.
    Search {
        query: String,
    },
    SendGif {
        url: String,
    },
    SendSticker {
        id: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum GifsMsg {
    Action(GifsAction),
    Found(u64, String, Result<Vec<teams_chat::Gif>, String>),
    Sent(u64, Selection, Result<(), String>),
}

#[derive(Default)]
pub(crate) struct GifsState {
    pub view: GifsView,
    results: Vec<teams_chat::Gif>,
}

/// The sticker catalog, by category, in the app's language; empty until it is downloaded.
pub fn sticker_categories() -> Vec<StickerCategory> {
    let mut categories: Vec<StickerCategory> = Vec::new();
    for sticker in stickers::current().iter() {
        let view = StickerView {
            id: sticker.id.clone(),
            name: sticker.name.clone(),
            keywords: sticker.keywords.clone(),
            url: sticker.url.clone(),
        };
        match categories.iter_mut().find(|c| c.id == sticker.category) {
            Some(category) => category.stickers.push(view),
            None => categories.push(StickerCategory {
                id: sticker.category.clone(),
                stickers: vec![view],
            }),
        }
    }
    categories
}
/// The web client's GIF message: the Giphy URL in an `<img>` inside a read-only block.
fn gif_html(gif: &teams_chat::Gif) -> String {
    let (mut width, mut height) = (gif.width, gif.height);
    if height > GIF_MAX_HEIGHT {
        width = width * GIF_MAX_HEIGHT / height;
        height = GIF_MAX_HEIGHT;
    }
    let title = if gif.title.is_empty() {
        t("core.gifs.gif").to_owned()
    } else {
        gif.title.clone()
    };
    let label = escape_attr(&tf!("core.gifs.gif_image", title = title));
    let size = if width > 0 && height > 0 {
        format!(" width=\"{width}\" height=\"{height}\"")
    } else {
        String::new()
    };
    format!(
        "<p><readonly title=\"{label}\" aria-label=\"{label}\" contenteditable=\"false\" itemtype=\"{GIPHY}\"><img src=\"{}\"{size} style=\"margin-top: 4px; max-width: 100%; height: auto;\" alt=\"{label}\"></readonly></p>",
        escape_attr(&gif.url)
    )
}
fn sticker_html(sticker: &Sticker) -> String {
    let caption = escape_attr(&sticker.name);
    format!(
        "<p><readonly aria-label=\"{caption}\" contenteditable=\"false\" itemtype=\"{STICKER}\" title=\"{caption}\"><img src=\"{}\" height=\"{STICKER_SIZE}\" width=\"{STICKER_SIZE}\" alt=\"{caption}\"><span title=\"{caption}\"></span></readonly></p>",
        escape_attr(&sticker.url)
    )
}

impl App {
    pub(crate) fn update_gifs(&mut self, msg: GifsMsg) -> Task<Msg> {
        let generation = self.generation;
        let task = match msg {
            GifsMsg::Found(g, ..) | GifsMsg::Sent(g, ..) if g != generation => Task::none(),
            GifsMsg::Action(GifsAction::Search { query }) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                let query = query.trim().to_owned();
                let gifs = &mut *self.gifs;
                gifs.view.query = query.clone();
                gifs.view.loading = true;
                gifs.view.error = None;
                let lang = crate::emoji::locale();
                // `giphyRatingType` defaults to Moderate in the messaging policy.
                let rating = teams_chat::giphy_rating(Some("Moderate"));
                Task::perform(
                    async move {
                        let r = client
                            .search_gifs(&query, rating, lang)
                            .await
                            .map_err(|e| e.to_string());
                        (query, r)
                    },
                    move |(q, r)| Msg::Gifs(GifsMsg::Found(generation, q, r)),
                )
            }
            GifsMsg::Found(_, query, result) => {
                let gifs = &mut *self.gifs;
                if gifs.view.query != query {
                    return Task::none();
                }
                gifs.view.loading = false;
                match result {
                    Ok(found) => gifs.results = found,
                    Err(e) => gifs.view.error = Some(tf!("core.gifs.search_failed", error = e)),
                }
                Task::none()
            }
            GifsMsg::Action(GifsAction::SendGif { url }) => {
                let Some(gif) = self.gifs.results.iter().find(|g| g.url == url).cloned() else {
                    return Task::none();
                };
                self.send_fun(gif_html(&gif))
            }
            GifsMsg::Action(GifsAction::SendSticker { id }) => {
                match stickers::current().iter().find(|s| s.id == id) {
                    Some(sticker) => self.send_fun(sticker_html(sticker)),
                    None => Task::none(),
                }
            }
            GifsMsg::Sent(_, selection, result) => {
                self.gifs.view.sending = false;
                match result {
                    Ok(()) => {
                        if self.selected.as_ref() == Some(&selection) {
                            return Task::batch([self.load_feed(false), self.load_workspace()]);
                        }
                        return self.load_workspace();
                    }
                    Err(e) => {
                        *self.error = Some(tf!("core.gifs.send_failed", error = e));
                    }
                }
                Task::none()
            }
        };
        self.refresh_gifs_view();
        task
    }
    /// Sends a GIF or sticker to the open conversation, in the open thread if any.
    fn send_fun(&mut self, html: String) -> Task<Msg> {
        if self.read_only() || self.gifs.view.sending {
            return Task::none();
        }
        let (Some(client), Some(selection)) = (self.client.clone(), self.selected.clone()) else {
            return Task::none();
        };
        self.gifs.view.sending = true;
        let reply = self.conv.reply().cloned();
        let message = OutgoingMessage::html(client_message_id(), html, &*self.display_name);
        let generation = self.generation;
        Task::perform(
            async move {
                let r = client
                    .send_message(selection.id(), reply.as_deref(), &message)
                    .await
                    .map(drop)
                    .map_err(|e| e.to_string());
                (selection, r)
            },
            move |(s, r)| Msg::Gifs(GifsMsg::Sent(generation, s, r)),
        )
    }
    fn refresh_gifs_view(&mut self) {
        let gifs = &mut *self.gifs;
        let results: Vec<GifView> = gifs
            .results
            .iter()
            .map(|g| GifView {
                url: g.url.clone(),
                preview: g.preview.clone(),
                width: g.width,
                height: g.height,
                title: g.title.clone(),
            })
            .collect();
        if gifs.view.results != results {
            gifs.view.results = results;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gif_messages_follow_the_web_client_markup() {
        let html = gif_html(&teams_chat::Gif {
            url: "https://media.giphy.com/media/a/giphy.gif".into(),
            preview: String::new(),
            width: 480,
            height: 500,
            title: "Cat \"typing\"".into(),
        });
        assert!(html.contains(r#"itemtype="http://schema.skype.com/Giphy""#));
        assert!(html.contains(r#"width="240" height="250""#));
        assert!(html.contains("Cat &quot;typing&quot;"));
    }
    #[test]
    fn stickers_go_out_in_a_read_only_block() {
        let html = sticker_html(&Sticker {
            id: "a".into(),
            category: "BMac".into(),
            name: "J’aime \"ça\"".into(),
            keywords: String::new(),
            url: "https://statics.teams.cdn.office.net/x.png?v=5".into(),
        });
        assert!(html.contains("schema.skype.com/Sticker"));
        assert!(html.contains(r#"height="250" width="250""#));
        assert!(html.contains("J’aime &quot;ça&quot;"));
    }
}
