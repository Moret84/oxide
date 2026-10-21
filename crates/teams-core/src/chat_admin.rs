//! Managing chats: a group chat's name and people, leaving it, and the list's pinned/hidden/unread marks.
use super::view::PersonView;
use super::*;
use teams_i18n::{t, tf};

const DAY_MS: i64 = 86_400_000;
/// `chatService.maxPinnedChats`.
const MAX_PINNED: usize = 15;

/// How much of the chat the people added can read, as the "Add people" dialog offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ShareHistory {
    #[default]
    None,
    /// The last `ChatAdminView.adding.days` days.
    Days,
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct AddPeopleView {
    pub query: String,
    pub results: Vec<PersonView>,
    pub chosen: Vec<PersonView>,
    pub searching: bool,
    pub searched: bool,
    pub history: ShareHistory,
    pub days: u32,
    pub adding: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ChatAdminView {
    /// The "Add people" dialog of the open chat.
    pub adding: Option<AddPeopleView>,
    /// A rename, removal or departure is under way.
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum ChatAdminAction {
    /// Names the open group chat; an empty title removes the name.
    Rename {
        title: String,
    },
    StartAdding,
    CancelAdding,
    AddQuery {
        query: String,
    },
    AddSearch,
    Choose {
        mri: String,
    },
    Unchoose {
        mri: String,
    },
    SetHistory {
        history: ShareHistory,
        days: u32,
    },
    ConfirmAdd,
    RemoveMember {
        mri: String,
    },
    Leave {
        chat_id: String,
    },
    /// Pins a chat at the top of the list, or unpins it.
    SetPinned {
        chat_id: String,
        pinned: bool,
    },
    /// Hides a chat until a new message arrives in it.
    Hide {
        chat_id: String,
    },
    Unhide {
        chat_id: String,
    },
    MarkUnread {
        chat_id: String,
    },
    MarkRead {
        chat_id: String,
    },
    DismissError,
}

#[derive(Debug, Clone)]
pub(crate) enum ChatAdminMsg {
    Action(ChatAdminAction),
    PeopleFound(u64, String, Result<Vec<Person>, String>),
    /// A change to `chat` was applied, or refused.
    Done(u64, String, Change, Result<(), String>),
}
#[derive(Debug, Clone)]
pub(crate) enum Change {
    Renamed(String),
    Added,
    Removed(String),
    Left,
    /// The chat list mark that was set: undone locally when the service refuses it.
    Pinned(bool),
    Hidden(Option<i64>),
    Unread(bool),
}

#[derive(Default)]
pub(crate) struct ChatAdminState {
    pub view: ChatAdminView,
    adding: Option<Adding>,
    busy: u32,
}
#[derive(Default)]
struct Adding {
    query: String,
    results: Vec<Person>,
    chosen: Vec<Person>,
    searching: bool,
    searched: bool,
    history: ShareHistory,
    days: u32,
    adding: bool,
}

impl App {
    /// The open chat when it is a group chat whose people and name can change.
    pub(crate) fn managed_chat(&self) -> Option<&Conversation> {
        let Some(Selection::Chat(id)) = &*self.selected else {
            return None;
        };
        self.chats
            .iter()
            .find(|c| &c.id == id)
            .filter(|c| c.id != NOTES && c.is_group() && !c.is_disabled())
    }
    pub(crate) fn update_chat_admin(&mut self, msg: ChatAdminMsg) -> Task<Msg> {
        let generation = self.generation;
        let task = match msg {
            ChatAdminMsg::PeopleFound(g, _, _) | ChatAdminMsg::Done(g, ..) if g != generation => {
                Task::none()
            }
            ChatAdminMsg::Action(action) => self.chat_admin_action(action),
            ChatAdminMsg::PeopleFound(_, query, result) => {
                let admin = &mut *self.chat_admin;
                if let Some(adding) = &mut admin.adding {
                    adding.searching = false;
                    if adding.query.trim() == query {
                        adding.searched = true;
                        match result {
                            Ok(people) => adding.results = people,
                            Err(e) => admin.view.error = Some(e),
                        }
                    }
                }
                Task::none()
            }
            ChatAdminMsg::Done(_, chat, change, result) => {
                self.chat_admin.busy = self.chat_admin.busy.saturating_sub(1);
                match result {
                    Ok(()) => self.chat_changed(&chat, change),
                    Err(e) => {
                        let error = match change {
                            Change::Renamed(_) => tf!("core.chat_admin.rename_failed", error = e),
                            Change::Added => {
                                if let Some(adding) = &mut self.chat_admin.adding {
                                    adding.adding = false;
                                }
                                tf!("core.chat_admin.add_failed", error = e)
                            }
                            Change::Removed(_) => tf!("core.chat_admin.remove_failed", error = e),
                            Change::Left => tf!("core.chat_admin.leave_failed", error = e),
                            Change::Pinned(pinned) => {
                                self.set_favorite(&chat, !pinned);
                                tf!("core.chat_admin.pin_failed", error = e)
                            }
                            Change::Hidden(time) => {
                                if time.is_some() {
                                    self.hidden_chats.remove(&chat);
                                }
                                tf!("core.chat_admin.hide_failed", error = e)
                            }
                            Change::Unread(unread) => {
                                if let Some(c) = self.chats.iter_mut().find(|c| c.id == chat) {
                                    c.is_read = Some(unread);
                                }
                                tf!("core.chat_admin.unread_failed", error = e)
                            }
                        };
                        self.chat_admin.view.error = Some(error);
                        Task::none()
                    }
                }
            }
        };
        self.refresh_chat_admin_view();
        task
    }
    fn chat_admin_action(&mut self, action: ChatAdminAction) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let generation = self.generation;
        let open = self.managed_chat().map(|c| c.id.clone());
        let done = |chat, change| done(generation, chat, change);
        match action {
            ChatAdminAction::DismissError => {
                self.chat_admin.view.error = None;
                Task::none()
            }
            ChatAdminAction::Rename { title } => {
                let Some(chat) = open else {
                    return Task::none();
                };
                let title = title.trim().to_owned();
                self.chat_admin.busy += 1;
                let (id, value) = (chat.clone(), title.clone());
                Task::perform(
                    async move {
                        client
                            .set_thread_property(&id, "topic", serde_json::json!(value))
                            .await
                            .map(drop)
                            .map_err(|e| e.to_string())
                    },
                    done(chat, Change::Renamed(title)),
                )
            }
            ChatAdminAction::StartAdding => {
                if open.is_some() {
                    self.chat_admin.adding = Some(Adding {
                        days: 1,
                        ..Default::default()
                    });
                }
                Task::none()
            }
            ChatAdminAction::CancelAdding => {
                self.chat_admin.adding = None;
                Task::none()
            }
            ChatAdminAction::AddQuery { query } => {
                if let Some(adding) = &mut self.chat_admin.adding {
                    adding.query = query;
                    adding.results.clear();
                    adding.searched = false;
                }
                Task::none()
            }
            ChatAdminAction::AddSearch => {
                let Some(adding) = &mut self.chat_admin.adding else {
                    return Task::none();
                };
                let query = adding.query.trim().to_owned();
                if query.chars().count() < 2 || adding.searching {
                    return Task::none();
                }
                adding.searching = true;
                Task::perform(
                    async move {
                        let r = client
                            .search_people(&query)
                            .await
                            .map_err(|e| e.to_string());
                        (query, r)
                    },
                    move |(q, r)| Msg::ChatAdmin(ChatAdminMsg::PeopleFound(generation, q, r)),
                )
            }
            ChatAdminAction::Choose { mri } => {
                let members: HashSet<String> = self
                    .managed_chat()
                    .map(|c| c.members.iter().map(|m| m.id.to_lowercase()).collect())
                    .unwrap_or_default();
                if let Some(adding) = &mut self.chat_admin.adding
                    && let Some(person) = adding.results.iter().find(|p| p.mri == mri).cloned()
                    && !members.contains(&mri.to_lowercase())
                    && !adding.chosen.iter().any(|p| p.mri == mri)
                {
                    adding.chosen.push(person);
                    adding.query.clear();
                    adding.results.clear();
                    adding.searched = false;
                }
                Task::none()
            }
            ChatAdminAction::Unchoose { mri } => {
                if let Some(adding) = &mut self.chat_admin.adding {
                    adding.chosen.retain(|p| p.mri != mri);
                }
                Task::none()
            }
            ChatAdminAction::SetHistory { history, days } => {
                if let Some(adding) = &mut self.chat_admin.adding {
                    adding.history = history;
                    adding.days = days.clamp(1, 365);
                }
                Task::none()
            }
            ChatAdminAction::ConfirmAdd => {
                let (Some(chat), Some(adding)) = (open, &mut self.chat_admin.adding) else {
                    return Task::none();
                };
                if adding.chosen.is_empty() || adding.adding {
                    return Task::none();
                }
                adding.adding = true;
                let share = share_history_time(
                    adding.history,
                    adding.days,
                    chrono::Utc::now().timestamp_millis(),
                );
                let mris: Vec<String> = adding.chosen.iter().map(|p| p.mri.clone()).collect();
                self.chat_admin.busy += 1;
                let id = chat.clone();
                Task::perform(
                    async move {
                        client
                            .add_chat_members(&id, &mris, share)
                            .await
                            .map(drop)
                            .map_err(|e| e.to_string())
                    },
                    done(chat, Change::Added),
                )
            }
            ChatAdminAction::RemoveMember { mri } => {
                let Some(chat) = open else {
                    return Task::none();
                };
                if mri.eq_ignore_ascii_case(&self.me) {
                    return self.chat_admin_action(ChatAdminAction::Leave { chat_id: chat });
                }
                self.chat_admin.busy += 1;
                let (id, who) = (chat.clone(), mri.clone());
                Task::perform(
                    async move {
                        client
                            .remove_member(&id, &who)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    done(chat, Change::Removed(mri)),
                )
            }
            ChatAdminAction::SetPinned { chat_id, pinned } => {
                if self.is_favorite_id(&chat_id) == pinned {
                    return Task::none();
                }
                if pinned && self.favorite_count() >= MAX_PINNED {
                    self.chat_admin.view.error =
                        Some(tf!("core.chat_admin.pin_limit", count = MAX_PINNED));
                    return Task::none();
                }
                self.set_favorite(&chat_id, pinned);
                self.chat_admin.busy += 1;
                let folder = self.favorites_folder.clone();
                let order = self.favorite_order();
                let id = chat_id.clone();
                Task::perform(
                    async move {
                        match folder {
                            // The new chat list keeps pins in the CSA "Favorites" folder.
                            Some((folder, version)) => {
                                client
                                    .update_folder_item(version, &folder, &id, pinned)
                                    .await
                            }
                            // The classic one in the `favorites` user property: {conversationId: order} as a JSON string.
                            None => {
                                let map: serde_json::Map<_, _> = order
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, id)| (id, serde_json::json!(i + 1)))
                                    .collect();
                                client
                                    .set_user_property(
                                        "favorites",
                                        serde_json::json!(
                                            serde_json::Value::Object(map).to_string()
                                        ),
                                    )
                                    .await
                            }
                        }
                        .map_err(|e| e.to_string())
                    },
                    done(chat_id, Change::Pinned(pinned)),
                )
            }
            ChatAdminAction::Hide { chat_id } => self.set_hidden(client, chat_id, true),
            ChatAdminAction::Unhide { chat_id } => self.set_hidden(client, chat_id, false),
            ChatAdminAction::MarkUnread { chat_id } => {
                let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else {
                    return Task::none();
                };
                let Some(last) = chat.last_message.clone() else {
                    return Task::none();
                };
                chat.is_read = Some(false);
                let bookmark = unread_bookmark(&last, chrono::Utc::now().timestamp_millis());
                chat.properties.insert(
                    "consumptionHorizonBookmark".into(),
                    serde_json::json!(bookmark),
                );
                self.kept_unread.insert(chat_id.clone());
                self.chat_admin.busy += 1;
                let id = chat_id.clone();
                Task::perform(
                    async move {
                        client
                            .set_conversation_property(
                                &id,
                                "consumptionHorizonBookmark",
                                serde_json::json!(bookmark),
                            )
                            .await
                            .map(drop)
                            .map_err(|e| e.to_string())
                    },
                    done(chat_id, Change::Unread(false)),
                )
            }
            ChatAdminAction::MarkRead { chat_id } => {
                self.kept_unread.remove(&chat_id);
                let last = self
                    .chats
                    .iter()
                    .find(|c| c.id == chat_id)
                    .and_then(|c| c.last_message.as_ref())
                    .map(|m| m.id.clone());
                match last {
                    Some(id) => Task::done(Msg::MarkRead(Selection::Chat(chat_id), id)),
                    None => Task::none(),
                }
            }
            ChatAdminAction::Leave { chat_id } => {
                let leavable = self
                    .chats
                    .iter()
                    .any(|c| c.id == chat_id && c.id != NOTES && c.is_group());
                if !leavable {
                    return Task::none();
                }
                self.chat_admin.busy += 1;
                let (id, me) = (chat_id.clone(), self.me.clone());
                Task::perform(
                    async move {
                        client
                            .remove_member(&id, &me)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    done(chat_id, Change::Left),
                )
            }
        }
    }
    fn set_hidden(&mut self, client: Client, chat_id: String, hide: bool) -> Task<Msg> {
        let generation = self.generation;
        let now = chrono::Utc::now().timestamp_millis();
        let time = hide.then_some(now);
        match time {
            Some(t) => {
                self.hidden_chats.insert(chat_id.clone(), t);
            }
            None => {
                self.hidden_chats.remove(&chat_id);
            }
        }
        self.chat_admin.busy += 1;
        let id = chat_id.clone();
        let task = Task::perform(
            async move {
                // `unpinnedTime` (a number) then `historyHiddenTime` (a string); unhiding clears both, as the new chat list does.
                let hidden = time.map_or(serde_json::Value::Null, |t| serde_json::json!(t));
                let history = time.map_or(serde_json::Value::Null, |t| {
                    serde_json::json!(t.to_string())
                });
                client
                    .set_conversation_property(&id, "unpinnedTime", hidden)
                    .await
                    .map_err(|e| e.to_string())?;
                client
                    .set_conversation_property(&id, "historyHiddenTime", history)
                    .await
                    .map(drop)
                    .map_err(|e| e.to_string())
            },
            done(generation, chat_id.clone(), Change::Hidden(time)),
        );
        if hide && self.selected.as_ref().is_some_and(|s| s.id() == chat_id) {
            let next = self
                .chats
                .iter()
                .find(|c| c.id != chat_id && !self.is_hidden(c))
                .map(|c| c.id.clone());
            if let Some(next) = next {
                return Task::batch([task, Task::done(Msg::Select(Selection::Chat(next)))]);
            }
        }
        task
    }
    /// Applies an accepted change locally, then reads the chat back from the service.
    fn chat_changed(&mut self, chat: &str, change: Change) -> Task<Msg> {
        match change {
            Change::Renamed(title) => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.id == chat) {
                    for map in [&mut c.thread_properties, &mut c.properties] {
                        map.remove("title");
                        map.remove("displayName");
                    }
                    c.extra.remove("title");
                    c.thread_properties
                        .insert("topic".into(), serde_json::json!(title));
                    c.properties.remove("topic");
                    c.extra.remove("topic");
                }
                *self.notice = Some(if title.is_empty() {
                    t("core.chat_admin.name_removed").into()
                } else {
                    tf!("core.chat_admin.renamed", title = title)
                });
            }
            Change::Added => {
                let added = self
                    .chat_admin
                    .adding
                    .take()
                    .map(|a| a.chosen)
                    .unwrap_or_default();
                if let Some(c) = self.chats.iter_mut().find(|c| c.id == chat) {
                    for person in &added {
                        if !c
                            .members
                            .iter()
                            .any(|m| m.id.eq_ignore_ascii_case(&person.mri))
                        {
                            let mut member = teams_chat::Member {
                                id: person.mri.clone(),
                                role: Some("Admin".into()),
                                ..Default::default()
                            };
                            member
                                .extra
                                .insert("friendlyName".into(), serde_json::json!(person.name()));
                            c.members.push(member);
                        }
                    }
                }
                Arc::make_mut(&mut self.profiles)
                    .extend(added.into_iter().map(|p| (p.mri.clone(), p)));
            }
            Change::Removed(mri) => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.id == chat) {
                    c.members.retain(|m| !m.id.eq_ignore_ascii_case(&mri));
                }
            }
            Change::Pinned(_) | Change::Hidden(_) | Change::Unread(_) => return Task::none(),
            Change::Left => {
                self.chats.retain(|c| c.id != chat);
                self.details.remove(chat);
                *self.notice = Some(t("core.chat_admin.left").into());
                if self.selected.as_ref().is_some_and(|s| s.id() == chat) {
                    *self.selected = None;
                    if let Some(next) = self.chats.first().map(|c| c.id.clone()) {
                        return Task::done(Msg::Select(Selection::Chat(next)));
                    }
                }
                return Task::none();
            }
        }
        // The roster and name come back from the thread: drop what was resolved.
        self.details.remove(chat);
        self.resolved_ids.remove(chat);
        self.resolve_chats()
    }
    pub(crate) fn refresh_chat_admin_view(&mut self) {
        let admin = &mut *self.chat_admin;
        let view = ChatAdminView {
            adding: admin.adding.as_ref().map(|a| AddPeopleView {
                query: a.query.clone(),
                results: a.results.iter().map(person_view).collect(),
                chosen: a.chosen.iter().map(person_view).collect(),
                searching: a.searching,
                searched: a.searched,
                history: a.history,
                days: a.days,
                adding: a.adding,
            }),
            busy: admin.busy > 0,
            error: admin.view.error.clone(),
        };
        if admin.view != view {
            admin.view = view;
        }
    }
}
fn done(
    generation: u64,
    chat: String,
    change: Change,
) -> impl FnOnce(Result<(), String>) -> Msg + Send + 'static {
    move |r| Msg::ChatAdmin(ChatAdminMsg::Done(generation, chat, change, r))
}
/// "Mark as unread" `consumptionHorizonBookmark`, just before the last message: `{oat - 1};{now};{clientmessageid}`.
fn unread_bookmark(last: &Message, now_ms: i64) -> String {
    let oat = super::view::arrival_ms(last).map_or(0, |t| t - 1);
    format!(
        "{oat};{now_ms};{}",
        last.clientmessageid
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("0")
    )
}
impl App {
    pub(crate) fn is_favorite_id(&self, id: &str) -> bool {
        match &*self.favorites {
            Some(ids) => ids.contains(id),
            None => self
                .chats
                .iter()
                .any(|c| c.id == id && c.is_sticky == Some(true)),
        }
    }
    fn favorite_count(&self) -> usize {
        self.chats
            .iter()
            .filter(|c| self.is_favorite_id(&c.id))
            .count()
    }
    /// Pinned chats in their list order, for the `favorites` property.
    fn favorite_order(&self) -> Vec<String> {
        self.pinned_order
            .iter()
            .filter(|id| self.is_favorite_id(id))
            .cloned()
            .collect()
    }
    fn set_favorite(&mut self, id: &str, pinned: bool) {
        let mut ids = self.favorites.clone().unwrap_or_else(|| {
            self.chats
                .iter()
                .filter(|c| c.is_sticky == Some(true))
                .map(|c| c.id.clone())
                .collect()
        });
        if pinned {
            ids.insert(id.to_owned());
            if !self.pinned_order.iter().any(|p| p == id) {
                self.pinned_order.push(id.to_owned());
            }
        } else {
            ids.remove(id);
            self.pinned_order.retain(|p| p != id);
        }
        *self.favorites = Some(ids);
    }
    /// Hidden from this app since the last message: shown again when a newer one arrives.
    pub(crate) fn is_hidden(&self, c: &Conversation) -> bool {
        self.hidden_chats
            .get(&c.id)
            .is_some_and(|at| conversations::last_message_time(c).is_none_or(|t| t <= *at))
    }
}
/// A "Mark as unread" bookmark is set (CSA `userConsumptionHorizon` or chat service `consumptionHorizonBookmark`).
pub(crate) fn has_unread_bookmark(c: &Conversation) -> bool {
    let set = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => !s.is_empty() && !s.starts_with("0;"),
        serde_json::Value::Object(o) => o.get("originalArrivalTime").is_some_and(|t| {
            t.as_i64().is_some_and(|t| t > 0)
                || t.as_str().is_some_and(|t| !t.is_empty() && t != "0")
        }),
        _ => false,
    };
    c.properties
        .get("consumptionHorizonBookmark")
        .is_some_and(set)
        || c.extra.get("userConsumptionHorizon").is_some_and(set)
}
/// Pins of the `favorites` user property, a JSON string `{conversationId: order}`.
pub(crate) fn favorite_property(properties: &serde_json::Value) -> Option<Vec<String>> {
    let raw = properties.get("favorites")?;
    let map: serde_json::Map<String, serde_json::Value> = match raw {
        serde_json::Value::String(s) => serde_json::from_str(s).ok()?,
        serde_json::Value::Object(o) => o.clone(),
        _ => return None,
    };
    let mut pins: Vec<(i64, String)> = map
        .into_iter()
        .map(|(id, order)| (order.as_i64().unwrap_or(i64::MAX), id))
        .collect();
    pins.sort();
    Some(pins.into_iter().map(|(_, id)| id).collect())
}
fn person_view(p: &Person) -> PersonView {
    PersonView {
        mri: p.mri.clone(),
        name: p.name().to_owned(),
        email: p.email.clone(),
    }
}
/// `shareHistoryTime` of the members added: -1 none, 0 everything, else when it starts.
fn share_history_time(history: ShareHistory, days: u32, now_ms: i64) -> i64 {
    match history {
        ShareHistory::None => -1,
        ShareHistory::All => 0,
        ShareHistory::Days => now_ms - i64::from(days.max(1)) * DAY_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unread_bookmarks_sit_just_before_the_last_message() {
        let last = Message {
            originalarrivaltime: Some("2026-10-21T10:00:00.000Z".into()),
            clientmessageid: Some("123".into()),
            ..Default::default()
        };
        assert_eq!(
            unread_bookmark(&last, 42),
            format!("{};42;123", 1_792_576_800_000i64 - 1)
        );
    }
    #[test]
    fn history_choices_map_to_the_web_client_values() {
        assert_eq!(share_history_time(ShareHistory::None, 3, 1_000_000_000), -1);
        assert_eq!(share_history_time(ShareHistory::All, 3, 1_000_000_000), 0);
        assert_eq!(
            share_history_time(ShareHistory::Days, 2, 1_000_000_000),
            1_000_000_000 - 2 * DAY_MS
        );
    }
}
