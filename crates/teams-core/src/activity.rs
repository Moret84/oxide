//! Teams' notification stream keeps read items as history, independently of chats.
use super::*;
use serde_json::{Value, json};
use teams_i18n::{t, tf};
pub(crate) const FEED: &str = "48:notifications";
/// Items kept while the reader is at the top of the feed, where the newest are.
const MAX_ACTIVITIES: usize = 200;
#[derive(Default)]
pub(crate) struct ActivityState {
    pub messages: Vec<Message>,
    pub loading: bool,
    pub cursor: Option<String>,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub unread_only: bool,
    locally_read: HashSet<String>,
    /// Feed `consumptionHorizonBookmark` (epoch ms): older items are read whatever their `isread`, as in Teams.
    pub(crate) bookmark_ms: Option<i64>,
    bookmark_requested: bool,
    /// Calendar event found for a meeting notification without `eventId`, by activity id.
    meeting_events: HashMap<String, String>,
    /// Older items were asked for since the screen opened: the list is not trimmed.
    pub(crate) paged: bool,
}
impl ActivityState {
    /// "Mark all as read": every loaded item, and the ones a later reload brings back.
    pub(crate) fn mark_all_locally_read(&mut self, now_ms: i64) {
        for message in &mut self.messages {
            self.locally_read.insert(message.id.clone());
            set_read(message, true);
        }
        self.bookmark_ms = self.bookmark_ms.max(Some(now_ms));
    }
    pub(crate) fn is_unread(&self, message: &Message) -> bool {
        unread(message)
            && self
                .bookmark_ms
                .zip(activity_version(message))
                .is_none_or(|(bookmark, version)| bookmark < version)
    }
}
/// Version of an activity: its `version`, else its arrival time (epoch ms).
fn activity_version(message: &Message) -> Option<i64> {
    message
        .version
        .as_deref()
        .and_then(|v| v.parse().ok())
        .or_else(|| conversations::message_time(message))
}
/// `0;<epoch ms>;0` ("mark all as read"), or the latest of a `{activity,chat,channel}Timestamp` object or JSON string.
pub(crate) fn parse_bookmark(value: &Value) -> Option<i64> {
    let stamp = |v: &Value| match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|t| t.timestamp_millis())
        }),
        _ => None,
    };
    match value {
        Value::String(s) if s.contains(';') => s.split(';').nth(1).and_then(|t| t.parse().ok()),
        Value::String(s) => parse_bookmark(&serde_json::from_str::<Value>(s).ok()?),
        Value::Object(map) => ["activityTimestamp", "chatTimestamp", "channelTimestamp"]
            .iter()
            .filter_map(|k| map.get(*k).and_then(stamp))
            .max(),
        _ => None,
    }
    .filter(|t| *t > 0)
}
#[derive(Debug, Clone)]
pub(crate) enum ActivityMsg {
    Open(String),
    /// An item opened its conversation or meeting: the Activity screen stays.
    Shown,
    More,
    Loaded(u64, bool, Result<MessagesPage, String>),
    ReadMarked(u64, String, Result<(), String>),
    BookmarkLoaded(u64, Option<i64>),
    MeetingFound(
        u64,
        String,
        Result<Option<Box<teams_calendar::Event>>, String>,
    ),
}
/// Small icon on the actor's avatar, as in Teams' feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ActivityBadge {
    None,
    Mention,
    Reply,
    /// See `ActivityRow::reaction` for the emoji.
    Reaction,
    MissedCall,
    Voicemail,
    Meeting,
    App,
    Team,
    Urgent,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ActivityRow {
    pub id: String,
    pub actor_id: String,
    pub actor: String,
    /// Whole sentence: "Alice a réagi à votre message", "Appel manqué de Bob".
    pub title: String,
    /// Where or when: "Équipe › Canal", "3 nov., 09:00 - 17:00", "Appel Teams"…
    pub context: String,
    pub preview: String,
    pub time: String,
    /// "14:30" today, "13/10" before.
    pub short_time: String,
    pub badge: ActivityBadge,
    pub reaction: Option<String>,
    /// A meeting notification: opening it shows the meeting's details.
    pub meeting: bool,
    /// Its calendar event, once known.
    pub event_id: Option<String>,
    /// MRI to call back (missed calls).
    pub call_back: Option<String>,
    /// Chat for the "Conversation" action (missed calls, meeting chats).
    pub chat_id: Option<String>,
    pub unread: bool,
    pub selected: bool,
    pub has_target: bool,
}

/// How a feed item reads, from `properties.activity` (meeting invitations are server-worded `msGraph` items).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Described {
    pub title: String,
    pub context: Option<String>,
    pub badge: ActivityBadge,
    pub reaction: Option<String>,
    pub event_id: Option<String>,
    pub call_back: Option<String>,
    pub meeting: bool,
    /// UTC start and end of a meeting, from `<DateTimeRange>`.
    pub range: Option<(String, String)>,
}

