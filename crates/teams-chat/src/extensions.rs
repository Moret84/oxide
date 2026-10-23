//! Message extensions (the compose box's apps): their commands, and the bot invokes that search, select and submit.
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};

use crate::{ChatClient, Error};

/// One command of an app's message extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionCommand {
    pub id: String,
    /// `action` commands open a form (`composeExtension/fetchTask`); the others search.
    pub action: bool,
    pub title: String,
    pub description: String,
    /// The search runs as soon as the command opens, without a query.
    pub initial_run: bool,
    /// An action command whose form is in the manifest (`parameters`) rather than asked of the bot.
    pub fetch_task: bool,
    pub parameters: Vec<ExtensionParameter>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionParameter {
    pub name: String,
    pub title: String,
    pub description: String,
    /// `text`, `textarea`, `number`, `date`, `time`, `toggle` or `choiceset`.
    pub input_type: String,
    /// `(title, value)` of a `choiceset`.
    pub choices: Vec<(String, String)>,
}
/// An app with a message extension usable from the compose box.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageExtension {
    pub app_id: String,
    pub name: String,
    pub icon: Option<String>,
    /// The bot answering the invokes, without its `28:` prefix.
    pub bot_id: String,
    pub commands: Vec<ExtensionCommand>,
}

/// An attachment a search or a form answered with: what is inserted in the message, and how the list shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtensionItem {
    /// `{contentType, content}` as the message will carry it.
    pub attachment: Value,
    pub title: String,
    pub text: String,
    pub image: Option<String>,
    /// The preview's `tap` value: picking the item asks the bot (`composeExtension/selectItem`) for the card.
    pub tap: Option<Value>,
}
/// What a message extension answered.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionAnswer {
    /// Results to pick from; `grid` lays them out as tiles.
    Results {
        items: Vec<ExtensionItem>,
        grid: bool,
    },
    /// Text the bot shows instead of results.
    Message(String),
    /// Sign-in or settings, in a page Teams opens (`auth`, `silentAuth`, `config`).
    OpenUrl {
        url: String,
        title: String,
        config: bool,
    },
    /// A form to fill: an Adaptive Card, or a web page only Teams can host.
    Task(TaskModule),
    /// Nothing to show: the bot acted on its own.
    Done,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TaskModule {
    pub title: String,
    /// The Adaptive Card of the form.
    pub card: Option<Value>,
    /// A web form, which this app cannot host.
    pub url: Option<String>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn https(url: Option<&str>) -> Option<String> {
    url.filter(|u| u.starts_with("https://")).map(str::to_owned)
}

/// The message extensions of app definitions (`messageExtensions`, or the manifest's `composeExtensions`).
pub fn extensions(definitions: &Value) -> Vec<MessageExtension> {
    let list = definitions
        .as_array()
        .or_else(|| definitions.get("value")?.as_array())
        .or_else(|| definitions.pointer("/value/appDefinitions")?.as_array())
        .or_else(|| definitions.get("appDefinitions")?.as_array());
    list.into_iter().flatten().filter_map(extension).collect()
}
fn extension(app: &Value) -> Option<MessageExtension> {
    // The service's definitions call them `inputExtensions`; the web client maps them to `messageExtensions`.
    let me = ["inputExtensions", "messageExtensions", "composeExtensions"]
        .iter()
        .find_map(|key| {
            app.get(*key)
                .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        })?
        .as_array()?
        .first()?;
    let commands: Vec<ExtensionCommand> = me
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| compose_context(c.get("context")))
        .filter_map(command)
        .collect();
    if commands.is_empty() {
        return None;
    }
    Some(MessageExtension {
        app_id: text(app, "id")?.to_owned(),
        name: text(app, "name")?.to_owned(),
        icon: https(text(app, "largeImageUrl").or_else(|| text(app, "smallImageUrl"))),
        bot_id: text(me, "botId")?.trim_start_matches("28:").to_owned(),
        commands,
    })
}
/// A command without `context` is offered in the compose box and the command box, as Teams does.
fn compose_context(context: Option<&Value>) -> bool {
    match context.and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list
            .iter()
            .filter_map(Value::as_str)
            .any(|c| c.eq_ignore_ascii_case("compose")),
        _ => true,
    }
}
fn command(c: &Value) -> Option<ExtensionCommand> {
    let id = text(c, "id")?.to_owned();
    let kind = text(c, "type").unwrap_or("query");
    Some(ExtensionCommand {
        title: text(c, "title").unwrap_or(&id).to_owned(),
        description: text(c, "description").unwrap_or_default().to_owned(),
        action: kind.eq_ignore_ascii_case("action"),
        initial_run: c
            .get("initialRun")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fetch_task: c.get("fetchTask").and_then(Value::as_bool).unwrap_or(false),
        parameters: c
            .get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parameter)
            .collect(),
        id,
    })
}
fn parameter(p: &Value) -> Option<ExtensionParameter> {
    let name = text(p, "name")?.to_owned();
    Some(ExtensionParameter {
        title: text(p, "title").unwrap_or(&name).to_owned(),
        description: text(p, "description").unwrap_or_default().to_owned(),
        input_type: text(p, "inputType").unwrap_or("text").to_ascii_lowercase(),
        choices: p
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| Some((text(c, "title")?.to_owned(), text(c, "value")?.to_owned())))
            .collect(),
        name,
    })
}

