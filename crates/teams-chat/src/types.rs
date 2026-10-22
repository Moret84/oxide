//! Wire types of the chat service; shapes marked "unverified" were never checked against a live response.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A message as returned by the service or carried by a `NewMessage` event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Message {
    #[serde(default, deserialize_with = "wire_text")]
    pub id: String,
    #[serde(
        default,
        alias = "clientMessageId",
        deserialize_with = "wire_optional_text"
    )]
    pub clientmessageid: Option<String>,
    #[serde(default, deserialize_with = "wire_optional_text")]
    pub version: Option<String>,
    #[serde(default, rename = "sequenceId")]
    pub sequence_id: Option<u64>,
    #[serde(default, alias = "messageType", deserialize_with = "null_default")]
    pub messagetype: String,
    #[serde(default)]
    pub contenttype: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub content: String,
    /// `…/v1/users/ME/contacts/{mri}`; see [`Message::sender_mri`].
    #[serde(default, deserialize_with = "wire_sender")]
    pub from: String,
    #[serde(default)]
    pub imdisplayname: Option<String>,
    #[serde(default)]
    #[serde(alias = "composeTime", deserialize_with = "wire_time")]
    pub composetime: Option<String>,
    #[serde(default)]
    #[serde(alias = "originalArrivalTime", deserialize_with = "wire_time")]
    pub originalarrivaltime: Option<String>,
    #[serde(default, rename = "conversationLink")]
    pub conversation_link: Option<String>,
    #[serde(default)]
    pub threadtype: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub properties: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Message {
    pub fn is_deleted(&self) -> bool {
        self.properties.get("deletetime").is_some_and(|v| {
            v.as_str().is_some_and(|s| !s.is_empty() && s != "0")
                || v.as_u64().is_some_and(|n| n > 0)
        })
    }

    /// MRI of the sender (`8:orgid:…`), the last path segment of `from`.
    pub fn sender_mri(&self) -> &str {
        self.from.rsplit('/').next().unwrap_or_default()
    }

    /// Conversation id, the last path segment of `conversationLink`.
    pub fn conversation_id(&self) -> Option<&str> {
        let link = self.conversation_link.as_deref()?;
        let last = link.rsplit('/').next()?;
        Some(last.split(';').next().unwrap_or(last))
    }
}

/// Body of `POST v1/users/ME/conversations/{conv}/messages`.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingMessage {
    /// Random id echoed back by the service, used to deduplicate our own message.
    pub clientmessageid: String,
    pub messagetype: String,
    pub contenttype: String,
    pub content: String,
    pub imdisplayname: String,
    pub properties: OutgoingProperties,
}

impl OutgoingMessage {
    /// Rich-text (HTML) message, the format the Teams client sends.
    pub fn html(
        clientmessageid: impl Into<String>,
        html: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            clientmessageid: clientmessageid.into(),
            messagetype: "RichText/Html".into(),
            contenttype: "text".into(),
            content: html.into(),
            imdisplayname: display_name.into(),
            properties: OutgoingProperties::default(),
        }
    }
}

/// Message properties. List-valued fields are JSON *strings*, not arrays.
#[derive(Debug, Clone, Serialize)]
pub struct OutgoingProperties {
    pub mentions: String,
    pub cards: String,
    pub links: String,
    pub files: String,
    pub importance: String,
    pub subject: String,
    pub title: String,
    #[serde(rename = "formatVariant")]
    pub format_variant: String,
    /// A card's `messageBack` text: the bot gets the invoke, not this message.
    #[serde(rename = "skipfanouttobots", skip_serializing_if = "std::ops::Not::not")]
    pub skip_fan_out_to_bots: bool,
}

impl Default for OutgoingProperties {
    fn default() -> Self {
        Self {
            mentions: "[]".into(),
            cards: "[]".into(),
            links: "[]".into(),
            files: "[]".into(),
            importance: String::new(),
            subject: String::new(),
            title: String::new(),
            format_variant: "TEAMS".into(),
            skip_fan_out_to_bots: false,
        }
    }
}

