//! Bot card buttons: `Action.Submit` (and its `msteams` variants) and `Action.Execute`, invoked via `v1/agents/{bot}/invoke`.
use super::view::{BlockView, CardInputValue};
use super::*;
use crate::cards::{ActionKind, Card, CardAction};
use crate::html::Block;
use serde_json::{Map, Value, json};
use teams_i18n::{t, tf};

const ADAPTIVE: &str = "application/vnd.microsoft.card.adaptive";
const ACTIVITY_MESSAGE: &str = "application/vnd.microsoft.activity.message";

/// What the app shows of a card after one of its buttons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CardState {
    /// The user-specific card an `Action.Execute` answered with.
    pub card: Option<Card>,
    pub footer: Option<String>,
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum CardMsg {
    Action {
        message: String,
        card: u32,
        action: u32,
        inputs: Vec<CardInputValue>,
    },
    Done(u64, (String, u32), bool, Result<Value, String>),
}

/// What a button sends: an invoke, a chat message, or both (message first).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Plan {
    pub message: Option<String>,
    /// The message only goes to the people of the chat (`skipfanouttobots`).
    pub message_back: bool,
    pub invoke: Option<Value>,
    /// The answer may replace the card (`Action.Execute`).
    pub execute: bool,
}

/// Inputs merged into `data` as strings, replacing existing keys as the Adaptive Cards SDK does.
fn merged_data(data: Option<&Value>, inputs: &[CardInputValue]) -> Value {
    let mut map = match data {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    };
    for input in inputs.iter().filter(|i| !i.value.is_empty()) {
        map.insert(input.id.clone(), json!(input.value));
    }
    Value::Object(map)
}
/// `data` without `msteams`, then the keys of `msteams.value` it lacks (lodash `defaults`).
fn with_defaults(data: &Value, embedded: Option<&Value>) -> Value {
    let mut map = match data {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    strip_msteams(&mut map);
    let embedded = match embedded {
        Some(Value::String(s)) => serde_json::from_str(s).ok(),
        Some(v @ Value::Object(_)) => Some(v.clone()),
        _ => None,
    };
    if let Some(Value::Object(extra)) = embedded {
        for (key, value) in extra {
            map.entry(key).or_insert(value);
        }
    }
    Value::Object(map)
}
fn strip_msteams(map: &mut Map<String, Value>) {
    map.remove("msteams");
    map.remove("msTeams");
}
/// The request a button makes, from the card's JSON and the values typed in.
pub(crate) fn plan(action: &CardAction, inputs: &[CardInputValue]) -> Option<Plan> {
    let raw = &action.action;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Hero and thumbnail card buttons carry their payload in `value`.
    match kind.as_str() {
        "imback" => {
            let text = raw.get("value").and_then(Value::as_str)?;
            return Some(Plan {
                message: Some(text.to_owned()),
                ..Default::default()
            });
        }
        "messageback" => {
            let value = with_defaults(&Value::Null, raw.get("value"));
            return Some(Plan {
                message: raw
                    .get("displayText")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message_back: true,
                invoke: Some(json!({
                    "name": "messageback",
                    "text": raw.get("text").and_then(Value::as_str),
                    "value": value,
                })),
                ..Default::default()
            });
        }
        "invoke" => {
            return Some(Plan {
                invoke: Some(json!({"value": with_defaults(&Value::Null, raw.get("value"))})),
                ..Default::default()
            });
        }
        _ => {}
    }
    match action.kind {
        ActionKind::Execute => {
            let mut action_json = raw.clone();
            let mut data = merged_data(raw.get("data"), inputs);
            if let Value::Object(map) = &mut data {
                strip_msteams(map);
            }
            action_json["data"] = data;
            Some(Plan {
                invoke: Some(json!({
                    "name": "adaptiveCard/action",
                    "value": {"action": action_json, "trigger": "manual"},
                })),
                execute: true,
                ..Default::default()
            })
        }
        ActionKind::Submit => {
            let data = raw.get("data");
            let msteams = data.and_then(|d| d.get("msteams").or_else(|| d.get("msTeams")));
            let variant = msteams
                .and_then(|m| m.get("type"))
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase);
            if let Some(Value::String(text)) = data {
                // A string `data` is posted as the user's message, then invoked as text.
                return Some(Plan {
                    message: Some(text.clone()),
                    message_back: true,
                    invoke: Some(json!({"name": "messageback", "text": text, "value": {}})),
                    ..Default::default()
                });
            }
            let merged = merged_data(data, inputs);
            let msteams_value = msteams.and_then(|m| m.get("value"));
            match variant.as_deref() {
                Some("imback") => Some(Plan {
                    message: Some(
                        msteams_value
                            .and_then(Value::as_str)
                            .unwrap_or(&action.title)
                            .to_owned(),
                    ),
                    ..Default::default()
                }),
                Some("invoke") => Some(Plan {
                    invoke: Some(json!({"value": with_defaults(&merged, msteams_value)})),
                    ..Default::default()
                }),
                Some("messageback") => Some(Plan {
                    message: msteams
                        .and_then(|m| m.get("displayText"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    message_back: true,
                    invoke: Some(json!({
                        "name": "messageback",
                        "text": msteams.and_then(|m| m.get("text")).and_then(Value::as_str),
                        "value": with_defaults(&merged, msteams_value),
                    })),
                    ..Default::default()
                }),
                None => Some(Plan {
                    invoke: Some(
                        json!({"name": "messageback", "value": with_defaults(&merged, None)}),
                    ),
                    ..Default::default()
                }),
                Some(_) => None,
            }
        }
        ActionKind::OpenUrl | ActionKind::Unsupported => None,
    }
}
/// What the card shows after the bot answered an `Action.Execute`.
fn execute_result(response: &Value) -> CardState {
    let status = response
        .get("statusCode")
        .and_then(Value::as_u64)
        .unwrap_or(200);
    let kind = response
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let value = response.get("value");
    if (200..300).contains(&status) && kind == ADAPTIVE {
        let content = match value {
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
            Some(v) => v.clone(),
            None => Value::Null,
        };
        if let Some(card) = crate::cards::adaptive_card(&content) {
            return CardState {
                card: Some(card),
                ..Default::default()
            };
        }
    }
    let footer = if kind == ACTIVITY_MESSAGE {
        value.and_then(Value::as_str).map(|s| s.trim().to_owned())
    } else if status >= 400 {
        Some(
            value
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str)
                .map(|m| tf!("core.card_actions.bot_error", error = m))
                .unwrap_or_else(|| t("core.card_actions.bot_refused").into()),
        )
    } else {
        None
    };
    CardState {
        footer: footer.filter(|f| !f.is_empty()),
        ..Default::default()
    }
}

impl App {
    pub(crate) fn update_card(&mut self, msg: CardMsg) -> Task<Msg> {
        match msg {
            CardMsg::Action {
                message,
                card,
                action,
                inputs,
            } => self.card_action(message, card, action, inputs),
            CardMsg::Done(g, ..) if g != self.generation => Task::none(),
            CardMsg::Done(_, key, execute, result) => {
                let previous = self.card_states.get(&key).and_then(|s| s.card.clone());
                let state = match result {
                    Ok(response) if execute => {
                        let mut state = execute_result(&response);
                        if state.card.is_none() {
                            state.card = previous;
                        }
                        state
                    }
                    Ok(_) => CardState {
                        card: previous,
                        footer: Some(t("core.card_actions.sent").into()),
                        busy: false,
                    },
                    Err(e) => CardState {
                        card: previous,
                        footer: Some(tf!("core.card_actions.failed", error = e)),
                        busy: false,
                    },
                };
                self.card_states.insert(key, state);
                // The bot often answers with a new or edited message.
                self.load_feed(false)
            }
        }
    }
    fn card_action(
        &mut self,
        message_id: String,
        card: u32,
        action: u32,
        inputs: Vec<CardInputValue>,
    ) -> Task<Msg> {
        let (Some(client), Some(selection)) = (self.client.clone(), self.selected.clone()) else {
            return Task::none();
        };
        let key = (message_id.clone(), card);
        if self.card_states.get(&key).is_some_and(|s| s.busy) {
            return Task::none();
        }
        let Some(message) = self
            .conv
            .all_messages()
            .find(|m| m.id == message_id)
            .cloned()
        else {
            return Task::none();
        };
        let Some(shown) = self.shown_card(&message, card) else {
            return Task::none();
        };
        let Some(button) = shown.actions.get(action as usize) else {
            return Task::none();
        };
        if let Some(url) = &button.url {
            return Task::done(Msg::OpenUrl(url.clone()));
        }
        // Required inputs block the button, as the SDK's validation does.
        let missing = shown.inputs().any(|input| {
            input.required
                && inputs
                    .iter()
                    .find(|v| v.id == input.id)
                    .is_none_or(|v| v.value.trim().is_empty())
        });
        if missing && button.kind != ActionKind::OpenUrl {
            let mut state = self.card_states.get(&key).cloned().unwrap_or_default();
            state.footer = Some(t("core.card_actions.fill_required").into());
            self.card_states.insert(key, state);
            return Task::none();
        }
        let Some(plan) = plan(button, &inputs) else {
            return Task::none();
        };
        let bot = message.sender_mri().to_owned();
        // A channel post's replies are addressed as `{channel};messageid={root}`.
        let thread = match &selection {
            Selection::Channel { channel, .. } => {
                let root = self
                    .conv
                    .posts()
                    .iter()
                    .find(|p| p.messages.iter().any(|m| m.id == message_id))
                    .map(|p| p.id.clone())
                    .or_else(|| self.conv.reply().cloned())
                    .unwrap_or_else(|| message_id.clone());
                format!("{channel};messageid={root}")
            }
            Selection::Chat(id) => id.clone(),
        };
        let reply = self.conv.reply().cloned();
        let display_name = self.display_name.clone();
        let mut state = self.card_states.get(&key).cloned().unwrap_or_default();
        state.busy = true;
        state.footer = None;
        self.card_states.insert(key.clone(), state);
        let app_id = message
            .properties
            .get("fromAppMetadata")
            .and_then(|m| match m {
                Value::String(s) => serde_json::from_str::<Value>(s).ok(),
                other => Some(other.clone()),
            })
            .and_then(|m| m.get("id").and_then(Value::as_str).map(str::to_owned));
        let generation = self.generation;
        let execute = plan.execute;
        Task::perform(
            async move {
                if let Some(text) = &plan.message {
                    let mut out =
                        OutgoingMessage::html(client_message_id(), escape(text), &display_name);
                    out.messagetype = "Text".into();
                    out.content = text.clone();
                    out.properties.skip_fan_out_to_bots = plan.message_back;
                    client
                        .send_message(selection.id(), reply.as_deref(), &out)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                let Some(mut body) = plan.invoke else {
                    return Ok(Value::Null);
                };
                body["messageType"] = json!(message.messagetype);
                body["imdisplayname"] = json!(display_name);
                body["serverMessageId"] = json!(message.id);
                if let Some(id) = &message.clientmessageid {
                    body["clientMessageId"] = json!(id);
                }
                if let Some(app) = app_id {
                    body["appId"] = json!(app);
                }
                body["conversation"] = json!({"id": thread});
                if let Value::Object(map) = &mut body {
                    map.retain(|_, v| !v.is_null());
                }
                client
                    .invoke_bot(&bot, &body)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| Msg::Card(CardMsg::Done(generation, key, execute, r)),
        )
    }
    /// The card as shown: the bot's replacement when an `Action.Execute` gave one.
    fn shown_card(&self, message: &Message, index: u32) -> Option<Card> {
        if let Some(card) = self
            .card_states
            .get(&(message.id.clone(), index))
            .and_then(|s| s.card.clone())
        {
            return Some(card);
        }
        self.conv
            .rendered()
            .get(message)
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Card(card) => Some(card),
                _ => None,
            })
            .nth(index as usize)
            .cloned()
    }
    /// A message's blocks as views, with its cards numbered and their state applied.
    pub(crate) fn message_blocks(&self, message: &Message, blocks: &[Block]) -> Vec<BlockView> {
        let mut index = 0u32;
        let replaced: Vec<Block> = blocks
            .iter()
            .map(|block| match block {
                Block::Card(card) => {
                    let shown = self
                        .card_states
                        .get(&(message.id.clone(), index))
                        .and_then(|s| s.card.clone())
                        .unwrap_or_else(|| card.clone());
                    index += 1;
                    Block::Card(shown)
                }
                other => other.clone(),
            })
            .collect();
        let mut views = self.block_views(&replaced);
        let mut index = 0u32;
        for view in &mut views {
            if let BlockView::Card { card } = view {
                card.index = index;
                if let Some(state) = self.card_states.get(&(message.id.clone(), index)) {
                    card.footer = state.footer.clone();
                    card.busy = state.busy;
                }
                index += 1;
            }
        }
        views
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn action(raw: Value, kind: ActionKind) -> CardAction {
        CardAction {
            title: "Go".into(),
            url: None,
            kind,
            action: raw,
        }
    }
    fn input(id: &str, value: &str) -> CardInputValue {
        CardInputValue {
            id: id.into(),
            value: value.into(),
        }
    }
    #[test]
    fn submit_merges_inputs_and_msteams_defaults() {
        let plan = plan(
            &action(
                json!({"type": "Action.Submit", "data": {"a": 1, "msteams": {"type": "messageBack",
                    "text": "ok", "displayText": "J’accepte", "value": {"a": 2, "b": 3}}}}),
                ActionKind::Submit,
            ),
            &[input("comment", "Bien"), input("empty", "")],
        )
        .unwrap();
        assert_eq!(plan.message.as_deref(), Some("J’accepte"));
        assert!(plan.message_back);
        let invoke = plan.invoke.unwrap();
        assert_eq!(invoke["name"], "messageback");
        assert_eq!(invoke["text"], "ok");
        assert_eq!(invoke["value"], json!({"a": 1, "b": 3, "comment": "Bien"}));
    }
    #[test]
    fn plain_submit_and_imback_and_execute() {
        let plain = plan(
            &action(
                json!({"type": "Action.Submit", "data": {"x": "y"}}),
                ActionKind::Submit,
            ),
            &[input("n", "3")],
        )
        .unwrap();
        assert_eq!(plain.message, None);
        assert_eq!(plain.invoke.unwrap()["value"], json!({"x": "y", "n": "3"}));
        let imback = plan(
            &action(
                json!({"type": "Action.Submit", "data": {"msteams": {"type": "imBack", "value": "Oui"}}}),
                ActionKind::Submit,
            ),
            &[],
        )
        .unwrap();
        assert_eq!(
            (imback.message.as_deref(), imback.invoke),
            (Some("Oui"), None)
        );
        let execute = plan(
            &action(
                json!({"type": "Action.Execute", "verb": "approve", "data": {"id": 7}}),
                ActionKind::Execute,
            ),
            &[input("note", "ok")],
        )
        .unwrap();
        let invoke = execute.invoke.unwrap();
        assert_eq!(invoke["name"], "adaptiveCard/action");
        assert_eq!(invoke["value"]["trigger"], "manual");
        assert_eq!(invoke["value"]["action"]["verb"], "approve");
        assert_eq!(
            invoke["value"]["action"]["data"],
            json!({"id": 7, "note": "ok"})
        );
    }
    #[test]
    fn execute_answers_replace_the_card_or_add_a_footer() {
        let replaced = execute_result(&json!({"statusCode": 200, "type": ADAPTIVE,
            "value": {"type": "AdaptiveCard", "body": [{"type": "TextBlock", "text": "Approuvé"}]}}));
        assert!(replaced.card.is_some());
        let footer = execute_result(
            &json!({"statusCode": 200, "type": ACTIVITY_MESSAGE, "value": " Merci "}),
        );
        assert_eq!(footer.footer.as_deref(), Some("Merci"));
        let error = execute_result(
            &json!({"statusCode": 500, "type": "application/vnd.microsoft.error",
            "value": {"message": "boom"}}),
        );
        assert!(error.footer.unwrap().contains("boom"));
    }
}
