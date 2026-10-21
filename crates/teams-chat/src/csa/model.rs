//! CSA wire types: startup, chats, teams, channels and their posts.
use crate::people::Person;
use crate::types::{null_default, wire_optional_text, wire_text};
use crate::{Conversation, Member, Message};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Startup {
    #[serde(default, deserialize_with = "null_default")]
    pub chats: Vec<CsaChat>,
    #[serde(default, deserialize_with = "null_default")]
    pub teams: Vec<Team>,
    #[serde(default, deserialize_with = "null_default")]
    pub channels: Vec<Channel>,
    #[serde(default, deserialize_with = "null_default")]
    pub metadata: StartupMetadata,
    #[serde(default, deserialize_with = "null_default")]
    pub users: Vec<Person>,
    pub conversation_folders: Option<ConversationFolders>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolders {
    #[serde(default, deserialize_with = "null_default")]
    pub conversation_folders: Vec<ConversationFolder>,
    /// Sent back with every change; a stale one is refused with 412.
    #[serde(default)]
    pub folder_hierarchy_version: Option<i64>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolder {
    #[serde(default, deserialize_with = "null_default")]
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub folder_type: String,
    #[serde(default, deserialize_with = "null_default")]
    pub is_deleted: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub conversation_folder_items: Vec<ConversationFolderItem>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolderItem {
    pub conversation_id: String,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupMetadata {
    pub sync_token: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub is_partial_data: bool,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsaChat {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub title: String,
    #[serde(default, deserialize_with = "null_default")]
    pub members: Vec<Person>,
    pub last_message: Option<Message>,
    pub is_read: Option<bool>,
    #[serde(default, deserialize_with = "null_default")]
    pub is_sticky: bool,
    #[serde(default)]
    pub is_one_on_one: Option<bool>,
    #[serde(default, deserialize_with = "null_default")]
    pub hidden: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub is_conversation_deleted: bool,
    #[serde(default, deserialize_with = "chat_sub_type")]
    pub chat_sub_type: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub is_messaging_disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl CsaChat {
    /// `hidden` alone hides meeting threads; regular chats reappear on a message after the hide horizon.
    pub fn visible_in_chat_list(&self, favorite: bool) -> bool {
        if self.is_conversation_deleted {
            return false;
        }
        if favorite || !self.hidden {
            return true;
        }
        let regular_chat = self.is_one_on_one.is_some()
            || self
                .extra
                .get("chatType")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("chat"));
        if !regular_chat
            || self
                .chat_sub_type
                .as_deref()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("meeting"))
        {
            return false;
        }
        let horizon = ["hiddenTime", "unpinnedTime", "historyHiddenTime"]
            .into_iter()
            .filter_map(|key| self.extra.get(key).and_then(time))
            .max()
            .unwrap_or(0);
        self.last_message
            .as_ref()
            .and_then(|message| {
                [
                    message.originalarrivaltime.as_ref(),
                    message.composetime.as_ref(),
                ]
                .into_iter()
                .flatten()
                .find_map(|t| time(&Value::String(t.clone())))
            })
            .is_some_and(|time| time > horizon)
    }
    pub fn conversation(&self) -> Conversation {
        let mut c = Conversation {
            id: self.id.clone(),
            last_message: self.last_message.clone(),
            extra: self.extra.clone(),
            ..Default::default()
        };
        if !self.title.is_empty() {
            c.thread_properties
                .insert("topic".into(), json!(self.title));
        }
        c.members = self
            .members
            .iter()
            .map(|p| Member {
                id: p.mri.clone(),
                role: p.role.clone(),
                extra: p
                    .known_name()
                    .map(|name| Map::from_iter([("friendlyName".into(), json!(name))]))
                    .unwrap_or_default(),
            })
            .collect();
        c.roster_version = c.extra.remove("rosterVersion");
        c.needs_thread_sync = c.extra.remove("needsThreadSync").and_then(|v| v.as_bool());
        c.is_one_on_one = self.is_one_on_one;
        c.is_read = Some(self.is_read.unwrap_or(true));
        c.is_sticky = Some(self.is_sticky);
        c.meeting = Some(
            self.chat_sub_type
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("meeting")),
        );
        c.disabled = Some(self.is_messaging_disabled);
        c
    }
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub display_name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default, deserialize_with = "null_default")]
    pub channels: Vec<Channel>,
    #[serde(default, deserialize_with = "null_default")]
    pub is_deleted: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub is_archived: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub parent_team_id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub display_name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub description: String,
    #[serde(default, deserialize_with = "channel_type")]
    pub channel_type: String,
    #[serde(default, deserialize_with = "null_default")]
    pub is_general: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub is_deleted: bool,
    #[serde(default, deserialize_with = "null_default")]
    pub is_archived: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
fn extra_str<'a>(extra: &'a Map<String, Value>, path: &str) -> Option<&'a str> {
    let mut parts = path.split('/');
    let mut value = extra.get(parts.next()?)?;
    for part in parts {
        value = value.get(part)?;
    }
    value.as_str().filter(|s| !s.trim().is_empty())
}
impl Team {
    /// The team's Microsoft 365 group (`teamSiteInformation.groupId`).
    pub fn group_id(&self) -> Option<&str> {
        extra_str(&self.extra, "teamSiteInformation/groupId")
    }
    /// SharePoint site of the team (`teamSiteInformation.sharepointSiteUrl`).
    pub fn sharepoint_site_url(&self) -> Option<&str> {
        extra_str(&self.extra, "teamSiteInformation/sharepointSiteUrl")
    }
}
impl Channel {
    /// Own site of a private or shared channel; standard channels use the team's.
    pub fn sharepoint_site_url(&self) -> Option<&str> {
        extra_str(&self.extra, "sharepointSiteUrl")
    }
    /// Server-relative folder of the channel's files (`defaultFileSettings.filesRelativePath`).
    pub fn files_relative_path(&self) -> Option<&str> {
        extra_str(&self.extra, "defaultFileSettings/filesRelativePath")
    }
    /// Latest post or reply (`lastMessage`, else `lastL1Message`).
    pub fn last_message(&self) -> Option<Message> {
        ["lastMessage", "lastL1Message"]
            .into_iter()
            .filter_map(|key| self.extra.get(key))
            .find_map(|v| serde_json::from_value(v.clone()).ok())
    }
    pub fn set_last_message(&mut self, message: &Message) {
        if let Ok(value) = serde_json::to_value(message) {
            self.extra.insert("lastMessage".into(), value);
        }
    }
    /// Read up to `horizon` (the chat service's `consumptionhorizon`); also clears a "marked as unread" bookmark.
    pub fn set_read(&mut self, horizon: String) {
        self.extra
            .insert("consumptionHorizon".into(), json!(horizon));
        self.extra.remove("userConsumptionHorizon");
    }
    /// The web client's `!isRead`: marked unread, or someone else's last message is past the horizon.
    pub fn is_unread(&self, me: &str) -> bool {
        let bookmark = self
            .extra
            .get("userConsumptionHorizon")
            .map(horizon)
            .unwrap_or_default();
        if bookmark.0 > 0 && !bookmark.1.is_empty() && bookmark.1 != "0" {
            return true;
        }
        let Some(last) = self.last_message() else {
            return false;
        };
        let arrived = [&last.originalarrivaltime, &last.composetime]
            .into_iter()
            .flatten()
            .find_map(|t| time(&Value::String(t.clone())))
            .unwrap_or(0);
        let read = self
            .extra
            .get("consumptionHorizon")
            .map(horizon)
            .unwrap_or_default()
            .0;
        let mine = !me.is_empty() && last.from.contains(me);
        arrived > read && !mine
    }
}
/// `originalArrivalTime;timeStamp;clientMessageId`, as a string or an object.
fn horizon(value: &Value) -> (i64, String) {
    match value {
        Value::Object(o) => (
            o.get("originalArrivalTime").and_then(time).unwrap_or(0),
            o.get("clientMessageId")
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                .unwrap_or_default(),
        ),
        Value::String(s) => {
            let mut parts = s.split(';');
            let arrived = parts
                .next()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            (arrived, parts.nth(1).unwrap_or_default().trim().to_owned())
        }
        _ => (0, String::new()),
    }
}
/// Milliseconds, as a number, a numeric string or an RFC 3339 date.
fn time(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let s = value.as_str()?;
        s.parse().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|t| t.timestamp_millis())
        })
    })
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyChain {
    #[serde(deserialize_with = "wire_text")]
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub messages: Vec<Message>,
    #[serde(default, deserialize_with = "null_default")]
    pub container_id: String,
}
impl ReplyChain {
    pub fn root(&self) -> Option<&Message> {
        self.messages
            .iter()
            .find(|m| m.id == self.id)
            .or_else(|| self.messages.first())
    }
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostsPage {
    #[serde(deserialize_with = "null_default")]
    pub reply_chains: Vec<ReplyChain>,
    pub continuation_token: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepliesPage {
    #[serde(default, deserialize_with = "null_default")]
    pub messages: Vec<Message>,
    pub continuation_token: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatsPage {
    #[serde(default, deserialize_with = "null_default")]
    pub chats: Vec<CsaChat>,
    pub continuation_token: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub has_more_data: bool,
}

/// One page of `calling/users/me/callhistory`: call-log messages of `48:calllogs`. Unverified shape.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHistoryPage {
    #[serde(default, deserialize_with = "null_default")]
    pub messages: Vec<Message>,
    /// `x-ms-continuation` of the response, when the service pages that way.
    pub continuation_token: Option<String>,
    #[serde(default, rename = "_metadata", deserialize_with = "null_default")]
    pub metadata: Map<String, Value>,
}

/// CSA wire enum; these are not the Graph/API enum values.
fn channel_type<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let code = match &value {
        Value::Null => return Ok(String::new()),
        Value::String(s) => match s.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return Ok(s.clone()),
        },
        Value::Number(n) => n.as_i64().ok_or_else(|| {
            serde::de::Error::custom("channelType doit être un entier ou un libellé")
        })?,
        _ => {
            return Err(serde::de::Error::custom(
                "channelType doit être un entier ou un libellé",
            ));
        }
    };
    Ok(match code {
        0 => "Standard".into(),
        1 => "Private".into(),
        4 => "Shared".into(),
        9 => "Standalone".into(),
        _ => format!("Unknown({code})"),
    })
}

/// CSA export vW: None=0, Meeting=1, Channel=2 (not Graph conversation types).
fn chat_sub_type<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(
        wire_optional_text(deserializer)?.map(|value| match value.as_str() {
            "0" => "None".into(),
            "1" => "Meeting".into(),
            "2" => "Channel".into(),
            _ if value.parse::<i64>().is_ok() => format!("Unknown({value})"),
            _ => value,
        }),
    )
}