/// Page of messages. Unverified shape.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessagesPage {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, rename = "_metadata")]
    pub metadata: PageMetadata,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Pagination cursor of a [`MessagesPage`]. Unverified shape.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PageMetadata {
    #[serde(default, rename = "syncState")]
    pub sync_state: Option<String>,
    #[serde(default, rename = "backwardLink")]
    pub backward_link: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Conversation or thread as returned with `view=msnp24Equivalent`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub properties: Map<String, Value>,
    /// Thread-level properties such as `topic`. Unverified shape.
    #[serde(default, rename = "threadProperties")]
    pub thread_properties: Map<String, Value>,
    #[serde(default, rename = "lastMessage")]
    pub last_message: Option<Message>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(
        default,
        rename = "isOneOnOne",
        deserialize_with = "flag",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_one_on_one: Option<bool>,
    /// `Some(false)`: has unread messages.
    #[serde(
        default,
        rename = "isRead",
        deserialize_with = "flag",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_read: Option<bool>,
    /// Pinned to the top of the chat list.
    #[serde(
        default,
        rename = "isSticky",
        deserialize_with = "flag",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_sticky: Option<bool>,
    /// The chat of a meeting. Any non-null, non-false value counts.
    #[serde(
        default,
        deserialize_with = "presence",
        skip_serializing_if = "Option::is_none"
    )]
    pub meeting: Option<bool>,
    /// Messaging is turned off: the chat is read-only.
    #[serde(
        default,
        deserialize_with = "flag",
        skip_serializing_if = "Option::is_none"
    )]
    pub disabled: Option<bool>,
    #[serde(
        default,
        rename = "needsThreadSync",
        deserialize_with = "flag",
        skip_serializing_if = "Option::is_none"
    )]
    pub needs_thread_sync: Option<bool>,
    /// Changes when the roster does.
    #[serde(
        default,
        rename = "rosterVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub roster_version: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Conversation {
    pub fn is_one_on_one(&self) -> bool {
        self.is_one_on_one == Some(true)
    }
    pub fn is_unread(&self) -> bool {
        self.is_read == Some(false)
    }
    pub fn is_meeting(&self) -> bool {
        self.meeting == Some(true)
    }
    pub fn is_disabled(&self) -> bool {
        self.disabled == Some(true)
    }
    pub fn needs_thread_sync(&self) -> bool {
        self.needs_thread_sync == Some(true)
    }
    /// Explicit title only. A sender name is never a conversation/group title.
    pub fn title(&self) -> Option<&str> {
        [&self.thread_properties, &self.properties, &self.extra]
            .into_iter()
            .flat_map(|map| {
                ["topic", "title", "displayName"]
                    .into_iter()
                    .filter_map(move |key| map.get(key).and_then(Value::as_str))
            })
            .map(str::trim)
            .find(|s| !s.is_empty())
            .or_else(|| {
                self.extra
                    .get("meetingInformation")?
                    .get("subject")?
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
    }
    pub fn is_group(&self) -> bool {
        self.members.len() > 2 || self.is_one_on_one == Some(false)
    }
}

/// Page of conversations. Unverified shape.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConversationsPage {
    #[serde(default)]
    pub conversations: Vec<Conversation>,
    #[serde(default, rename = "_metadata")]
    pub metadata: PageMetadata,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Member {
    /// MRI such as `8:orgid:{objectId}`.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Member {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if let Some(id) = value.as_str() {
            return Ok(Self {
                id: id.to_owned(),
                ..Default::default()
            });
        }
        let mut fields = value
            .as_object()
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("invalid member"))?;
        let id = fields
            .remove("id")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                fields
                    .remove("mri")
                    .and_then(|v| v.as_str().map(str::to_owned))
            })
            .unwrap_or_default();
        let role = fields
            .remove("role")
            .and_then(|v| v.as_str().map(str::to_owned));
        Ok(Self {
            id,
            role,
            extra: fields,
        })
    }
}
impl Member {
    pub fn display_name(&self) -> Option<&str> {
        ["friendlyName", "displayName", "displayname", "name"]
            .into_iter()
            .filter_map(|key| self.extra.get(key).and_then(Value::as_str))
            .map(str::trim)
            .find(|s| !s.is_empty() && !is_placeholder_name(&self.id, s))
    }
}