/// `<DateTimeRange>` with `<StartDateTime>`/`<EndDateTime>`, the `location.Custom` of meeting notifications.
pub(crate) fn date_time_range(text: &str) -> Option<(String, String)> {
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&format!("</{name}>"))? + start;
        Some(text[start..end].trim().to_owned()).filter(|s| !s.is_empty())
    };
    Some((tag("StartDateTime")?, tag("EndDateTime")?))
}

/// `activityContext`, an object or a JSON string.
fn activity_context(data: &Value) -> Value {
    match data.get("activityContext") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(value) => value.clone(),
        None => Value::Null,
    }
}
/// `templateParameters`: a list of `{name, value}`, possibly as a JSON string.
fn template_parameters(context: &Value) -> Vec<(String, String)> {
    let value = match context.get("templateParameters") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(value) => value.clone(),
        None => return Vec::new(),
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                Some((field(item, "name"), field(item, "value"))).filter(|(n, _)| !n.is_empty())
            })
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}
/// `location.Custom`: free text the server already formatted (a meeting's time range).
fn custom_location(context: &Value) -> Option<String> {
    let location = match context.get("location") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({"Custom": s})),
        Some(value) => value.clone(),
        None => return None,
    };
    location
        .get("Custom")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
/// `eventId` of a `/l/meeting/details?eventId=…` tap action, or of the context itself.
fn meeting_event(context: &Value, data: &Value) -> Option<String> {
    for key in ["eventId", "calendarEventId"] {
        let id = field(context, key);
        if !id.is_empty() {
            return Some(id);
        }
    }
    let link = [
        field(context, "customTapAction"),
        field(data, "deeplinkUrl"),
    ]
    .into_iter()
    .find(|l| l.contains("eventId="))?;
    let absolute = if link.starts_with("http") {
        link
    } else {
        format!(
            "https://teams.microsoft.com{}",
            if link.starts_with('/') { "" } else { "/" }
        ) + &link
    };
    let url = Url::parse(&absolute).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == "eventId")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}
