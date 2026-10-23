//! Bot and connector cards reduced to what the app draws safely; only `http(s)`/`mailto` links survive.
use crate::html::{self, InlineImage, Run};
use base64::Engine;
use scraper::ElementRef;
use serde_json::Value;
use teams_chat::Message;
use teams_i18n::{t, tf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub items: Vec<CardItem>,
    pub actions: Vec<CardAction>,
    /// A connector card's `themeColor` as `0xRRGGBB`, drawn as a bar along its top edge.
    pub accent: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardItem {
    Text {
        runs: Vec<Run>,
        muted: bool,
    },
    /// HTML text with structure (lists, rules, paragraphs) that runs alone would lose.
    Blocks(Vec<html::Block>),
    /// Extra space above the next element, in points, and whether a line separates them.
    Gap {
        points: u32,
        separator: bool,
    },
    Facts(Vec<Fact>),
    Image(InlineImage),
    Input(CardInput),
    /// A `ColumnSet` of two or more visible columns, side by side.
    Columns(Vec<CardColumn>),
    /// Items on the shaded background of a `Container`, table row or cell with `"style": "emphasis"`.
    Emphasis(Vec<CardItem>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardColumn {
    pub width: ColumnWidth,
    pub items: Vec<CardItem>,
}
/// A column's `width`: `auto` fits its content, `stretch` and weights share what is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnWidth {
    Auto,
    Stretch,
    Weight(u32),
    Points(u32),
}
/// An `Input.*` element: its id names its value in what a button sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardInput {
    pub id: String,
    pub label: Option<String>,
    pub kind: InputKind,
    /// The initial value, as the card states it.
    pub value: String,
    pub required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    Text { placeholder: String, multiline: bool },
    Number { placeholder: String },
    Date,
    Time,
    /// `valueOn`/`valueOff` are what is sent.
    Toggle { title: String, on: String, off: String },
    Choice { choices: Vec<(String, String)>, multi: bool },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub title: String,
    pub value: String,
}
/// A button. `kind` says what it does; `action` keeps the card's JSON for the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardAction {
    pub title: String,
    pub url: Option<String>,
    pub kind: ActionKind,
    pub action: Value,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    OpenUrl,
    /// `Action.Submit` (and `messageBack`, `imBack`, `invoke` in its `msteams` data).
    Submit,
    /// `Action.Execute`, Universal Actions.
    Execute,
    /// Shown, but only Teams can run it (task modules, sign-in flows, unknown types).
    Unsupported,
}
/// Bounds for hostile or huge payloads.
const MAX_ITEMS: usize = 200;
const MAX_DEPTH: usize = 12;

