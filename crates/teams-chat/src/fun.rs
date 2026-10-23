//! GIF search, which the middle tier proxies to Giphy as an "input extension" command.
use http::Method;
use serde_json::{Value, json};
use teams_auth::Authenticator;

use crate::client::ChatClient;
use crate::error::Error;

const GIPHY: &str =
    "beta/commands/com.microsoft.teamspace.inputextension.giphy/execute?version=1.0";
/// `GIPHYS_RESULT_LIMIT`, sent as a string.
const LIMIT: &str = "27";

/// One GIF as the middle tier flattens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gif {
    /// Full GIF URL: what the message carries (`composeValue`).
    pub url: String,
    /// Small still or animation for a picker (`previewImage`).
    pub preview: String,
    pub width: u32,
    pub height: u32,
    pub title: String,
}
/// `giphyRatingType` to Giphy's rating, as the web client maps it: Strict `y`, Moderate `g`, NoRestriction `pg-13`.
pub fn giphy_rating(policy: Option<&str>) -> &'static str {
    match policy {
        Some("Moderate") => "g",
        Some("NoRestriction") => "pg-13",
        _ => "y",
    }
}
fn field<'a>(item: &'a Value, name: &str) -> Option<&'a str> {
    item.get("fieldValues")?
        .as_array()?
        .iter()
        .find(|f| f.get("fieldName").and_then(Value::as_str) == Some(name))?
        .get("fieldValue")?
        .as_str()
}
/// The shape of an answer, without its content: its keys, nested two levels.
pub(crate) fn shape(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let keys: Vec<String> = o
                .iter()
                .take(12)
                .map(|(k, v)| match v {
                    Value::Object(_) | Value::Array(_) => format!("{k}:{}", shape_inner(v)),
                    _ => k.clone(),
                })
                .collect();
            format!("{{{}}}", keys.join(","))
        }
        Value::Array(a) => format!(
            "[{} × {}]",
            a.len(),
            a.first().map(shape_inner).unwrap_or_default()
        ),
        Value::Null => "vide".into(),
        _ => "valeur".into(),
    }
}
fn shape_inner(v: &Value) -> String {
    match v {
        Value::Object(o) => format!(
            "{{{}}}",
            o.keys().take(12).cloned().collect::<Vec<_>>().join(",")
        ),
        Value::Array(a) => format!("[{}]", a.len()),
        _ => "valeur".into(),
    }
}
/// GIFs of `{result: [{result: [...]}]}` (first entry, as the web client) or a flat `result`; else the shape.
fn gifs(v: &Value) -> Result<Vec<Gif>, String> {
    let outer = v
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| shape(v))?;
    let items = match outer.first().and_then(|first| first.get("result")) {
        Some(Value::Array(inner)) => inner,
        Some(Value::Null) => return Ok(vec![]),
        _ => outer,
    };
    let found = parse_items(items);
    if found.is_empty() && !items.is_empty() {
        return Err(shape(v));
    }
    Ok(found)
}
fn parse_items(items: &[Value]) -> Vec<Gif> {
    items
        .iter()
        .filter_map(|item| {
            let url = item.get("composeValue")?.as_str()?.to_owned();
            if !url.starts_with("https://") {
                return None;
            }
            let number = |name| field(item, name).and_then(|s| s.trim().parse().ok());
            Some(Gif {
                preview: field(item, "previewImage")
                    .or_else(|| field(item, "image"))
                    .filter(|s| s.starts_with("https://"))
                    .unwrap_or(&url)
                    .to_owned(),
                width: number("width").unwrap_or(0),
                height: number("height").unwrap_or(0),
                title: field(item, "title").unwrap_or_default().trim().to_owned(),
                url,
            })
        })
        .collect()
}

impl<A: Authenticator> ChatClient<A> {
    /// GIFs matching `query`; an empty query gives the popular ones.
    pub async fn search_gifs(
        &self,
        query: &str,
        rating: &str,
        lang: &str,
    ) -> Result<Vec<Gif>, Error> {
        let body = json!([
            {"name": "query", "value": query.trim().to_lowercase()},
            {"name": "rating", "value": rating},
            {"name": "limit", "value": LIMIT},
            {"name": "lang", "value": lang},
        ]);
        let v = self
            .mt("getGiphys", Method::POST, GIPHY, Some(body))
            .await?;
        gifs(&v).map_err(|shape| {
            eprintln!("teams-chat: unexpected Giphy answer {shape}");
            Error::Decode {
                endpoint: "getGiphys",
                source: serde::de::Error::custom(format!("unexpected answer {shape}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_command_answer_nests_its_gifs() {
        let v = json!({"commandId": "giphy", "parameters": [{}, {}, {}, {}], "schema": {},
        "result": [{"result": [
            {"composeValue": "https://media.giphy.com/media/b/giphy.gif", "fieldValues": []}
        ]}]});
        assert_eq!(
            gifs(&v).unwrap()[0].url,
            "https://media.giphy.com/media/b/giphy.gif"
        );
        assert_eq!(
            gifs(&json!({"result": [{"result": null}]})).unwrap(),
            vec![]
        );
    }
    #[test]
    fn results_keep_https_gifs_with_their_fields() {
        let v = json!({"result": [
            {"composeValue": "https://media.giphy.com/media/a/giphy.gif", "fieldValues": [
                {"fieldName": "previewImage", "fieldValue": "https://media.giphy.com/media/a/200w.gif"},
                {"fieldName": "width", "fieldValue": "480"},
                {"fieldName": "height", "fieldValue": "270"},
                {"fieldName": "title", "fieldValue": "Cat typing"}]},
            {"composeValue": "http://insecure/x.gif"}
        ]});
        let found = gifs(&v).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].width, 480);
        assert_eq!(found[0].preview, "https://media.giphy.com/media/a/200w.gif");
        assert_eq!(giphy_rating(Some("Moderate")), "g");
        assert_eq!(gifs(&json!({"result": []})).unwrap(), vec![]);
        let wrapped = gifs(&json!({"value": {"result": [{"x": 1}]}})).unwrap_err();
        assert_eq!(wrapped, "{value:{result}}");
        assert!(
            gifs(&json!({"result": [{"composeValue": "data:x"}]}))
                .unwrap_err()
                .contains("result:[1]")
        );
        assert_eq!(giphy_rating(None), "y");
    }
}