/// Verb of a meeting notification, guessed from the subtype: Teams' app manifest text is unavailable here.
fn meeting_verb(subtype: &str) -> &'static str {
    let s = subtype.to_lowercase();
    if s.contains("cancel") {
        t("core.activity.canceled")
    } else if s.contains("forward") {
        t("core.activity.forwarded_a_meeting")
    } else if s.contains("decline") {
        t("core.activity.declined")
    } else if s.contains("tentative") {
        t("core.activity.tentatively_accepted")
    } else if s.contains("accept") {
        t("core.activity.accepted")
    } else if s.contains("update") || s.contains("change") || s.contains("reschedul") {
        t("core.activity.updated")
    } else {
        t("core.activity.invited_you")
    }
}
/// Fills `{name}` placeholders from the template parameters, `{actor}` with the actor.
fn fill_template(text: &str, parameters: &[(String, String)], actor: &str) -> String {
    let mut text = text.replace("{actor}", actor);
    for (name, value) in parameters {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub(crate) fn describe(data: &Value, actor: &str, actor_id: &str) -> Described {
    let kind = field(data, "activityType");
    let subtype = field(data, "activitySubtype");
    let context = activity_context(data);
    let parameters = template_parameters(&context);
    let base = |title: String, badge: ActivityBadge| Described {
        title,
        context: None,
        badge,
        reaction: None,
        event_id: None,
        call_back: None,
        meeting: false,
        range: None,
    };
    let phone = actor_id.starts_with("4:") || actor_id.starts_with('+');
    match (kind.as_str(), subtype.as_str()) {
        ("call", "missedCall") | ("missedCall", _) => Described {
            context: Some(if phone {
                actor_id.trim_start_matches("4:").into()
            } else {
                t("core.activity.teams_call").into()
            }),
            call_back: Some(actor_id.to_owned()).filter(|id| !id.is_empty()),
            ..base(
                tf!("core.activity.missed_call_from", actor = actor),
                ActivityBadge::MissedCall,
            )
        },
        ("call", "voicemail") | ("voicemail", _) => Described {
            context: Some(t("core.activity.voicemail").into()),
            call_back: Some(actor_id.to_owned()).filter(|id| !id.is_empty()),
            ..base(
                tf!("core.activity.voicemail_from", actor = actor),
                ActivityBadge::Voicemail,
            )
        },
        ("call", "missedMeetingNudge") => Described {
            context: Some(tf!("core.activity.meeting_with", actor = actor)),
            ..base(
                tf!("core.activity.invited_you_to_join_a_meeting", actor = actor),
                ActivityBadge::Meeting,
            )
        },
        ("reaction" | "reactionInChat" | "like" | "likeInChat" | "reactionInCommunity", key) => {
            Described {
                reaction: Some(crate::emoji::reaction(if key.is_empty() { "like" } else { key }).0),
                ..base(
                    tf!("core.activity.reacted_to_your_message", actor = actor),
                    ActivityBadge::Reaction,
                )
            }
        }
        ("mention" | "mentionInChat" | "mentions" | "mentionInCommunity", sub) => {
            let target = match sub {
                "team" | "channel" | "tag" | "everyone" => parameters
                    .iter()
                    .find(|(n, _)| {
                        n.eq_ignore_ascii_case("mentionedEntity")
                            || n.eq_ignore_ascii_case("teamName")
                    })
                    .map(|(_, v)| v.clone()),
                _ => None,
            };
            let title = match (sub, target) {
                ("team" | "channel" | "tag", Some(target)) => {
                    tf!("core.activity.mentioned", actor = actor, target = target)
                }
                ("everyone", _) => tf!("core.activity.mentioned_everyone", actor = actor),
                _ => tf!("core.activity.mentioned_you", actor = actor),
            };
            base(title, ActivityBadge::Mention)
        }
        ("reply" | "replyToReply" | "replyInChat" | "communityReply", _) => base(
            tf!("core.activity.replied", actor = actor),
            ActivityBadge::Reply,
        ),
        ("follow", "channelReplyMessage") => base(
            tf!("core.activity.replied_in_a_followed_thread", actor = actor),
            ActivityBadge::Reply,
        ),
        ("priority", _) => base(
            tf!("core.activity.sent_you_an_urgent_message", actor = actor),
            ActivityBadge::Urgent,
        ),
        ("teamMembershipChange" | "addedToTeam" | "promotedToTeamAdmin", sub) => base(
            if sub.contains("promoted") || kind == "promotedToTeamAdmin" {
                tf!("core.activity.made_you_an_owner", actor = actor)
            } else {
                tf!("core.activity.added_you_to_a_team", actor = actor)
            },
            ActivityBadge::Team,
        ),
        ("chat", _) => base(
            tf!("core.activity.sent_a_message", actor = actor),
            ActivityBadge::None,
        ),
        _ => {
            let event_id = meeting_event(&context, data);
            let custom = custom_location(&context);
            let range = custom.as_deref().and_then(date_time_range);
            let meeting = event_id.is_some()
                || range.is_some()
                || kind == "calendar"
                || subtype.to_lowercase().contains("meeting")
                || subtype.to_lowercase().contains("event");
            let template = ["templateText", "title"]
                .into_iter()
                .map(|k| field(&context, k))
                .find(|t| !t.is_empty())
                .map(|t| fill_template(&t, &parameters, actor));
            let title = match template {
                Some(text) if text.contains(actor) => text,
                Some(text) => format!("{actor} {text}"),
                None if meeting => format!("{actor} {}", meeting_verb(&subtype)),
                None => format!("{actor} · notification"),
            };
            let context = match &range {
                Some((start, end)) => super::calendar::meeting_range(start, end, false),
                // Never show markup the server meant for a template.
                None => custom.filter(|c| !c.starts_with('<')),
            };
            Described {
                context,
                event_id,
                meeting,
                range,
                ..base(
                    title,
                    if meeting {
                        ActivityBadge::Meeting
                    } else {
                        ActivityBadge::App
                    },
                )
            }
        }
    }
}
pub(crate) fn metadata(message: &Message) -> Value {
    match message.properties.get("activity") {
        Some(Value::String(value)) => serde_json::from_str(value).unwrap_or(Value::Null),
        value => value.cloned().unwrap_or(Value::Null),
    }
}
pub(crate) fn field(data: &Value, key: &str) -> String {
    match data.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn set_read(message: &mut Message, read: bool) {
    message
        .properties
        .insert("isread".into(), json!(read.to_string()));
}
fn unread(message: &Message) -> bool {
    !message.properties.get("isread").is_some_and(|v| {
        v.as_bool() == Some(true) || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true"))
    })
}
pub(crate) fn label(kind: &str) -> &'static str {
    match kind {
        "mention" | "mentionInChat" | "mentionInCommunity" | "mentions" => {
            t("core.activity.mentioned_you_2")
        }
        "like" | "likeInChat" | "reaction" | "reactionInChat" | "reactionInCommunity" => {
            t("core.activity.reacted_to_a_message")
        }
        "reply" | "replyToReply" | "replyInChat" | "communityReply" => t("core.activity.replied_2"),
        "chat" => t("core.activity.sent_a_message_2"),
        "missedCall" => t("core.activity.missed_call"),
        "addedToTeam" | "teamMembershipChange" => t("core.activity.team_update"),
        "calendar" => t("core.activity.calendar_update"),
        _ => "notification",
    }
}
impl App {
    /// The team owning this channel (or team) thread.
    pub(crate) fn thread_team(&self, source: &str) -> Option<&Team> {
        self.teams
            .iter()
            .find(|t| t.id == source || t.channels.iter().any(|c| c.id == source))
    }
    /// "Équipe › Canal" for a known channel.
    pub(crate) fn channel_place(&self, channel: &str) -> Option<String> {
        self.teams.iter().find_map(|t| {
            t.channels
                .iter()
                .find(|c| c.id == channel)
                .map(|c| format!("{} › {}", t.display_name, c.display_name))
        })
    }
    /// "Équipe › Canal", "En conversation avec vous" for a 1:1 chat, or the group's title.
    fn activity_place(&self, source: &str) -> Option<String> {
        if source.is_empty() {
            return None;
        }
        if let Some(place) = self.channel_place(source) {
            return Some(place);
        }
        let chat = self.chats.iter().find(|c| c.id == source)?;
        if !chat.is_group() && chat.title().is_none() {
            Some(t("core.activity.in_a_chat_with_you").into())
        } else {
            Some(chat_title(chat, &self.me))
        }
    }
    /// The 1:1 chat with this person, for "Conversation" on a missed call.
    fn direct_chat_with(&self, mri: &str) -> Option<String> {
        let oid = mri.rsplit(':').next().filter(|s| !s.is_empty())?;
        self.chats
            .iter()
            .find(|c| {
                !c.is_group()
                    && (c.members.iter().any(|m| m.id.eq_ignore_ascii_case(mri))
                        || (c.id.contains(oid) && c.id.ends_with("@unq.gbl.spaces")))
            })
            .map(|c| c.id.clone())
    }
    pub(crate) fn activity_rows(&self) -> Vec<ActivityRow> {
        self.activity
            .messages
            .iter()
            .rev()
            .filter(|m| !m.is_deleted())
            .filter_map(|message| {
                let is_unread = self.activity.is_unread(message);
                if self.activity.unread_only && !is_unread {
                    return None;
                }
                let data = metadata(message);
                let mut actor_id = field(&data, "sourceUserId");
                if actor_id.starts_with("orgid:") {
                    actor_id = format!("8:{actor_id}");
                }
                let actor = field(&data, "sourceUserImDisplayName");
                let actor = if actor.trim().is_empty() {
                    self.known_name(&actor_id)
                        .unwrap_or_else(|| "Notification".into())
                } else {
                    actor
                };
                let described = describe(&data, &actor, &actor_id);
                let title = described.title;
                let source = field(&data, "sourceThreadId");
                let topic = field(&data, "sourceThreadTopic");
                let place = self.activity_place(&source);
                let context = described
                    .context
                    .or_else(|| {
                        described
                            .event_id
                            .as_deref()
                            .and_then(|id| self.calendar.event_range(id))
                    })
                    .or(place)
                    .unwrap_or(topic);
                let preview = field(&data, "messagePreview");
                let preview = html::to_text_cached(if preview.is_empty() {
                    &message.content
                } else {
                    &preview
                });
                if !self.search.trim().is_empty()
                    && !format!("{title} {context} {preview}")
                        .to_lowercase()
                        .contains(&self.search.trim().to_lowercase())
                {
                    return None;
                }
                let stamp = field(&data, "activityTimestamp");
                let stamp = if stamp.is_empty() {
                    message
                        .originalarrivaltime
                        .as_deref()
                        .or(message.composetime.as_deref())
                        .unwrap_or("")
                } else {
                    &stamp
                };
                let local = DateTime::parse_from_rfc3339(stamp).map(|d| d.with_timezone(&Local));
                let time = local
                    .map(|d| d.format(t("core.activity.m_d_h_m")).to_string())
                    .unwrap_or_default();
                let short_time = local
                    .map(|d| {
                        if d.date_naive() == Local::now().date_naive() {
                            d.format("%H:%M").to_string()
                        } else {
                            d.format(t("core.activity.m_d")).to_string()
                        }
                    })
                    .unwrap_or_default();
                // A meeting opens its details here; the rest opens its conversation.
                let event_id = described
                    .event_id
                    .clone()
                    .or_else(|| self.activity.meeting_events.get(&message.id).cloned());
                let has_target = !described.meeting
                    && !source.is_empty()
                    && (!source.contains("@thread.tacv2") || self.thread_team(&source).is_some());
                let chat_id = if !source.is_empty() && !source.contains("@thread.tacv2") {
                    Some(source.clone())
                } else {
                    described
                        .call_back
                        .as_deref()
                        .and_then(|mri| self.direct_chat_with(mri))
                };
                Some(ActivityRow {
                    id: message.id.clone(),
                    actor_id,
                    actor,
                    title,
                    context,
                    preview,
                    time,
                    short_time,
                    badge: described.badge,
                    reaction: described.reaction,
                    meeting: described.meeting,
                    event_id,
                    call_back: described.call_back,
                    chat_id,
                    unread: is_unread,
                    selected: self.activity.selected.as_ref() == Some(&message.id),
                    has_target,
                })
            })
            .collect()
    }
    pub(crate) fn load_activity(&mut self, append: bool) -> Task<Msg> {
        if self.activity.loading || (append && self.activity.cursor.is_none()) {
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.activity.loading = true;
        self.activity.error = None;
        let bookmark = self.load_activity_bookmark();
        let query = MessagesQuery {
            page_size: Some(50),
            start_time: Some(0),
            sync_state: if append {
                self.activity.cursor.clone()
            } else {
                None
            },
            ..Default::default()
        };
        let generation = self.generation;
        let load = Task::perform(
            async move {
                client.messages(FEED, &query).await.map_err(|_| {
                    t("core.activity.cannot_load_the_notifications_try_again").to_owned()
                })
            },
            move |r| Msg::Activity(ActivityMsg::Loaded(generation, append, r)),
        );
        Task::batch([bookmark, load])
    }
    /// Once per session: the bookmark another client may have moved ("mark all as read").
    fn load_activity_bookmark(&mut self) -> Task<Msg> {
        let Some(client) = self
            .client
            .clone()
            .filter(|_| !self.activity.bookmark_requested)
        else {
            return Task::none();
        };
        self.activity.bookmark_requested = true;
        let generation = self.generation;
        Task::perform(
            async move {
                client.conversation(FEED).await.ok().and_then(|c| {
                    c.properties
                        .get("consumptionHorizonBookmark")
                        .and_then(parse_bookmark)
                })
            },
            move |b| Msg::Activity(ActivityMsg::BookmarkLoaded(generation, b)),
        )
    }
    pub(crate) fn update_activity(&mut self, msg: ActivityMsg) -> Task<Msg> {
        match msg {
            ActivityMsg::Loaded(g, _, _)
            | ActivityMsg::ReadMarked(g, _, _)
            | ActivityMsg::BookmarkLoaded(g, _)
            | ActivityMsg::MeetingFound(g, _, _)
                if g != self.generation =>
            {
                Task::none()
            }
            ActivityMsg::MeetingFound(_, activity, found) => {
                if let Ok(Some(event)) = &found {
                    self.activity
                        .meeting_events
                        .insert(activity.clone(), event.object_id.clone());
                }
                // Only if that notification is still the one on screen.
                if self.activity.selected.as_deref() == Some(activity.as_str()) {
                    self.calendar
                        .show_found_meeting(found.map(|event| event.map(|e| *e)));
                }
                Task::none()
            }
            ActivityMsg::BookmarkLoaded(_, bookmark) => {
                // A later local "mark all as read" is not undone by an older server value.
                if bookmark.is_some() {
                    self.activity.bookmark_ms = self.activity.bookmark_ms.max(bookmark);
                }
                Task::none()
            }
            ActivityMsg::More => self.load_activity(true),
            ActivityMsg::Loaded(_, append, result) => {
                self.activity.loading = false;
                match result {
                    Ok(mut page) => {
                        let next_cursor = page.metadata.older_page_cursor();
                        if append || self.activity.messages.is_empty() {
                            self.activity.cursor = if page.messages.is_empty()
                                || (append && next_cursor == self.activity.cursor)
                            {
                                None
                            } else {
                                next_cursor.clone()
                            };
                        }
                        for message in &mut page.messages {
                            if self.activity.locally_read.contains(&message.id) {
                                set_read(message, true);
                            }
                        }
                        let activity = &mut *self.activity;
                        activity.paged |= append;
                        conversation::merge_messages(&mut activity.messages, page.messages);
                        // Only the newest are kept; "more" then asks again from the newest page.
                        if !append && !activity.paged && activity.messages.len() > MAX_ACTIVITIES {
                            let excess = activity.messages.len() - MAX_ACTIVITIES;
                            activity.messages.drain(..excess);
                            activity.cursor = next_cursor;
                        }
                    }
                    Err(error) => self.activity.error = Some(error),
                }
                Task::none()
            }
            ActivityMsg::ReadMarked(_, id, result) => {
                if result.is_err() {
                    self.activity.locally_read.remove(&id);
                    if let Some(m) = self.activity.messages.iter_mut().find(|m| m.id == id) {
                        set_read(m, false);
                    }
                    self.activity.error =
                        Some(t("core.activity.the_notification_could_not_be_marked").into());
                }
                Task::none()
            }
            ActivityMsg::Open(id) => {
                let Some(message) = self.activity.messages.iter().find(|m| m.id == id).cloned()
                else {
                    return Task::none();
                };
                self.activity.selected = Some(id.clone());
                let data = metadata(&message);
                let source = field(&data, "sourceThreadId");
                let actor = field(&data, "sourceUserImDisplayName");
                let described = describe(&data, &actor, &field(&data, "sourceUserId"));
                let event = described
                    .event_id
                    .clone()
                    .or_else(|| self.activity.meeting_events.get(&id).cloned());
                let selection = if source.is_empty() || described.meeting {
                    None
                } else if source.contains("@thread.tacv2") {
                    self.thread_team(&source)
                        .map(|t| Selection::Channel {
                            team: t.id.clone(),
                            channel: source,
                        })
                } else {
                    Some(Selection::Chat(source))
                };
                let select = match (event, described.range) {
                    // Meeting notifications show the event, as Teams' details pane does.
                    (Some(event), _) => {
                        Task::done(Msg::Calendar(super::calendar::CalendarMsg::Action(
                            super::calendar::CalendarAction::Select { id: event },
                        )))
                    }
                    (None, Some((start, end))) if described.meeting => {
                        let subject = html::to_text(&field(&data, "messagePreview"));
                        let (g, activity) = (self.generation, id.clone());
                        self.find_meeting(start, end, subject, move |found| {
                            let found = found.map(|event| event.map(Box::new));
                            Msg::Activity(ActivityMsg::MeetingFound(g, activity, found))
                        })
                    }
                    _ => selection
                        .map(|s| Task::done(Msg::Select(s)))
                        .unwrap_or_else(Task::none),
                };
                *self.nav = Nav::Activity;
                let read = if self.activity.is_unread(&message)
                    && let Some(client) = self.client.clone()
                {
                    self.activity.locally_read.insert(id.clone());
                    if let Some(m) = self.activity.messages.iter_mut().find(|m| m.id == id) {
                        set_read(m, true);
                    }
                    let generation = self.generation;
                    let result_id = id.clone();
                    Task::perform(
                        async move {
                            client
                                .set_message_property(
                                    FEED,
                                    &id,
                                    "isread",
                                    &json!({"isread":"true"}),
                                    false,
                                )
                                .await
                                .map(drop)
                                .map_err(|_| "read failed".to_owned())
                        },
                        move |result| {
                            Msg::Activity(ActivityMsg::ReadMarked(generation, result_id, result))
                        },
                    )
                } else {
                    Task::none()
                };
                // Opening a chat or channel from here stays on the Activity screen.
                let stay = Task::done(Msg::Activity(ActivityMsg::Shown));
                Task::batch([select, stay, read])
            }
            ActivityMsg::Shown => {
                *self.nav = Nav::Activity;
                Task::none()
            }
        }
    }
    pub(crate) fn activity_unread_count(&self) -> u32 {
        self.activity
            .messages
            .iter()
            .filter(|m| !m.is_deleted() && self.activity.is_unread(m))
            .count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeting_invitations_read_like_teams_and_point_to_their_event() {
        let data = json!({
            "activityType": "msGraph",
            "activitySubtype": "meetingInvitation",
            "activityContext": serde_json::to_string(&json!({
                "location": "{\"Custom\":\"3 nov., 09:00 - 17:00\"}",
                "customTapAction": "/l/meeting/details?eventId=AAMkAD%3D%3D&conversationId=19:x@thread.v2",
            })).unwrap(),
        });
        let d = describe(&data, "Josoa", "8:orgid:j");
        assert_eq!(d.title, "Josoa vous a invité");
        assert_eq!(d.context.as_deref(), Some("3 nov., 09:00 - 17:00"));
        assert_eq!(d.event_id.as_deref(), Some("AAMkAD=="));
        assert_eq!(d.badge, ActivityBadge::Meeting);

        let cancelled = describe(
            &json!({"activityType":"msGraph","activitySubtype":"meetingCancelled",
                    "activityContext":{"customTapAction":"https://teams.microsoft.com/l/meeting/details?eventId=E1"}}),
            "Daniel",
            "",
        );
        assert_eq!(cancelled.title, "Daniel a annulé");

        let templated = describe(
            &json!({"activityType":"msGraph","activitySubtype":"taskAssigned",
                    "activityContext":{"templateText":"{actor} vous a assigné {task}",
                        "templateParameters":"[{\"name\":\"task\",\"value\":\"Rapport\"}]"}}),
            "Loïc",
            "",
        );
        assert_eq!(templated.title, "Loïc vous a assigné Rapport");
        assert_eq!(templated.badge, ActivityBadge::App);
    }

    #[test]
    fn date_time_range_markup_becomes_a_local_range_and_marks_a_meeting() {
        let data = json!({
            "activityType": "msGraph",
            "activitySubtype": "meetingInvite",
            "activityContext": {"location": "{\"Custom\":\"<DateTimeRange><StartDateTime>2026-11-03T08:00:00Z</StartDateTime><EndDateTime>2026-11-03T16:00:00Z</EndDateTime></DateTimeRange>\"}"},
        });
        let d = describe(&data, "Josoa", "8:orgid:j");
        assert!(d.meeting);
        assert_eq!(d.event_id, None);
        assert_eq!(
            d.range,
            Some(("2026-11-03T08:00:00Z".into(), "2026-11-03T16:00:00Z".into()))
        );
        let expected = super::super::calendar::meeting_range(
            "2026-11-03T08:00:00Z",
            "2026-11-03T16:00:00Z",
            false,
        );
        assert_eq!(d.context, expected);
        assert!(!d.context.unwrap().contains('<'));
    }

    #[test]
    fn a_meeting_without_event_id_is_found_by_subject_then_start() {
        let event = |id: &str, subject: &str, start: &str| teams_calendar::Event {
            object_id: id.into(),
            subject: subject.into(),
            start_time: start.into(),
            ..Default::default()
        };
        let events = vec![
            event("a", "Autre", "2026-11-03T08:00:00Z"),
            event(
                "b",
                "[BERTIN] :  Kickoff Banc de tests",
                "2026-11-03T09:00:00Z",
            ),
        ];
        let found = super::super::calendar::pick_meeting(
            events.clone(),
            "2026-11-03T08:00:00Z",
            "[bertin] : Kickoff Banc de tests",
        );
        assert_eq!(found.unwrap().object_id, "b");
        let by_start =
            super::super::calendar::pick_meeting(events, "2026-11-03T08:00:00Z", "Inconnue");
        assert_eq!(by_start.unwrap().object_id, "a");
    }

    #[test]
    fn calls_reactions_and_mentions_have_their_own_wording() {
        let call = describe(
            &json!({"activityType":"call","activitySubtype":"missedCall"}),
            "Josoa",
            "8:orgid:j",
        );
        assert_eq!(call.title, "Appel manqué de Josoa");
        assert_eq!(call.context.as_deref(), Some("Appel Teams"));
        assert_eq!(call.call_back.as_deref(), Some("8:orgid:j"));
        assert_eq!(call.badge, ActivityBadge::MissedCall);

        let reaction = describe(
            &json!({"activityType":"reactionInChat","activitySubtype":"like"}),
            "Loïc",
            "",
        );
        assert_eq!(reaction.title, "Loïc a réagi à votre message");
        assert_eq!(reaction.reaction.as_deref(), Some("👍"));

        let mention = describe(
            &json!({"activityType":"mention","activitySubtype":"person"}),
            "Edouard",
            "",
        );
        assert_eq!(mention.title, "Edouard vous a mentionné");
        assert_eq!(mention.badge, ActivityBadge::Mention);
    }

    #[test]
    fn meeting_ranges_use_short_french_months() {
        use chrono::TimeZone;
        let start = Local
            .with_ymd_and_hms(2026, 11, 3, 9, 0, 0)
            .unwrap()
            .to_rfc3339();
        let end = Local
            .with_ymd_and_hms(2026, 11, 3, 17, 0, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(
            super::super::calendar::meeting_range(&start, &end, false).as_deref(),
            Some("3 nov., 09:00 - 17:00")
        );
        assert_eq!(
            super::super::calendar::meeting_range(&start, &end, true).as_deref(),
            Some("3 nov.")
        );
    }

    #[test]
    fn the_bookmark_reads_older_items_in_both_stored_forms() {
        assert_eq!(
            parse_bookmark(&json!("0;1700000000000;0")),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_bookmark(&json!(
                "{\"id\":1,\"activityTimestamp\":1700000000000,\"chatTimestamp\":1700000000500}"
            )),
            Some(1_700_000_000_500)
        );
        assert_eq!(
            parse_bookmark(&json!({"channelTimestamp": "2026-10-15T08:00:00Z"})),
            Some(1_792_051_200_000)
        );
        assert_eq!(parse_bookmark(&json!("0;0;0")), None);

        let mut app = App::empty();
        let _ = app.update_activity(ActivityMsg::Loaded(
            0,
            false,
            Ok(MessagesPage {
                messages: vec![
                    notification("old", false, "2026-10-14T08:00:00Z"),
                    notification("new", false, "2026-10-15T09:00:00Z"),
                ],
                ..Default::default()
            }),
        ));
        assert_eq!(app.activity_unread_count(), 2);
        let _ = app.update_activity(ActivityMsg::BookmarkLoaded(0, Some(1_792_051_200_000)));
        assert_eq!(
            app.activity_unread_count(),
            1,
            "only the item after the bookmark"
        );
        // An older server value never undoes a newer local "mark all as read".
        app.activity.mark_all_locally_read(1_792_200_000_000);
        let _ = app.update_activity(ActivityMsg::BookmarkLoaded(0, Some(1)));
        assert_eq!(app.activity.bookmark_ms, Some(1_792_200_000_000));
        assert_eq!(app.activity_unread_count(), 0);
    }
    fn notification(id: &str, read: bool, date: &str) -> Message {
        serde_json::from_value(json!({"id":id,"originalarrivaltime":date,"properties":{
            "isread":read.to_string(),"activity":{"activityType":"mentionInChat",
                "sourceUserId":"8:orgid:peer","sourceUserImDisplayName":"Alice",
                "sourceMessageId":"source-message","messagePreview":"<b>Bonjour</b>"}
        }}))
        .unwrap()
    }
    #[tokio::test]
    async fn read_notifications_remain_in_history_through_click_refresh_and_pagination() {
        use axum::{Json, Router, routing::put};
        let router = Router::new().route(
            "/v1/users/ME/conversations/{conv}/messages/{id}/properties",
            put(|Json(body): Json<Value>| async move {
                assert_eq!(body, json!({"isread":"true"}));
                Json(json!({}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut app = App::empty();
        *app.nav = Nav::Activity;
        *app.filter = Filter::Unread; // Discussion's filter must not leak into Activity.
        *app.client = Some(ChatClient::new(Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new().with_service(Service::ChatService, base),
        ))));
        let page = || {
            MessagesPage {
            messages: vec![
                notification("old", true, "2026-10-14T08:00:00Z"),
                notification("new", false, "2026-10-15T08:00:00Z"),
            ],
            metadata: teams_chat::PageMetadata {
                sync_state: Some("older".into()),
                backward_link: Some("https://chat.example/v1/users/ME/conversations/48%3Anotifications/messages?syncState=older".into()),
                ..Default::default()
            },
            ..Default::default()
        }
        };
        let _ = app.update_activity(ActivityMsg::Loaded(0, false, Ok(page())));
        assert_eq!(app.activity_rows().len(), 2);
        assert_eq!(app.activity_unread_count(), 1);
        assert_eq!(app.activity_rows()[0].preview, "Bonjour");
        let task = app.update_activity(ActivityMsg::Open("new".into()));
        assert_eq!(app.activity_rows().len(), 2);
        assert_eq!(app.activity_unread_count(), 0);
        for job in task.0 {
            if let crate::task::Job::Future(future) = job {
                let Msg::Activity(result) = future.await else {
                    panic!("unexpected task")
                };
                let _ = app.update_activity(result);
            }
        }
        assert!(app.activity.error.is_none(), "{:?}", app.activity.error);
        let _ = app.update_activity(ActivityMsg::Loaded(0, false, Ok(page())));
        assert_eq!(
            app.activity_unread_count(),
            0,
            "stale refresh must not undo a successful read"
        );
        let _ = app.update_activity(ActivityMsg::Loaded(
            0,
            true,
            Ok(MessagesPage {
                messages: vec![
                    notification("old", true, "2026-10-14T08:00:00Z"),
                    notification("older", true, "2026-10-01T08:00:00Z"),
                ],
                ..Default::default()
            }),
        ));
        assert_eq!(app.activity_rows().len(), 3);
        assert!(app.activity.cursor.is_none());
        let _ = app.update(Msg::SetFilter(Filter::Unread));
        assert!(app.activity_rows().is_empty());
        let _ = app.update(Msg::SetFilter(Filter::All));
        assert_eq!(app.activity_rows().len(), 3);
        let _ = app.update_activity(ActivityMsg::Loaded(0, false, Err("offline".into())));
        assert_eq!(app.activity_rows().len(), 3);
        let _ = app.update_activity(ActivityMsg::ReadMarked(
            0,
            "new".into(),
            Err("failed".into()),
        ));
        assert_eq!(app.activity_unread_count(), 1);
        server.abort();
    }
    #[test]
    fn late_activity_results_cannot_restore_history_after_sign_out() {
        let mut app = App::empty();
        let _ = app.update(Msg::SignOut);
        let _ = app.update_activity(ActivityMsg::Loaded(
            0,
            false,
            Ok(MessagesPage {
                messages: vec![notification("private", true, "2026-10-15T08:00:00Z")],
                ..Default::default()
            }),
        ));
        assert!(app.activity.messages.is_empty());
    }
}