/// The bot's answer, whether bare (`{composeExtension}`, `{task}`) or wrapped in an InvokeResponse `body`/`value`.
pub fn answer(v: &Value) -> ExtensionAnswer {
    for root in [Some(v), v.get("body"), v.get("value")]
        .into_iter()
        .flatten()
    {
        if let Some(ce) = root.get("composeExtension") {
            return compose_extension(ce);
        }
        if let Some(task) = root.get("task") {
            return task_answer(task);
        }
    }
    ExtensionAnswer::Done
}
fn compose_extension(ce: &Value) -> ExtensionAnswer {
    let kind = text(ce, "type").unwrap_or("result").to_ascii_lowercase();
    match kind.as_str() {
        "message" => ExtensionAnswer::Message(text(ce, "text").unwrap_or_default().to_owned()),
        "auth" | "silentauth" | "config" => {
            let action = ce
                .pointer("/suggestedActions/actions")
                .and_then(Value::as_array)
                .and_then(|a| a.first());
            match https(action.and_then(|a| text(a, "value"))) {
                Some(url) => ExtensionAnswer::OpenUrl {
                    url,
                    title: action
                        .and_then(|a| text(a, "title"))
                        .unwrap_or_default()
                        .to_owned(),
                    config: kind == "config",
                },
                None => ExtensionAnswer::Message(String::new()),
            }
        }
        // `botMessagePreview` carries one card, to send as is.
        "botmessagepreview" => ExtensionAnswer::Results {
            items: ce
                .get("activityPreview")
                .and_then(|a| a.get("attachments"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(item)
                .collect(),
            grid: false,
        },
        _ => ExtensionAnswer::Results {
            items: ce
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(item)
                .collect(),
            grid: text(ce, "attachmentLayout").is_some_and(|l| l.eq_ignore_ascii_case("grid")),
        },
    }
}
fn task_answer(task: &Value) -> ExtensionAnswer {
    let kind = text(task, "type").unwrap_or("continue");
    let value = task.get("value").unwrap_or(&Value::Null);
    if kind.eq_ignore_ascii_case("message") {
        let message = value.as_str().map(str::trim).unwrap_or_default();
        return ExtensionAnswer::Message(message.to_owned());
    }
    let card = value
        .get("card")
        .map(|c| c.get("content").unwrap_or(c).clone())
        .map(|c| match c {
            Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
            other => other,
        })
        .filter(Value::is_object);
    ExtensionAnswer::Task(TaskModule {
        title: text(value, "title").unwrap_or_default().to_owned(),
        url: if card.is_none() {
            https(text(value, "url"))
        } else {
            None
        },
        card,
    })
}
/// A result attachment and its `preview` (a hero or thumbnail card, or the attachment itself).
fn item(a: &Value) -> Option<ExtensionItem> {
    let content_type = text(a, "contentType")?;
    let mut attachment = json!({"contentType": content_type, "content": a.get("content")?});
    if let Some(id) = a.get("id") {
        attachment["id"] = id.clone();
    }
    let preview = a.get("preview").and_then(|p| p.get("content"));
    let shown = preview.or_else(|| a.get("content")).unwrap_or(&Value::Null);
    let mut title = text(shown, "title").unwrap_or_default().to_owned();
    let mut body = text(shown, "text")
        .or_else(|| text(shown, "subtitle"))
        .unwrap_or_default()
        .to_owned();
    // An Adaptive Card without a preview: its first text blocks.
    if title.is_empty() && body.is_empty() {
        let mut blocks = Vec::new();
        text_blocks(shown.get("body"), &mut blocks);
        let mut blocks = blocks.into_iter();
        title = blocks.next().unwrap_or_default();
        body = blocks.next().unwrap_or_default();
    }
    let image = shown
        .get("images")
        .and_then(Value::as_array)
        .and_then(|i| i.first())
        .and_then(|i| https(text(i, "url")));
    Some(ExtensionItem {
        attachment,
        title,
        text: strip_tags(&body),
        image,
        tap: preview
            .and_then(|p| p.get("tap"))
            .and_then(|t| t.get("value"))
            .cloned(),
    })
}
fn text_blocks(body: Option<&Value>, out: &mut Vec<String>) {
    for element in body.and_then(Value::as_array).into_iter().flatten() {
        if out.len() >= 2 {
            return;
        }
        if let Some(t) = text(element, "text") {
            out.push(t.to_owned());
        }
        for key in ["items", "columns"] {
            text_blocks(element.get(key), out);
        }
    }
}
/// Hero card text may hold simple HTML.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// Where an invoke goes and what it says, as the web client's `botInvokeParams`.
#[derive(Debug, Clone, Default)]
pub struct ExtensionTarget {
    pub bot_id: String,
    pub app_id: String,
    /// The conversation, `;messageid=` and the post when answering in a channel thread.
    pub thread_id: String,
    pub display_name: String,
}

/// App ids of an entitlements answer: a bare array, or under `value`/`appEntitlements`; else its shape.
fn entitled_ids(v: &Value) -> Result<Vec<String>, String> {
    let list = v
        .as_array()
        .or_else(|| v.get("appEntitlements")?.as_array())
        .or_else(|| v.get("value")?.as_array())
        .or_else(|| v.pointer("/value/appEntitlements")?.as_array())
        .ok_or_else(|| crate::fun::shape(v))?;
    Ok(list
        .iter()
        .filter(|e| {
            !text(e, "state").is_some_and(|s| {
                ["blocked", "uninstalled", "removed"].contains(&s.to_ascii_lowercase().as_str())
            })
        })
        .filter_map(|e| text(e, "id").or_else(|| text(e, "appId")))
        .map(str::to_owned)
        .collect())
}

/// The message extensions found, and what the service answered when there are none, to tell why.
#[derive(Debug, Clone, Default)]
pub struct ExtensionsLookup {
    pub found: Vec<MessageExtension>,
    /// Installed apps: the user's, and the chat's.
    pub entitled: usize,
    /// The keys of the answers (no content), for an empty list.
    pub shapes: String,
}

impl<A: Authenticator> ChatClient<A> {
    /// The message extensions of the apps the user installed, and those of group chat `chat` if given.
    pub async fn message_extensions(
        &self,
        language: &str,
        chat: Option<&str>,
    ) -> Result<ExtensionsLookup, Error> {
        let decode = |endpoint: &'static str, shape: String| {
            eprintln!("teams-chat: unexpected {endpoint} answer {shape}");
            Error::Decode {
                endpoint,
                source: serde::de::Error::custom(format!("unexpected answer {shape}")),
            }
        };
        let user = self
            .mt(
                "getAppEntitlements",
                Method::GET,
                "beta/users/apps/entitlements",
                None,
            )
            .await?;
        let mut ids = entitled_ids(&user).map_err(|s| decode("getAppEntitlements", s))?;
        let mut shapes = vec![format!("entitlements {}", crate::fun::shape(&user))];
        if let Some(chat) = chat {
            let path = format!(
                "beta/chats/{}/apps/chatentitlements",
                crate::path::enc(chat)
            );
            // A chat without apps, or one the service does not know, still has the user's.
            if let Ok(v) = self
                .mt("getChatAppEntitlements", Method::GET, &path, None)
                .await
            {
                shapes.push(format!("chatentitlements {}", crate::fun::shape(&v)));
                ids.extend(entitled_ids(&v).unwrap_or_default());
            }
        }
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(id.to_ascii_lowercase()));
        let entitled = ids.len();
        let mut headers = HeaderMap::new();
        if let Ok(language) = HeaderValue::from_str(language) {
            headers.insert("x-client-ui-language", language);
        }
        // The flyout's "AppDefinitions" source: every app the user can use, in pages.
        let mut found = Vec::new();
        let mut continuation = String::new();
        for _ in 0..10 {
            let mut page_headers = headers.clone();
            if let Ok(token) = HeaderValue::from_str(&continuation)
                && !continuation.is_empty()
            {
                page_headers.insert("x-ms-continuation", token);
            }
            let response = self
                .service_request(
                    Service::MiddleTier,
                    "getAppDefinitions",
                    Method::GET,
                    "beta/users/apps/definitionsv2",
                    None,
                    page_headers,
                )
                .await;
            let response = match response {
                Ok(response) => response,
                Err(e) => {
                    shapes.push(format!("definitionsv2 {e}"));
                    break;
                }
            };
            let next = response
                .headers()
                .get("x-ms-continuation")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let answer: Value = match response.bytes().await {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                Err(e) => {
                    shapes.push(format!("definitionsv2 {e}"));
                    break;
                }
            };
            let page = extensions(&answer);
            if page.is_empty() {
                shapes.push(format!("definitionsv2 {}", crate::fun::shape(&answer)));
            }
            found.extend(page);
            if next.is_empty() || next == continuation {
                break;
            }
            continuation = next;
        }
        ids.retain(|id| !found.iter().any(|e| e.app_id.eq_ignore_ascii_case(id)));
        for chunk in ids.chunks(50) {
            let answer: Value = self
                .service_json(
                    Service::MiddleTier,
                    "appDefinitions",
                    Method::POST,
                    "beta/users/apps/batchedDefinitions",
                    Some(json!(chunk)),
                    headers.clone(),
                )
                .await?;
            let before = found.len();
            found.extend(extensions(&answer));
            if found.len() == before {
                shapes.push(format!("batchedDefinitions {}", crate::fun::shape(&answer)));
            }
        }
        let mut seen = std::collections::HashSet::new();
        found.retain(|e| seen.insert(e.app_id.to_ascii_lowercase()));
        found.sort_by_cached_key(|e| e.name.to_lowercase());
        let shapes = shapes.join(" · ");
        if found.is_empty() {
            eprintln!("teams-chat: no message extension among {entitled} apps: {shapes}");
        }
        Ok(ExtensionsLookup {
            found,
            entitled,
            shapes,
        })
    }
    /// One message extension invoke: `composeExtension/query`, `selectItem`, `fetchTask` or `submitAction`.
    pub async fn extension_invoke(
        &self,
        target: &ExtensionTarget,
        name: &str,
        value: Value,
    ) -> Result<ExtensionAnswer, Error> {
        let body = json!({
            "name": name,
            "appId": target.app_id,
            "messageType": "RichText/Html",
            "value": value,
            "imdisplayname": target.display_name,
            "conversation": {"id": target.thread_id},
        });
        let bot = format!("28:{}", target.bot_id);
        let v = self.invoke_bot(&bot, &body).await?;
        Ok(answer(&v))
    }
}
/// `composeExtension/query`: the first parameter holds the search, or `initialRun` when there is none.
pub fn query_value(command: &ExtensionCommand, search: &str, skip: u32) -> Value {
    let search = search.trim();
    let parameters = match command.parameters.first() {
        Some(p) if !search.is_empty() => json!([{"name": p.name, "value": search}]),
        _ => json!([{"name": "initialRun", "value": "true"}]),
    };
    json!({
        "commandId": command.id,
        "parameters": parameters,
        "queryOptions": {"skip": skip, "count": QUERY_COUNT},
    })
}
/// Results asked per page, as the web client.
pub const QUERY_COUNT: u32 = 25;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definitions_keep_compose_commands() {
        let apps = json!([{
            "id": "app-1", "name": "Tickets", "largeImageUrl": "https://cdn.test/t.png",
            "inputExtensions": [{"botId": "28:BOT", "commands": [
                {"id": "search", "type": "query", "title": "Rechercher", "initialRun": true,
                 "parameters": [{"name": "q", "title": "Mot-clé"}]},
                {"id": "share", "type": "action", "title": "Partager", "context": ["message"]},
                {"id": "create", "type": "action", "title": "Créer", "fetchTask": true, "context": ["compose", "commandBox"]}
            ]}]
        }, {"id": "app-2", "name": "Sans extension"}]);
        let found = extensions(&apps);
        assert_eq!(found.len(), 1);
        let me = &found[0];
        assert_eq!(me.bot_id, "BOT");
        assert_eq!(
            me.commands
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["search", "create"]
        );
        assert!(me.commands[0].initial_run && !me.commands[0].action);
        assert!(me.commands[1].action && me.commands[1].fetch_task);
        assert_eq!(me.commands[0].parameters[0].input_type, "text");
    }
    #[test]
    fn entitlements_list_installed_app_ids() {
        let v = json!({"appEntitlements": [{"id": "a", "state": "Installed"}, {"id": "b", "state": "Blocked"}, {"appId": "c"}]});
        assert_eq!(entitled_ids(&v).unwrap(), ["a", "c"]);
        assert_eq!(entitled_ids(&json!([{"id": "x"}])).unwrap(), ["x"]);
        assert!(entitled_ids(&json!({"other": 1})).is_err());
    }
    #[test]
    fn queries_send_the_first_parameter_or_initial_run() {
        let command = ExtensionCommand {
            id: "search".into(),
            parameters: vec![ExtensionParameter {
                name: "q".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let v = query_value(&command, " rust ", 0);
        assert_eq!(v["parameters"], json!([{"name": "q", "value": "rust"}]));
        assert_eq!(v["queryOptions"], json!({"skip": 0, "count": 25}));
        let v = query_value(&command, "", 25);
        assert_eq!(v["parameters"][0]["name"], "initialRun");
    }
    #[test]
    fn results_keep_the_card_and_its_preview() {
        let v = json!({"composeExtension": {"type": "result", "attachmentLayout": "list", "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [{"type": "TextBlock", "text": "Ticket 42"}]},
            "preview": {"contentType": "application/vnd.microsoft.card.thumbnail", "content": {
                "title": "Ticket 42", "text": "<b>Ouvert</b>", "images": [{"url": "https://cdn.test/i.png"}],
                "tap": {"type": "invoke", "value": {"id": 42}}}}
        }, {
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {"type": "AdaptiveCard", "body": [{"type": "Container", "items": [
                {"type": "TextBlock", "text": "Sans aperçu"}, {"type": "TextBlock", "text": "Détail"}]}]}
        }]}});
        let ExtensionAnswer::Results { items, grid } = answer(&v) else {
            panic!("results");
        };
        assert!(!grid);
        assert_eq!(items[0].title, "Ticket 42");
        assert_eq!(items[0].text, "Ouvert");
        assert_eq!(items[0].image.as_deref(), Some("https://cdn.test/i.png"));
        assert_eq!(items[0].tap, Some(json!({"id": 42})));
        assert_eq!(items[0].attachment["content"]["type"], "AdaptiveCard");
        assert_eq!(
            (items[1].title.as_str(), items[1].text.as_str()),
            ("Sans aperçu", "Détail")
        );
    }
    #[test]
    fn other_answers_are_messages_links_and_forms() {
        assert_eq!(
            answer(&json!({"composeExtension": {"type": "message", "text": "Rien"}})),
            ExtensionAnswer::Message("Rien".into())
        );
        let auth = json!({"composeExtension": {"type": "auth", "suggestedActions": {"actions": [
            {"type": "openUrl", "title": "Se connecter", "value": "https://login.test/x"}]}}});
        assert!(matches!(
            answer(&auth),
            ExtensionAnswer::OpenUrl { config: false, .. }
        ));
        let task = json!({"task": {"type": "continue", "value": {"title": "Nouveau", "card": {
            "contentType": "application/vnd.microsoft.card.adaptive", "content": {"type": "AdaptiveCard", "body": []}}}}});
        let ExtensionAnswer::Task(form) = answer(&task) else {
            panic!("task");
        };
        assert_eq!(form.title, "Nouveau");
        assert!(form.card.is_some() && form.url.is_none());
        assert_eq!(answer(&json!({})), ExtensionAnswer::Done);
        assert_eq!(
            answer(
                &json!({"statusCode": 200, "body": {"task": {"type": "message", "value": "Envoyé"}}})
            ),
            ExtensionAnswer::Message("Envoyé".into())
        );
    }
}
