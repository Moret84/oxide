//! The compose box's message extensions: the apps flyout, their searches and forms, and the cards they add to the draft.
use super::view::{CardInputValue, CardView};
use super::*;
use crate::cards::{self, Card};
use serde_json::{Value, json};
use teams_chat::{
    ExtensionAnswer, ExtensionCommand, ExtensionItem, ExtensionTarget, MessageExtension, TaskModule,
};
use teams_i18n::{t, tf};

/// Left out of the flyout: Forms (its polls) and Praise. GIFs and stickers are not apps but the picker's own.
const HIDDEN_APPS: [&str; 2] = [
    // Forms, also in the web client's `tflMEFlyoutBlockedList`.
    "81fef3a6-72aa-4648-a763-de824aeafb7d",
    // Praise, the `send_praise` intent's `createKudos` command.
    "d832a33f-28c2-4969-8ad0-4fee681dc5b4",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionCommandView {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Opens a form rather than a search.
    pub action: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionAppView {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub commands: Vec<ExtensionCommandView>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionResultView {
    pub index: u32,
    pub title: String,
    pub text: String,
    pub image: Option<String>,
}
/// A page the bot wants opened: sign-in or settings, which happen in the browser.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionLinkView {
    pub url: String,
    pub title: String,
}
/// An action command's form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionFormView {
    pub title: String,
    pub card: Option<CardView>,
    /// A web form only Teams can host, offered in the browser.
    pub url: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ExtensionsView {
    pub apps_loading: bool,
    pub apps: Vec<ExtensionAppView>,
    pub apps_error: Option<String>,
    /// Why the list is empty: installed apps counted and the answers' keys, to report.
    pub apps_note: Option<String>,
    pub app_id: Option<String>,
    pub command_id: Option<String>,
    pub query: String,
    pub loading: bool,
    pub results: Vec<ExtensionResultView>,
    /// Results as tiles rather than a list.
    pub grid: bool,
    /// The bot's text, a hint, or why the search failed.
    pub message: Option<String>,
    pub link: Option<ExtensionLinkView>,
    pub form: Option<ExtensionFormView>,
    /// A picked result or a form is being turned into a card.
    pub inserting: bool,
    /// Bumped when a card went into the draft, so the flyout closes.
    pub inserted: u32,
}
/// A card waiting in the draft, from a message extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ComposerCardView {
    pub index: u32,
    pub app_name: String,
    pub card: CardView,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ExtensionsAction {
    /// The flyout opened: list the apps once.
    Load,
    Choose {
        app_id: String,
        command_id: String,
    },
    Search {
        query: String,
    },
    Pick {
        index: u32,
    },
    /// A button of the open form, with what was typed in.
    FormAction {
        action: u32,
        inputs: Vec<CardInputValue>,
    },
    CloseForm,
    /// Back to the list of apps.
    Back,
    RemoveCard {
        index: u32,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum ExtensionsMsg {
    Action(ExtensionsAction),
    Apps(u64, Result<teams_chat::ExtensionsLookup, String>),
    /// The answer to a search (by its sequence number), a pick or a form.
    Answer(u64, u64, Step, Result<ExtensionAnswer, String>),
}
/// What an invoke was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Query,
    Select,
    Form,
}

/// A card for the draft, and the app that made it.
#[derive(Debug, Clone)]
pub(crate) struct PendingCard {
    pub attachment: Value,
    pub app_id: String,
    pub app_name: String,
    pub app_icon: Option<String>,
    pub card: Card,
}

/// As the web client's `MessageExtensionCardSerializer`: an inline placeholder per card in the HTML,
/// and `properties.cards` with each card's `content` as an object.
pub(crate) fn cards_markup(cards: &[PendingCard]) -> (String, String) {
    let mut html = String::new();
    let mut list = Vec::new();
    for pending in cards {
        let id = uuid::Uuid::new_v4().to_string();
        html.push_str(&format!(
            "<p><span class=\"inline-compose-card\" itemid=\"{id}\" itemscope=\"\" itemtype=\"http://schema.skype.com/InputExtension\"><span content=\"{id}\" itemprop=\"cardId\"></span></span></p>"
        ));
        let mut content = pending
            .attachment
            .get("content")
            .cloned()
            .unwrap_or(Value::Null);
        if let Some(s) = content.as_str() {
            content = serde_json::from_str(s).unwrap_or(Value::Null);
        }
        let mut entry = json!({
            "appId": pending.app_id,
            "appName": pending.app_name,
            "contentType": pending.attachment.get("contentType").cloned().unwrap_or(Value::Null),
            "cardClientId": id,
            "content": content,
        });
        if let Some(icon) = &pending.app_icon {
            entry["appIcon"] = json!(icon);
        }
        list.push(entry);
    }
    (html, Value::Array(list).to_string())
}

#[derive(Default)]
pub(crate) struct ExtensionsState {
    pub view: ExtensionsView,
    apps: Option<Vec<MessageExtension>>,
    /// The group chat whose apps `apps` include.
    apps_for: Option<String>,
    items: Vec<ExtensionItem>,
    form: Option<(TaskModule, Option<Card>)>,
    /// Numbers searches, so a slow answer never replaces a newer one.
    seq: u64,
    /// Cards waiting in each draft, by draft key.
    pub pending: HashMap<String, Vec<PendingCard>>,
    /// The cards of the message being sent, with their draft key.
    pub sending: Option<(String, Vec<PendingCard>)>,
}

/// The `type: AdaptiveCard` form Teams draws from an action command's manifest parameters.
fn parameters_card(command: &ExtensionCommand) -> Value {
    let body: Vec<Value> = command
        .parameters
        .iter()
        .map(|p| {
            let mut input = match p.input_type.as_str() {
                "textarea" => json!({"type": "Input.Text", "isMultiline": true}),
                "number" => json!({"type": "Input.Number"}),
                "date" => json!({"type": "Input.Date"}),
                "time" => json!({"type": "Input.Time"}),
                "toggle" => json!({"type": "Input.Toggle", "title": p.title, "valueOn": "true", "valueOff": "false"}),
                "choiceset" => json!({"type": "Input.ChoiceSet", "choices": p.choices.iter()
                    .map(|(title, value)| json!({"title": title, "value": value})).collect::<Vec<_>>()}),
                _ => json!({"type": "Input.Text"}),
            };
            input["id"] = json!(p.name);
            input["label"] = json!(p.title);
            if !p.description.is_empty() {
                input["placeholder"] = json!(p.description);
            }
            input
        })
        .collect();
    json!({"type": "AdaptiveCard", "version": "1.4", "body": body,
        "actions": [{"type": "Action.Submit", "title": t("core.extensions.submit")}]})
}

impl App {
    pub(crate) fn update_extensions(&mut self, msg: ExtensionsMsg) -> Task<Msg> {
        let generation = self.generation;
        let task = match msg {
            ExtensionsMsg::Apps(g, ..) | ExtensionsMsg::Answer(g, ..) if g != generation => {
                Task::none()
            }
            ExtensionsMsg::Action(action) => self.extensions_action(action),
            ExtensionsMsg::Apps(_, result) => {
                let state = &mut *self.extensions;
                state.view.apps_loading = false;
                match result {
                    Ok(lookup) => {
                        state.view.apps_error = None;
                        state.view.apps_note = None;
                        let apps: Vec<MessageExtension> = lookup
                            .found
                            .into_iter()
                            .filter(|a| {
                                !HIDDEN_APPS
                                    .iter()
                                    .any(|id| id.eq_ignore_ascii_case(&a.app_id))
                            })
                            .collect();
                        if apps.is_empty() {
                            state.view.apps_note = Some(tf!(
                                "core.extensions.none_found",
                                count = lookup.entitled,
                                detail = lookup.shapes
                            ));
                        }
                        state.apps = Some(apps);
                    }
                    Err(e) => {
                        state.view.apps_error = Some(tf!("core.extensions.apps_failed", error = e))
                    }
                }
                Task::none()
            }
            ExtensionsMsg::Answer(_, seq, step, result) => self.extension_answer(seq, step, result),
        };
        self.refresh_extensions_view();
        task
    }
    fn extensions_action(&mut self, action: ExtensionsAction) -> Task<Msg> {
        let generation = self.generation;
        match action {
            ExtensionsAction::Load => {
                // A group chat adds its own apps to the user's.
                let chat = match &*self.selected {
                    Some(Selection::Chat(id)) if id.starts_with("19:") => Some(id.clone()),
                    _ => None,
                };
                let state = &mut *self.extensions;
                if state.view.apps_loading || (state.apps.is_some() && state.apps_for == chat) {
                    return Task::none();
                }
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                state.view.apps_loading = true;
                state.apps_for = chat.clone();
                let language = crate::emoji::locale();
                Task::perform(
                    async move {
                        client
                            .message_extensions(language, chat.as_deref())
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |r| Msg::Extensions(ExtensionsMsg::Apps(generation, r)),
                )
            }
            ExtensionsAction::Choose { app_id, command_id } => {
                let Some(command) = self.extension_command(&app_id, &command_id) else {
                    return Task::none();
                };
                let state = &mut *self.extensions;
                state.view.app_id = Some(app_id);
                state.view.command_id = Some(command_id);
                state.view.query.clear();
                state.view.message = None;
                state.view.link = None;
                state.items.clear();
                state.view.grid = false;
                state.form = None;
                if command.action {
                    if !command.fetch_task {
                        let card = parameters_card(&command);
                        state.form = Some((
                            TaskModule {
                                title: command.title.clone(),
                                card: Some(card.clone()),
                                url: None,
                            },
                            cards::adaptive_card(&card),
                        ));
                        return Task::none();
                    }
                    let value = json!({"commandId": command.id, "commandContext": "compose",
                        "context": {"theme": "default"}});
                    return self.extension_invoke("composeExtension/fetchTask", value, Step::Form);
                }
                if !command.initial_run {
                    state.view.message =
                        Some(command.description.clone()).filter(|d| !d.is_empty());
                    return Task::none();
                }
                self.extension_search(String::new())
            }
            ExtensionsAction::Search { query } => self.extension_search(query),
            ExtensionsAction::Pick { index } => {
                let Some(item) = self.extensions.items.get(index as usize).cloned() else {
                    return Task::none();
                };
                match item.tap {
                    Some(tap) => {
                        self.extension_invoke("composeExtension/selectItem", tap, Step::Select)
                    }
                    None => {
                        self.insert_extension_card(item.attachment);
                        Task::none()
                    }
                }
            }
            ExtensionsAction::FormAction { action, inputs } => {
                let Some((_, Some(card))) = &self.extensions.form else {
                    return Task::none();
                };
                let Some(button) = card.actions.get(action as usize) else {
                    return Task::none();
                };
                if let Some(url) = &button.url {
                    return Task::done(Msg::OpenUrl(url.clone()));
                }
                let missing = card.inputs().any(|input| {
                    input.required
                        && inputs
                            .iter()
                            .find(|v| v.id == input.id)
                            .is_none_or(|v| v.value.trim().is_empty())
                });
                if missing {
                    if let Some(form) = &mut self.extensions.view.form {
                        form.error = Some(t("core.card_actions.fill_required").into());
                    }
                    return Task::none();
                }
                let mut data = card_actions::merged_data(button.action.get("data"), &inputs);
                if let Value::Object(map) = &mut data {
                    map.remove("msteams");
                }
                let Some(command_id) = self.extensions.view.command_id.clone() else {
                    return Task::none();
                };
                let value = json!({"commandId": command_id, "commandContext": "compose",
                    "data": data, "context": {"theme": "default"}});
                self.extension_invoke("composeExtension/submitAction", value, Step::Form)
            }
            ExtensionsAction::CloseForm => {
                self.extensions.form = None;
                Task::none()
            }
            ExtensionsAction::Back => {
                let state = &mut *self.extensions;
                state.view.app_id = None;
                state.view.command_id = None;
                state.view.message = None;
                state.view.link = None;
                state.view.loading = false;
                state.view.inserting = false;
                state.items.clear();
                state.form = None;
                state.seq += 1;
                Task::none()
            }
            ExtensionsAction::RemoveCard { index } => {
                let key = self.draft_key();
                if let Some(list) = self.extensions.pending.get_mut(&key)
                    && (index as usize) < list.len()
                {
                    list.remove(index as usize);
                    if list.is_empty() {
                        self.extensions.pending.remove(&key);
                    }
                }
                Task::none()
            }
        }
    }
    fn chosen_extension(&self) -> Option<&MessageExtension> {
        let id = self.extensions.view.app_id.as_deref()?;
        self.extensions
            .apps
            .as_ref()?
            .iter()
            .find(|a| a.app_id == id)
    }
    fn extension_command(&self, app_id: &str, command_id: &str) -> Option<ExtensionCommand> {
        let app = self
            .extensions
            .apps
            .as_ref()?
            .iter()
            .find(|a| a.app_id == app_id)?;
        app.commands.iter().find(|c| c.id == command_id).cloned()
    }
    fn extension_search(&mut self, query: String) -> Task<Msg> {
        let (Some(app_id), Some(command_id)) = (
            self.extensions.view.app_id.clone(),
            self.extensions.view.command_id.clone(),
        ) else {
            return Task::none();
        };
        let Some(command) = self.extension_command(&app_id, &command_id) else {
            return Task::none();
        };
        let query = query.trim().to_owned();
        let state = &mut *self.extensions;
        state.view.query = query.clone();
        if query.is_empty() && !command.initial_run {
            state.seq += 1;
            state.items.clear();
            state.view.loading = false;
            state.view.message = Some(command.description.clone()).filter(|d| !d.is_empty());
            return Task::none();
        }
        let value = teams_chat::extensions::query_value(&command, &query, 0);
        self.extension_invoke("composeExtension/query", value, Step::Query)
    }
    /// The open conversation as invokes address it: a channel thread is `{channel};messageid={post}`.
    fn extension_target(&self) -> Option<ExtensionTarget> {
        let app = self.chosen_extension()?;
        let selection = self.selected.as_ref()?;
        let thread_id = match (selection, self.conv.reply()) {
            (Selection::Channel { channel, .. }, Some(post)) => {
                format!("{channel};messageid={post}")
            }
            _ => selection.id().to_owned(),
        };
        Some(ExtensionTarget {
            bot_id: app.bot_id.clone(),
            app_id: app.app_id.clone(),
            thread_id,
            display_name: self.display_name.to_string(),
        })
    }
    fn extension_invoke(&mut self, name: &'static str, value: Value, step: Step) -> Task<Msg> {
        let (Some(client), Some(target)) = (self.client.clone(), self.extension_target()) else {
            return Task::none();
        };
        // The web client skips invokes from the self chat.
        if target.thread_id == "48:notes" {
            self.extensions.view.message = Some(t("core.extensions.not_in_notes").into());
            return Task::none();
        }
        let state = &mut *self.extensions;
        state.seq += 1;
        let seq = state.seq;
        match step {
            Step::Query => {
                state.view.loading = true;
                state.view.message = None;
                state.view.link = None;
            }
            Step::Select => state.view.inserting = true,
            Step::Form => {
                if let Some(form) = &mut state.view.form {
                    form.busy = true;
                    form.error = None;
                } else {
                    state.view.inserting = true;
                }
            }
        }
        let generation = self.generation;
        Task::perform(
            async move {
                client
                    .extension_invoke(&target, name, value)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| Msg::Extensions(ExtensionsMsg::Answer(generation, seq, step, r)),
        )
    }
    fn extension_answer(
        &mut self,
        seq: u64,
        step: Step,
        result: Result<ExtensionAnswer, String>,
    ) -> Task<Msg> {
        let state = &mut *self.extensions;
        if seq != state.seq {
            return Task::none();
        }
        state.view.loading = false;
        state.view.inserting = false;
        let answer = match result {
            Ok(answer) => answer,
            Err(e) => {
                let text = tf!("core.extensions.invoke_failed", error = e);
                match (&mut state.view.form, step) {
                    (Some(form), Step::Form) => {
                        form.busy = false;
                        form.error = Some(text);
                    }
                    _ => state.view.message = Some(text),
                }
                return Task::none();
            }
        };
        match (step, answer) {
            (Step::Query, ExtensionAnswer::Results { items, grid }) => {
                state.view.message = items
                    .is_empty()
                    .then(|| t("core.extensions.no_results").to_owned());
                state.items = items;
                state.view.grid = grid;
            }
            (_, ExtensionAnswer::Results { items, .. }) => {
                state.form = None;
                match items.into_iter().next() {
                    Some(item) => self.insert_extension_card(item.attachment),
                    None => state.view.message = Some(t("core.extensions.no_card").into()),
                }
            }
            (_, ExtensionAnswer::Message(text)) => {
                if step == Step::Form {
                    state.form = None;
                }
                state.items.clear();
                state.view.message = Some(text).filter(|s| !s.is_empty());
            }
            (_, ExtensionAnswer::OpenUrl { url, title, config }) => {
                state.items.clear();
                state.view.message = Some(
                    if config {
                        t("core.extensions.needs_settings")
                    } else {
                        t("core.extensions.needs_sign_in")
                    }
                    .to_owned(),
                );
                state.view.link = Some(ExtensionLinkView {
                    url,
                    title: if title.is_empty() {
                        t("core.extensions.open").to_owned()
                    } else {
                        title
                    },
                });
            }
            (_, ExtensionAnswer::Task(task)) => {
                let card = task.card.as_ref().and_then(cards::adaptive_card);
                state.form = Some((task, card));
            }
            (_, ExtensionAnswer::Done) => {
                state.form = None;
                if step == Step::Form {
                    state.view.inserted += 1;
                }
            }
        }
        Task::none()
    }
    /// Puts a result's card in the draft, as Teams does before sending.
    fn insert_extension_card(&mut self, attachment: Value) {
        let Some(app) = self.chosen_extension().cloned() else {
            return;
        };
        let Some(card) = cards::attachment(&attachment) else {
            return;
        };
        let key = self.draft_key();
        let state = &mut *self.extensions;
        state.pending.entry(key).or_default().push(PendingCard {
            attachment,
            app_id: app.app_id,
            app_name: app.name,
            app_icon: app.icon,
            card,
        });
        state.form = None;
        state.view.inserted += 1;
    }
    pub(crate) fn has_extension_cards(&self) -> bool {
        self.extensions
            .pending
            .get(&self.draft_key())
            .is_some_and(|l| !l.is_empty())
    }
    /// The draft's cards, out of it while they are sent; [`Self::restore_extension_cards`] puts them back on failure.
    pub(crate) fn take_extension_cards(&mut self) -> Option<(String, Vec<PendingCard>)> {
        let key = self.draft_key();
        let cards = self.extensions.pending.remove(&key)?;
        Some((key, cards))
    }
    pub(crate) fn restore_extension_cards(&mut self, key: String, cards: Vec<PendingCard>) {
        let list = self.extensions.pending.entry(key).or_default();
        let added = std::mem::take(list);
        *list = cards;
        list.extend(added);
    }
    pub(crate) fn composer_cards(&self) -> Vec<ComposerCardView> {
        self.extensions
            .pending
            .get(&self.draft_key())
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, p)| {
                let card = self.card_view(&p.card)?;
                Some(ComposerCardView {
                    index: index as u32,
                    app_name: p.app_name.clone(),
                    card,
                })
            })
            .collect()
    }
    fn card_view(&self, card: &Card) -> Option<CardView> {
        match self.block_views(&[html::Block::Card(card.clone())]).pop()? {
            view::BlockView::Card { card } => Some(card),
            _ => None,
        }
    }
    fn refresh_extensions_view(&mut self) {
        let apps: Vec<ExtensionAppView> = self
            .extensions
            .apps
            .iter()
            .flatten()
            .map(|a| ExtensionAppView {
                id: a.app_id.clone(),
                name: a.name.clone(),
                icon: a.icon.clone(),
                commands: a
                    .commands
                    .iter()
                    .map(|c| ExtensionCommandView {
                        id: c.id.clone(),
                        title: c.title.clone(),
                        description: c.description.clone(),
                        action: c.action,
                    })
                    .collect(),
            })
            .collect();
        let results: Vec<ExtensionResultView> = self
            .extensions
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| ExtensionResultView {
                index: i as u32,
                title: item.title.clone(),
                text: item.text.clone(),
                image: item.image.clone(),
            })
            .collect();
        let form = self.extensions.form.as_ref().map(|(task, card)| {
            let previous = self.extensions.view.form.as_ref();
            ExtensionFormView {
                title: task.title.clone(),
                card: card.as_ref().and_then(|c| self.card_view(c)),
                url: task.url.clone(),
                busy: previous.is_some_and(|f| f.busy),
                error: previous.and_then(|f| f.error.clone()),
            }
        });
        let state = &mut *self.extensions;
        if state.view.apps != apps {
            state.view.apps = apps;
        }
        if state.view.results != results {
            state.view.results = results;
        }
        if state.view.form != form {
            state.view.form = form;
        }
    }
}