/// A name that only restates the id: the MRI itself, or `Bot-{id}`, which Teams gives a bot (`28:{id}`) it has no name for.
pub fn is_placeholder_name(mri: &str, name: &str) -> bool {
    name == mri
        || mri.strip_prefix("28:").is_some_and(|id| {
            name.get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bot-"))
                && name[4..].eq_ignore_ascii_case(id)
        })
}

/// Body of `POST v1/threads` (create a chat).
#[derive(Debug, Clone, Serialize)]
pub struct NewThread {
    pub members: Vec<Member>,
    pub properties: Map<String, Value>,
}

/// Kind of push event.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum ResourceType {
    NewMessage,
    MessageUpdate,
    ConversationUpdate,
    ThreadUpdate,
    CustomUserProperties,
    #[serde(other)]
    Other,
}

/// Push event envelope (long-poll and Trouter).
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub id: Value,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(rename = "resourceType")]
    pub resource_type: ResourceType,
    #[serde(default, rename = "resourceLink")]
    pub resource_link: String,
    #[serde(default)]
    pub resource: Value,
}

impl Event {
    /// The message carried by a `NewMessage` / `MessageUpdate` event.
    pub fn message(&self) -> Option<Message> {
        match self.resource_type {
            ResourceType::NewMessage | ResourceType::MessageUpdate => {
                serde_json::from_value(self.resource.clone()).ok()
            }
            _ => None,
        }
    }
}

/// Response of a long-poll request. Unverified shape.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EventBatch {
    #[serde(default, rename = "eventMessages")]
    pub events: Vec<Event>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// CSA uses numeric versions; Chat Service returns them as strings (FromCSAMessage).
pub(crate) fn wire_optional_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(Some(n.to_string())),
        _ => Err(serde::de::Error::custom("expected a string or integer")),
    }
}
pub(crate) fn wire_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    Ok(wire_optional_text(deserializer)?.unwrap_or_default())
}
fn wire_sender<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let from: String = null_default(deserializer)?;
    if from.starts_with("orgid:") {
        Ok(format!("8:{from}"))
    } else if let Some((path, id)) = from.rsplit_once('/')
        && id.starts_with("orgid:")
    {
        Ok(format!("{path}/8:{id}"))
    } else {
        Ok(from)
    }
}

/// A boolean flag, sometimes sent as `"true"`; any other value is ignored rather than failing the object.
fn flag<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Bool(b) => Some(b),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}
/// `true` for any value but `null` and `false`.
fn presence<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => None,
        Value::Bool(b) => Some(b),
        _ => Some(true),
    })
}

/// `null` reads as the default value.
pub(crate) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
fn wire_time<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => chrono::DateTime::from_timestamp_millis(
            n.as_i64()
                .ok_or_else(|| serde::de::Error::custom("expected integer milliseconds"))?,
        )
        .map(|t| Some(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)))
        .ok_or_else(|| serde::de::Error::custom("message timestamp out of range")),
        _ => Err(serde::de::Error::custom(
            "expected an ISO date or integer milliseconds",
        )),
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn message_sender_uses_the_same_mri_as_participants_and_signed_in_user() {
        for from in [
            "orgid:alice",
            "8:orgid:alice",
            "https://chat.example/v1/users/ME/contacts/orgid:alice",
            "https://chat.example/v1/users/ME/contacts/8:orgid:alice",
        ] {
            let message: Message =
                serde_json::from_value(serde_json::json!({"from":from})).unwrap();
            assert_eq!(message.sender_mri(), "8:orgid:alice");
        }
    }

    #[test]
    fn conversation_flags_are_typed_and_keep_their_wire_names() {
        let wire = serde_json::json!({
            "id": "c", "isOneOnOne": true, "isRead": false, "isSticky": "true",
            "meeting": {"subject": "Point"}, "disabled": 3, "rosterVersion": 7, "other": 1
        });
        let c: Conversation = serde_json::from_value(wire).unwrap();
        assert!(c.is_one_on_one() && c.is_unread() && c.is_meeting() && !c.is_disabled());
        assert_eq!(c.is_sticky, Some(true));
        assert_eq!(c.roster_version, Some(serde_json::json!(7)));
        assert_eq!(c.extra.keys().collect::<Vec<_>>(), ["other"]);
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["isRead"], false);
        assert_eq!(back["rosterVersion"], 7);
        assert!(back.get("disabled").is_none());
    }
}