impl Card {
    /// The card as plain runs, for previews and search.
    pub fn text_runs(&self) -> Vec<Run> {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.title.clone());
        parts.extend(self.subtitle.clone());
        item_texts(&self.items, &mut parts);
        if parts.is_empty() {
            parts.push(t("core.cards.card").into());
        }
        vec![Run {
            text: parts.join("\n"),
            ..Default::default()
        }]
    }
    pub fn images(&self) -> impl Iterator<Item = &InlineImage> {
        let mut images = Vec::new();
        walk(&self.items, &mut |item| {
            if let CardItem::Image(image) = item {
                images.push(image);
            }
        });
        images.into_iter()
    }
    pub fn inputs(&self) -> impl Iterator<Item = &CardInput> {
        let mut inputs = Vec::new();
        walk(&self.items, &mut |item| {
            if let CardItem::Input(input) = item {
                inputs.push(input);
            }
        });
        inputs.into_iter()
    }
}
/// Every item, columns included, in reading order.
fn walk<'a>(items: &'a [CardItem], visit: &mut impl FnMut(&'a CardItem)) {
    for item in items {
        visit(item);
        match item {
            CardItem::Columns(columns) => {
                for column in columns {
                    walk(&column.items, visit);
                }
            }
            CardItem::Emphasis(items) => walk(items, visit),
            _ => {}
        }
    }
}
fn item_texts(items: &[CardItem], parts: &mut Vec<String>) {
    for item in items {
        match item {
            CardItem::Text { runs, .. } => {
                parts.push(runs.iter().map(|r| r.text.as_str()).collect())
            }
            CardItem::Blocks(blocks) => {
                parts.push(html::Document::new(blocks.clone(), vec![]).text())
            }
            CardItem::Facts(facts) => parts.extend(
                facts
                    .iter()
                    .map(|f| tf!("core.cards.text", title = f.title, value = f.value)),
            ),
            CardItem::Input(input) => parts.extend(input.label.clone()),
            // A row of columns reads as one line.
            CardItem::Columns(columns) => {
                let mut row = Vec::new();
                for column in columns {
                    item_texts(&column.items, &mut row);
                }
                if !row.is_empty() {
                    parts.push(row.join(" "));
                }
            }
            CardItem::Emphasis(items) => item_texts(items, parts),
            CardItem::Image(_) | CardItem::Gap { .. } => {}
        }
    }
}
/// Cards attached through `properties.cards` (a JSON string, sometimes encoded twice).
pub fn from_message(message: &Message) -> Vec<Card> {
    let Some(raw) = message.properties.get("cards") else {
        return vec![];
    };
    let mut value = raw.clone();
    for _ in 0..2 {
        if let Some(s) = value.as_str() {
            value = serde_json::from_str(s).unwrap_or(Value::Null);
        }
    }
    let list = match value {
        Value::Array(list) => list,
        Value::Object(_) => vec![value],
        _ => vec![],
    };
    list.iter().filter_map(attachment).collect()
}
/// `<URIObject type="SWIFT.1"><Swift b64="…"/></URIObject>`: base64 JSON with attachments.
pub fn from_uri_object(element: ElementRef<'_>) -> Vec<Card> {
    if !element
        .value()
        .attr("type")
        .is_some_and(|t| t.to_ascii_uppercase().starts_with("SWIFT"))
    {
        return vec![];
    }
    let Some(b64) = element
        .descendants()
        .filter_map(ElementRef::wrap)
        .find_map(|el| el.value().attr("b64"))
    else {
        return vec![];
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return vec![];
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return vec![];
    };
    value
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(attachment)
        .collect()
}
/// One `{contentType, content}` attachment, or a bare Adaptive Card.
fn attachment(value: &Value) -> Option<Card> {
    let content_type = str_at(value, "contentType").unwrap_or_default();
    let mut content = value
        .get("content")
        .cloned()
        .unwrap_or_else(|| value.clone());
    if let Some(s) = content.as_str() {
        content = serde_json::from_str(s).unwrap_or(Value::Null);
    }
    let kind = content_type.to_ascii_lowercase();
    let card = if kind.contains("card.adaptive") || str_at(&content, "type") == Some("AdaptiveCard")
    {
        adaptive(&content)
    } else if kind.contains("card.hero") || kind.contains("card.thumbnail") {
        hero(&content)
    } else if kind.contains("o365connector") || content.get("sections").is_some() {
        connector(&content)
    } else if kind.is_empty() && content.is_null() {
        return None;
    } else {
        Card::default()
    };
    Some(with_fallback(card, value, &content))
}
fn with_fallback(mut card: Card, value: &Value, content: &Value) -> Card {
    if card.title.is_none() && card.items.is_empty() {
        let text = ["fallbackText", "summary", "speak"]
            .iter()
            .find_map(|k| str_at(content, k).or_else(|| str_at(value, k)))
            .unwrap_or(t("core.cards.card_not_supported_by_the_app"));
        card.items.push(CardItem::Text {
            runs: markdown(text, false, 0),
            muted: true,
        });
    }
    card
}
fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn adaptive(card: &Value) -> Card {
    let mut out = Card::default();
    elements(card.get("body"), &mut out, 0);
    actions(card.get("actions"), &mut out.actions);
    show_cards(card.get("actions"), &mut out, 0);
    out
}
/// An Adaptive Card returned by `Action.Execute`, which replaces the one shown.
pub fn adaptive_card(content: &Value) -> Option<Card> {
    (str_at(content, "type") == Some("AdaptiveCard")).then(|| adaptive(content))
}
/// `Action.ShowCard` cards are drawn after the buttons, under their title, with their own inputs and buttons.
fn show_cards(list: Option<&Value>, out: &mut Card, depth: usize) {
    if depth > 2 {
        return;
    }
    for action in list.and_then(Value::as_array).into_iter().flatten().take(12) {
        if !str_at(action, "type").is_some_and(|t| t.eq_ignore_ascii_case("Action.ShowCard")) {
            continue;
        }
        let Some(card) = action.get("card") else {
            continue;
        };
        out.items.push(CardItem::Gap {
            points: 8,
            separator: true,
        });
        if let Some(title) = str_at(action, "title") {
            out.items.push(CardItem::Text {
                runs: markdown(title, true, 0),
                muted: false,
            });
        }
        elements(card.get("body"), out, depth + 1);
        actions(card.get("actions"), &mut out.actions);
        show_cards(card.get("actions"), out, depth + 1);
    }
}
fn input(element: &Value, kind: &str) -> Option<CardInput> {
    let id = str_at(element, "id")?.to_owned();
    let placeholder = str_at(element, "placeholder").unwrap_or_default().to_owned();
    let value = match element.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    };
    let kind = match kind {
        "Input.Text" => InputKind::Text {
            placeholder,
            multiline: element.get("isMultiline") == Some(&Value::Bool(true)),
        },
        "Input.Number" => InputKind::Number { placeholder },
        "Input.Date" => InputKind::Date,
        "Input.Time" => InputKind::Time,
        "Input.Toggle" => InputKind::Toggle {
            title: str_at(element, "title").unwrap_or_default().to_owned(),
            on: str_at(element, "valueOn").unwrap_or("true").to_owned(),
            off: str_at(element, "valueOff").unwrap_or("false").to_owned(),
        },
        "Input.ChoiceSet" => InputKind::Choice {
            choices: element
                .get("choices")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(100)
                .filter_map(|c| {
                    let value = match c.get("value")? {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    Some((str_at(c, "title").unwrap_or(&value).to_owned(), value))
                })
                .collect(),
            multi: element.get("isMultiSelect") == Some(&Value::Bool(true)),
        },
        _ => return None,
    };
    Some(CardInput {
        id,
        label: str_at(element, "label").map(plain_markdown),
        kind,
        value,
        required: element.get("isRequired") == Some(&Value::Bool(true)),
    })
}
/// Adaptive Card `spacing` and `separator`, beyond the default gap the app already leaves.
fn gap(element: &Value, out: &mut Card) {
    if out.items.is_empty() {
        return;
    }
    let points = match str_at(element, "spacing")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("medium") | Some("padding") => 8,
        Some("large") => 16,
        Some("extralarge") => 24,
        _ => 0,
    };
    let separator = element.get("separator") == Some(&Value::Bool(true));
    if points > 0 || separator {
        out.items.push(CardItem::Gap { points, separator });
    }
}
fn elements(list: Option<&Value>, out: &mut Card, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for element in list.and_then(Value::as_array).into_iter().flatten() {
        if out.items.len() >= MAX_ITEMS {
            return;
        }
        if element.get("isVisible") == Some(&Value::Bool(false)) {
            continue;
        }
        gap(element, out);
        match str_at(element, "type").unwrap_or_default() {
            "TextBlock" => {
                if let Some(text) = str_at(element, "text") {
                    let bold =
                        str_at(element, "weight").is_some_and(|w| w.eq_ignore_ascii_case("bolder"));
                    let size = match str_at(element, "size").map(str::to_ascii_lowercase) {
                        Some(s) if s == "small" => 12,
                        Some(s) if s == "medium" => 15,
                        Some(s) if s == "large" => 18,
                        Some(s) if s == "extralarge" => 22,
                        _ => 0,
                    };
                    let muted = element.get("isSubtle") == Some(&Value::Bool(true));
                    out.items.push(markdown_item(text, bold, size, muted));
                }
            }
            "RichTextBlock" => {
                let mut runs = Vec::new();
                for inline in element
                    .get("inlines")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let (text, style) = match inline {
                        Value::String(s) => (s.as_str(), &Value::Null),
                        other => (str_at(other, "text").unwrap_or_default(), other),
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let flag = |k: &str| style.get(k) == Some(&Value::Bool(true));
                    runs.push(Run {
                        text: text.to_owned(),
                        bold: str_at(style, "weight")
                            .is_some_and(|w| w.eq_ignore_ascii_case("bolder")),
                        italic: flag("italic"),
                        strike: flag("strikethrough"),
                        underline: flag("underline"),
                        code: str_at(style, "fontType")
                            .is_some_and(|f| f.eq_ignore_ascii_case("monospace")),
                        link: style
                            .pointer("/selectAction/url")
                            .and_then(Value::as_str)
                            .and_then(html::safe_link),
                        ..Default::default()
                    });
                }
                if !runs.is_empty() {
                    out.items.push(CardItem::Text { runs, muted: false });
                }
            }
            "FactSet" => {
                let facts: Vec<_> = element
                    .get("facts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|f| Fact {
                        title: str_at(f, "title").unwrap_or_default().to_owned(),
                        value: plain_markdown(str_at(f, "value").unwrap_or_default()),
                    })
                    .collect();
                if !facts.is_empty() {
                    out.items.push(CardItem::Facts(facts));
                }
            }
            "Image" => image(element, "url", None, out),
            "ImageSet" => {
                // An ImageSet's images are medium unless it says otherwise.
                let size = str_at(element, "imageSize").or(Some("medium"));
                for item in element
                    .get("images")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    image(item, "url", size, out);
                }
            }
            "Container" => emphasized(element, out, |out| {
                elements(element.get("items"), out, depth + 1)
            }),
            "ColumnSet" => column_set(element, out, depth),
            "Table" => {
                for row in element
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    emphasized(row, out, |out| {
                        for cell in row
                            .get("cells")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            emphasized(cell, out, |out| {
                                elements(cell.get("items"), out, depth + 1)
                            });
                        }
                    });
                }
            }
            "ActionSet" => actions(element.get("actions"), &mut out.actions),
            kind if kind.starts_with("Input.") => {
                if let Some(input) = input(element, kind) {
                    out.items.push(CardItem::Input(input));
                }
            }
            _ => {
                // Unknown elements may name a simpler replacement.
                if let Some(fallback) = element.get("fallback").filter(|f| f.is_object()) {
                    elements(Some(&Value::Array(vec![fallback.clone()])), out, depth + 1);
                }
            }
        }
    }
}
/// Columns without `isVisible: false`; one alone is drawn as plain items, as before.
/// What `fill` adds, boxed on the emphasis background when `element` has `"style": "emphasis"`.
fn emphasized(element: &Value, out: &mut Card, fill: impl FnOnce(&mut Card)) {
    if !str_at(element, "style").is_some_and(|s| s.eq_ignore_ascii_case("emphasis")) {
        return fill(out);
    }
    let mut inner = Card::default();
    fill(&mut inner);
    out.actions.append(&mut inner.actions);
    if !inner.items.is_empty() {
        out.items.push(CardItem::Emphasis(inner.items));
    }
}
fn column_set(element: &Value, out: &mut Card, depth: usize) {
    let mut columns = Vec::new();
    for column in element
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("isVisible") != Some(&Value::Bool(false)))
    {
        let mut inner = Card::default();
        elements(column.get("items"), &mut inner, depth + 1);
        out.actions.append(&mut inner.actions);
        if !inner.items.is_empty() {
            columns.push(CardColumn {
                width: column_width(column.get("width")),
                items: inner.items,
            });
        }
    }
    match columns.len() {
        0 => {}
        1 => out.items.append(&mut columns.remove(0).items),
        _ => out.items.push(CardItem::Columns(columns)),
    }
}
/// `"auto"`, `"stretch"` (the default), a weight such as `2` or `"2"`, or `"50px"`.
fn column_width(value: Option<&Value>) -> ColumnWidth {
    match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|&n| n > 0)
            .map_or(ColumnWidth::Stretch, |n| ColumnWidth::Weight(n.min(100) as u32)),
        Some(Value::String(s)) => {
            let s = s.trim().to_ascii_lowercase();
            if s == "auto" {
                ColumnWidth::Auto
            } else if let Some(px) = s.strip_suffix("px").and_then(|n| n.trim().parse::<u32>().ok())
            {
                ColumnWidth::Points(px.min(600))
            } else if let Some(n) = s.parse::<u32>().ok().filter(|&n| n > 0) {
                ColumnWidth::Weight(n.min(100))
            } else {
                ColumnWidth::Stretch
            }
        }
        _ => ColumnWidth::Stretch,
    }
}
/// `"40px"` as points.
fn pixels(value: &Value, key: &str) -> Option<u32> {
    str_at(value, key)?
        .to_ascii_lowercase()
        .strip_suffix("px")?
        .trim()
        .parse()
        .ok()
        .filter(|&n| n > 0)
}
/// `default_size` is the enclosing ImageSet's `imageSize`.
fn image(value: &Value, key: &str, default_size: Option<&str>, out: &mut Card) {
    let Some(src) =
        str_at(value, key).filter(|s| s.starts_with("https://") || s.starts_with("data:image/"))
    else {
        return;
    };
    // The host config's sizes: an avatar asks for `small`, not a full-width box.
    let side = match str_at(value, "size")
        .or(default_size)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("small") => Some(40),
        Some("medium") => Some(80),
        Some("large") => Some(160),
        _ => None,
    };
    let width = pixels(value, "width").or(side);
    let height = pixels(value, "height").or(side).or(width);
    out.items.push(CardItem::Image(InlineImage {
        src: src.to_owned(),
        alt: str_at(value, "altText")
            .or_else(|| str_at(value, "alt"))
            .unwrap_or(t("core.cards.card_image"))
            .to_owned(),
        width: width.map(|w| w.min(600)),
        height: height.map(|h| h.min(600)),
    }));
}
fn actions(list: Option<&Value>, out: &mut Vec<CardAction>) {
    for action in list
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(12)
    {
        let title = str_at(action, "title")
            .or_else(|| str_at(action, "name"))
            .unwrap_or("Action");
        let kind = str_at(action, "type")
            .or_else(|| str_at(action, "@type"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let url = match kind.as_str() {
            "action.openurl" => str_at(action, "url"),
            "openurl" | "signin" => str_at(action, "value"),
            "openuri" => action.pointer("/targets/0/uri").and_then(Value::as_str),
            _ => None,
        }
        .and_then(html::safe_link);
        // Revealed in place by `show_cards`, not a button of its own.
        if kind == "action.showcard" {
            continue;
        }
        let msteams = action
            .pointer("/data/msteams")
            .or_else(|| action.pointer("/data/msTeams"))
            .and_then(|m| str_at(m, "type"))
            .map(str::to_ascii_lowercase);
        let kind = if url.is_some() {
            ActionKind::OpenUrl
        } else {
            match kind.as_str() {
                "action.execute" => ActionKind::Execute,
                "action.submit" => match msteams.as_deref() {
                    None | Some("messageback" | "imback" | "invoke") => ActionKind::Submit,
                    Some(_) => ActionKind::Unsupported,
                },
                "imback" | "messageback" | "invoke" => ActionKind::Submit,
                _ => ActionKind::Unsupported,
            }
        };
        out.push(CardAction {
            title: title.to_owned(),
            url,
            kind,
            action: action.clone(),
        });
    }
}
fn hero(card: &Value) -> Card {
    let mut out = Card {
        title: str_at(card, "title").map(plain_text),
        subtitle: str_at(card, "subtitle").map(plain_text),
        ..Default::default()
    };
    for image in card
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        self::image(image, "url", None, &mut out);
    }
    out.items
        .extend(str_at(card, "text").and_then(|t| rich_text(t, false, false)));
    actions(card.get("buttons"), &mut out.actions);
    out
}
fn connector(card: &Value) -> Card {
    let mut out = Card {
        title: str_at(card, "title").map(plain_text),
        accent: str_at(card, "themeColor").and_then(theme_color),
        ..Default::default()
    };
    out.items
        .extend(str_at(card, "text").and_then(|t| rich_text(t, false, false)));
    for section in card
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(20)
    {
        // Teams' order: title, the activity (its image beside its texts), text, facts, images, buttons.
        out.items
            .extend(str_at(section, "title").and_then(|t| rich_text(t, true, false)));
        let mut activity = Card::default();
        for (key, bold) in [
            ("activityTitle", true),
            ("activitySubtitle", false),
            ("activityText", false),
        ] {
            activity
                .items
                .extend(str_at(section, key).and_then(|t| rich_text(t, bold, false)));
        }
        let mut icon = Card::default();
        fixed_image(section, "activityImage", 52, &mut icon);
        if icon.items.is_empty() || activity.items.is_empty() {
            out.items.append(&mut icon.items);
            out.items.append(&mut activity.items);
        } else {
            out.items.push(CardItem::Columns(vec![
                CardColumn {
                    width: ColumnWidth::Points(52),
                    items: icon.items,
                },
                CardColumn {
                    width: ColumnWidth::Stretch,
                    items: activity.items,
                },
            ]));
        }
        out.items
            .extend(str_at(section, "text").and_then(|t| rich_text(t, false, false)));
        let facts: Vec<_> = section
            .get("facts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|f| Fact {
                title: str_at(f, "name").unwrap_or_default().to_owned(),
                value: plain_text(str_at(f, "value").unwrap_or_default()),
            })
            .collect();
        if !facts.is_empty() {
            out.items.push(CardItem::Facts(facts));
        }
        for image in section
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(20)
        {
            fixed_image(image, "image", 160, &mut out);
        }
        actions(section.get("potentialAction"), &mut out.actions);
    }
    actions(card.get("potentialAction"), &mut out.actions);
    out
}
/// A connector card's square image, at the size Teams draws it whatever the picture's own.
fn fixed_image(value: &Value, key: &str, side: u32, out: &mut Card) {
    let before = out.items.len();
    image(value, key, None, out);
    if out.items.len() > before
        && let Some(CardItem::Image(image)) = out.items.last_mut()
    {
        image.width = Some(side);
        image.height = Some(side);
    }
}
/// The first 6- or 3-digit hex colour in `themeColor` (`"FF0000"`, `"#0076D7"`, `"f00"`), as Teams reads it.
fn theme_color(text: &str) -> Option<u32> {
    let word = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| (w.len() == 6 || w.len() == 3) && w.chars().all(|c| c.is_ascii_hexdigit()))?;
    let hex: String = if word.len() == 3 {
        word.chars().flat_map(|c| [c, c]).collect()
    } else {
        word.to_owned()
    };
    u32::from_str_radix(&hex, 16).ok()
}
/// Markdown paragraphs and `-`/`*`/`+`/`1.` lists as blocks, so bullets are drawn without blank-line gaps.
fn markdown_item(text: &str, bold: bool, size: u32, muted: bool) -> CardItem {
    let text = text.replace("\r\n", "\n");
    if !text.lines().any(|l| list_marker(l).is_some()) {
        return CardItem::Text {
            runs: markdown(&text, bold, size),
            muted,
        };
    }
    let mut blocks: Vec<html::Block> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<html::Block>| {
        if !paragraph.is_empty() {
            blocks.push(html::Block::Text {
                runs: markdown(&paragraph.join("\n"), bold, size),
            });
            paragraph.clear();
        }
    };
    for line in text.lines() {
        if let Some((ordered, number, item)) = list_marker(line) {
            flush(&mut paragraph, &mut blocks);
            let entry = vec![html::Block::Text {
                runs: markdown(item, bold, size),
            }];
            match blocks.last_mut() {
                Some(html::Block::List {
                    ordered: same,
                    items,
                    ..
                }) if *same == ordered => items.push(entry),
                _ => blocks.push(html::Block::List {
                    ordered,
                    start: number,
                    items: vec![entry],
                }),
            }
        } else if line.trim().is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let (true, Some(html::Block::List { items, .. })) = (
            paragraph.is_empty() && line.starts_with("  "),
            blocks.last_mut(),
        ) {
            // An indented line continues the item above it.
            if let Some(html::Block::Text { runs }) = items.last_mut().and_then(|i| i.last_mut()) {
                runs.push(Run {
                    text: "\n".into(),
                    ..Default::default()
                });
                runs.extend(markdown(line.trim(), bold, size));
            }
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut blocks);
    CardItem::Blocks(blocks)
}
/// `- item`, `* item`, `+ item` or `3. item`: whether ordered, its number, its text.
fn list_marker(line: &str) -> Option<(bool, u32, &str)> {
    let line = line.trim_start();
    if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| line.strip_prefix(m)) {
        // `**bold**` at the start of a line is not a bullet.
        return Some((false, 1, rest.trim()));
    }
    let digits = line.find(|c: char| !c.is_ascii_digit())?;
    let rest = line[digits..].strip_prefix(". ")?;
    Some((true, line[..digits].parse().ok()?, rest.trim()))
}
/// Connector and hero card text: Markdown, or HTML as Teams also accepts there.
fn rich_text(text: &str, bold: bool, muted: bool) -> Option<CardItem> {
    if !looks_like_html(text) {
        return Some(markdown_item(text, bold, 0, muted));
    }
    let mut blocks = html::parse(text).blocks;
    match blocks.as_mut_slice() {
        [] => None,
        [html::Block::Text { runs }] => {
            for run in runs.iter_mut() {
                run.bold |= bold;
            }
            Some(CardItem::Text {
                runs: std::mem::take(runs),
                muted,
            })
        }
        _ => Some(CardItem::Blocks(blocks)),
    }
}
/// A title or fact value without markup, whether Markdown or HTML.
fn plain_text(text: &str) -> String {
    if looks_like_html(text) {
        html::to_text(text)
    } else {
        plain_markdown(text)
    }
}
/// A tag such as `<p>` or `</li>`, or an entity: not a Markdown `<https://…>` autolink.
fn looks_like_html(text: &str) -> bool {
    ["&nbsp;", "&amp;", "&lt;", "&gt;", "&quot;", "&#"]
        .iter()
        .any(|e| text.contains(e))
        || text.match_indices('<').any(|(i, _)| {
            let tag = text[i + 1..].trim_start_matches('/');
            tag.starts_with(|c: char| c.is_ascii_alphabetic())
                && !tag.starts_with("http")
                && !tag.starts_with("mailto")
                && tag.contains('>')
        })
}
fn plain_markdown(text: &str) -> String {
    markdown(text, false, 0)
        .into_iter()
        .map(|r| r.text)
        .collect()
}
/// The cards' Markdown subset (`**bold**`, `_italic_`/`*italic*`, `` `code` ``, `[label](url)`); the rest stays literal.
pub fn markdown(text: &str, bold: bool, size: u32) -> Vec<Run> {
    let base = Run {
        bold,
        size,
        ..Default::default()
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut current = base;
    let mut strong = false;
    let mut buffer = String::new();
    let flush = |runs: &mut Vec<Run>, buffer: &mut String, style: &Run| {
        if !buffer.is_empty() {
            runs.push(Run {
                text: std::mem::take(buffer),
                ..style.clone()
            });
        }
    };
    let chars: Vec<char> = text.replace("\r\n", "\n").chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut runs, &mut buffer, &current);
            strong = !strong;
            current.bold = bold || strong;
            i += 2;
            continue;
        }
        if (c == '_' || c == '*')
            && (current.italic
                || (i == 0 || !chars[i - 1].is_alphanumeric()) && chars[i + 1..].contains(&c))
        {
            flush(&mut runs, &mut buffer, &current);
            current.italic = !current.italic;
            i += 1;
            continue;
        }
        if c == '`'
            && let Some(end) = chars[i + 1..].iter().position(|&x| x == '`')
        {
            flush(&mut runs, &mut buffer, &current);
            runs.push(Run {
                text: chars[i + 1..i + 1 + end].iter().collect(),
                code: true,
                ..current.clone()
            });
            i += end + 2;
            continue;
        }
        if c == '['
            && let Some((label, url, used)) = link(&chars[i..].iter().collect::<String>())
        {
            flush(&mut runs, &mut buffer, &current);
            runs.push(Run {
                text: label,
                link: html::safe_link(&url),
                ..current.clone()
            });
            i += used;
            continue;
        }
        buffer.push(c);
        i += 1;
    }
    flush(&mut runs, &mut buffer, &current);
    runs
}
/// `[label](url)` at the start of `text`: label, url and the characters used.
fn link(text: &str) -> Option<(String, String, usize)> {
    let close = text.find("](")?;
    let label = &text[1..close];
    if label.contains('\n') {
        return None;
    }
    let end = text[close + 2..].find(')')?;
    let url = &text[close + 2..close + 2 + end];
    if url.contains(char::is_whitespace) {
        return None;
    }
    let used = text[..close + 3 + end].chars().count();
    Some((label.to_owned(), url.to_owned(), used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn message(cards: Value) -> Message {
        let mut m = Message::default();
        m.properties
            .insert("cards".into(), json!(cards.to_string()));
        m
    }
    #[test]
    fn adaptive_cards_keep_text_facts_images_and_safe_links() {
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [
                {"type": "TextBlock", "text": "Déploiement **réussi**", "weight": "Bolder", "size": "Large"},
                {"type": "ColumnSet", "columns": [{"items": [
                    {"type": "FactSet", "facts": [{"title": "Version", "value": "1.2.3"}]},
                    {"type": "Image", "url": "https://example.test/logo.png", "altText": "Logo"}
                ]}]}
            ], "actions": [
                {"type": "Action.OpenUrl", "title": "Voir", "url": "https://example.test/run/1"},
                {"type": "Action.OpenUrl", "title": "Piège", "url": "javascript:alert(1)"},
                {"type": "Action.Submit", "title": "Approuver"}
            ]}
        }])));
        let card = &cards[0];
        let CardItem::Text { runs, .. } = &card.items[0] else {
            panic!("{card:?}");
        };
        assert_eq!(runs[0].size, 18);
        assert!(runs.iter().any(|r| r.text == "réussi" && r.bold));
        assert_eq!(
            card.items[1],
            CardItem::Facts(vec![Fact {
                title: "Version".into(),
                value: "1.2.3".into()
            }])
        );
        assert_eq!(card.images().next().unwrap().alt, "Logo");
        assert_eq!(
            card.actions[0].url.as_deref(),
            Some("https://example.test/run/1")
        );
        assert_eq!(card.actions[1].url, None);
        assert_eq!(card.actions[2].url, None);
    }
    /// GitHub's pull request cards box their status and link in a shaded table row.
    #[test]
    fn emphasis_rows_and_containers_are_boxed_and_others_stay_inline() {
        let text = |t: &str| json!({"type": "TextBlock", "text": t});
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [
                {"type": "Table", "columns": [{"width": 1}], "rows": [
                    {"type": "TableRow", "style": "emphasis", "cells": [{"type": "TableCell", "items": [
                        {"type": "Image", "url": "https://example.test/open.png", "height": "24px"},
                        text("PR title")
                    ]}]},
                    {"type": "TableRow", "cells": [{"type": "TableCell", "items": [text("plain row")]}]}
                ]},
                {"type": "Container", "style": "Emphasis", "items": [text("boxed")]},
                {"type": "Container", "style": "default", "items": [text("inline")]}
            ]}
        }])));
        let card = &cards[0];
        let CardItem::Emphasis(row) = &card.items[0] else {
            panic!("{card:?}");
        };
        assert!(matches!(row[0], CardItem::Image(_)));
        assert!(matches!(row[1], CardItem::Text { .. }));
        assert!(matches!(card.items[1], CardItem::Text { .. }));
        assert!(matches!(&card.items[2], CardItem::Emphasis(items) if items.len() == 1));
        assert!(matches!(card.items[3], CardItem::Text { .. }));
        assert_eq!(card.items.len(), 4);
        assert_eq!(
            card.images().next().unwrap().src,
            "https://example.test/open.png"
        );
    }
    #[test]
    fn column_sets_stay_side_by_side_without_their_hidden_columns() {
        let text = |t: &str| json!({"type": "TextBlock", "text": t});
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [
                {"type": "ColumnSet", "columns": [
                    {"width": "auto", "items": [{"type": "Image", "url": "https://example.test/a.png", "size": "Small", "style": "Person"}]},
                    {"width": "stretch", "items": [text("bot wants to merge 4 commits")]}
                ]},
                {"type": "ColumnSet", "columns": [
                    {"id": "closed", "width": "auto", "items": [text("1 Reviewer ▼")]},
                    {"id": "open", "isVisible": false, "width": "auto", "items": [text("1 Reviewer ▲")]},
                    {"width": "auto", "items": [text("•")]},
                    {"width": 2, "items": [text("3 Files changed")]},
                    {"width": "20px", "items": []}
                ]},
                {"type": "ColumnSet", "columns": [{"items": [text("seule")]}]}
            ]}
        }])));
        let card = &cards[0];
        let CardItem::Columns(header) = &card.items[0] else {
            panic!("{card:?}");
        };
        assert_eq!(header[0].width, ColumnWidth::Auto);
        let CardItem::Image(avatar) = &header[0].items[0] else {
            panic!("{header:?}");
        };
        assert_eq!((avatar.width, avatar.height), (Some(40), Some(40)));
        assert_eq!(header[1].width, ColumnWidth::Stretch);
        let CardItem::Columns(stats) = &card.items[1] else {
            panic!("{card:?}");
        };
        assert_eq!(
            stats.iter().map(|c| c.width).collect::<Vec<_>>(),
            [ColumnWidth::Auto, ColumnWidth::Auto, ColumnWidth::Weight(2)],
            "the hidden and empty columns are gone"
        );
        assert!(matches!(&card.items[2], CardItem::Text { runs, .. } if runs[0].text == "seule"));
        assert_eq!(card.images().count(), 1);
        assert_eq!(
            card.text_runs()[0].text,
            "bot wants to merge 4 commits\n1 Reviewer ▼ • 3 Files changed\nseule"
        );
    }
    #[test]
    fn hero_and_connector_cards() {
        let cards = from_message(&message(json!([
            {"contentType": "application/vnd.microsoft.card.hero", "content": {
                "title": "Titre", "subtitle": "Sous-titre", "text": "Voir [la doc](https://doc.test/)",
                "buttons": [{"type": "openUrl", "title": "Ouvrir", "value": "https://doc.test/"}]}},
            {"contentType": "application/vnd.microsoft.teams.card.o365connector", "content": {
                "title": "Incident", "sections": [{"activityTitle": "Alerte", "facts": [{"name": "Sévérité", "value": "Haute"}]}],
                "potentialAction": [{"@type": "OpenUri", "name": "Détails", "targets": [{"os": "default", "uri": "https://status.test/"}]}]}}
        ])));
        assert_eq!(cards[0].title.as_deref(), Some("Titre"));
        let CardItem::Text { runs, .. } = &cards[0].items[0] else {
            panic!()
        };
        assert!(runs.iter().any(|r| r.text == "la doc" && r.link.is_some()));
        assert_eq!(
            cards[0].actions[0].url.as_deref(),
            Some("https://doc.test/")
        );
        assert_eq!(cards[1].title.as_deref(), Some("Incident"));
        assert!(matches!(&cards[1].items[1], CardItem::Facts(f) if f[0].value == "Haute"));
        assert_eq!(
            cards[1].actions[0].url.as_deref(),
            Some("https://status.test/")
        );
    }
    /// Uptime Kuma's MessageCard, as Workflows posts it.
    #[test]
    fn connector_sections_follow_teams_layout_and_accent() {
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.teams.card.o365connector", "content": {
                "@type": "MessageCard", "themeColor": "#ff0000", "summary": "down",
                "sections": [
                    {"activityImage": "https://example.test/icon.png", "activityTitle": "**Uptime Kuma**"},
                    {"activityTitle": "🔴 Application [Volumes Backup] went down"},
                    {"activityTitle": "**Description**", "text": "Request failed with status code 404",
                     "facts": [{"name": "Monitor", "value": "Volumes Backup"}, {"name": "URL", "value": "1"}],
                     "images": [{"image": "https://example.test/graph.png"}]}
                ]}
        }])));
        let card = &cards[0];
        assert_eq!(card.accent, Some(0xFF0000));
        let CardItem::Columns(activity) = &card.items[0] else {
            panic!("{card:?}");
        };
        assert_eq!(activity[0].width, ColumnWidth::Points(52));
        assert!(
            matches!(&activity[0].items[0], CardItem::Image(i) if (i.width, i.height) == (Some(52), Some(52)))
        );
        assert!(
            matches!(&activity[1].items[0], CardItem::Text { runs, .. } if runs[0].text == "Uptime Kuma" && runs[0].bold)
        );
        assert!(matches!(&card.items[1], CardItem::Text { .. }));
        assert!(matches!(&card.items[2], CardItem::Text { runs, .. } if runs[0].text == "Description"));
        assert!(matches!(&card.items[3], CardItem::Text { runs, .. } if !runs[0].bold));
        assert!(matches!(&card.items[4], CardItem::Facts(f) if f.len() == 2));
        assert!(
            matches!(&card.items[5], CardItem::Image(i) if (i.width, i.height) == (Some(160), Some(160)))
        );
        assert_eq!(theme_color("0076D7"), Some(0x0076D7));
        assert_eq!(theme_color("#f00"), Some(0xFF0000));
        assert_eq!(theme_color("red"), None);
    }
    #[test]
    fn markdown_lists_become_bullets_without_blank_gaps() {
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [
                {"type": "TextBlock", "text": "Usage du jour :\n- **apodex-mini** — 13.7k tokens\n\n- **north-mini** — 12.4k tokens\n  suite de l’élément\n\n1. Premier\n2. Second"},
                {"type": "TextBlock", "text": "**Gras** en début de ligne\n*italique*"}
            ]}
        }])));
        let card = &cards[0];
        let CardItem::Blocks(blocks) = &card.items[0] else {
            panic!("{card:?}");
        };
        let [
            html::Block::Text { runs },
            html::Block::List {
                ordered: false,
                items,
                ..
            },
            html::Block::List {
                ordered: true,
                start: 1,
                items: numbered,
            },
        ] = &blocks[..]
        else {
            panic!("{blocks:?}");
        };
        assert_eq!(runs[0].text, "Usage du jour :");
        assert_eq!(items.len(), 2);
        assert_eq!(numbered.len(), 2);
        let [html::Block::Text { runs: second }] = &items[1][..] else {
            panic!("{items:?}");
        };
        assert!(second.iter().any(|r| r.text == "north-mini" && r.bold));
        assert!(second.iter().any(|r| r.text.contains("suite de l’élément")));
        assert!(
            matches!(&card.items[1], CardItem::Text { runs, .. } if runs[0].text == "Gras" && runs[0].bold)
        );
    }
    #[test]
    fn adaptive_spacing_and_separators_are_kept() {
        let cards = from_message(&message(json!([{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [
                {"type": "TextBlock", "text": "Titre", "spacing": "large"},
                {"type": "TextBlock", "text": "Section", "spacing": "Medium", "separator": true},
                {"type": "TextBlock", "text": "Suite", "spacing": "small"}
            ]}
        }])));
        let kinds: Vec<_> = cards[0]
            .items
            .iter()
            .map(|i| match i {
                CardItem::Gap { points, separator } => format!("gap {points} {separator}"),
                _ => "text".into(),
            })
            .collect();
        assert_eq!(kinds, ["text", "gap 8 true", "text", "text"]);
    }
    #[test]
    fn connector_html_is_rendered_not_shown_as_source() {
        let cards = from_message(&message(json!([
            {"contentType": "application/vnd.microsoft.teams.card.o365connector", "content": {
                "title": "<p>Absences du : 19/10/2026</p>",
                "text": "<div><div>Bonjour</div><ul><li><strong>Aurélien</strong>&nbsp;Arrivée tardive</li>\
                         <li><strong>Eugène</strong>&nbsp;ABS Journée</li></ul><hr>\
                         <div>Perdus ? <a href=\"https://abs.example.test\">Outil ABS</a></div></div>",
                "sections": [{"activityText": "<b>Note</b> &amp; suite"}]}},
            {"contentType": "application/vnd.microsoft.card.hero", "content": {
                "title": "Voir <https://doc.test/>", "text": "a < b et 2 > 1"}}
        ])));
        let card = &cards[0];
        assert_eq!(card.title.as_deref(), Some("Absences du : 19/10/2026"));
        let CardItem::Blocks(blocks) = &card.items[0] else {
            panic!("{card:?}");
        };
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, html::Block::List { items, .. } if items.len() == 2))
        );
        assert!(blocks.contains(&html::Block::Rule));
        let text = card.text_runs()[0].text.clone();
        assert!(!text.contains('<') && !text.contains("&nbsp;"), "{text}");
        assert!(text.contains("Aurélien"));
        let CardItem::Text { runs, .. } = &card.items[1] else {
            panic!("{card:?}");
        };
        assert!(runs.iter().any(|r| r.text == "Note" && r.bold));
        assert_eq!(
            runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            "Note & suite"
        );
        // Markdown autolinks and comparisons are not mistaken for HTML.
        assert_eq!(cards[1].title.as_deref(), Some("Voir <https://doc.test/>"));
        assert!(
            matches!(&cards[1].items[0], CardItem::Text { runs, .. } if runs[0].text == "a < b et 2 > 1")
        );
    }
    #[test]
    fn swift_uri_objects_and_unknown_cards() {
        let payload = json!({"type": "message/card", "attachments": [
            {"contentType": "application/vnd.microsoft.card.unknown", "content": {"summary": "Résumé"}}
        ]});
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
        let doc = html::parse(&format!(
            "<URIObject type=\"SWIFT.1\" uri=\"x\"><Swift b64=\"{b64}\"></Swift></URIObject>"
        ));
        let [html::Block::Card(card)] = &doc.blocks[..] else {
            panic!("{:?}", doc.blocks);
        };
        assert!(
            matches!(&card.items[0], CardItem::Text { runs, muted: true } if runs[0].text == "Résumé")
        );
        assert_eq!(doc.text(), "Résumé");
    }
}
