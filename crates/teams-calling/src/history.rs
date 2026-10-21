//! Call logs from `48:calllogs` messages; the raw `call-log` object is kept so a renamed field never loses data.
use chrono::{DateTime, FixedOffset};
use serde_json::{Value, json};

/// The parts of a [`CALL_LOGS`] message a log is read from, so this crate needs no chat types.
#[derive(Debug, Clone, Copy)]
pub struct LogMessage<'a> {
    pub id: &'a str,
    pub original_arrival_time: Option<&'a str>,
    /// `properties["call-log"]`: a JSON string, or an object.
    pub call_log: Option<&'a Value>,
    pub is_deleted: bool,
}

/// Conversation that stores the personal call logs.
pub const CALL_LOGS: &str = "48:calllogs";

/// Numbers and bots the web client never shows (`callLogsBlacklistedMRIs`).
const HIDDEN: [&str; 3] = ["4:*11", "4:*12", "4:*13"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallState {
    Missed,
    Accepted,
    Declined,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallParticipant {
    pub id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CallLog {
    /// Message id(s) in `48:calllogs`, possibly several joined by `_`.
    pub message_id: String,
    pub call_id: String,
    pub direction: CallDirection,
    pub state: CallState,
    pub start_time: Option<String>,
    pub connect_time: Option<String>,
    pub end_time: Option<String>,
    pub originator: Option<CallParticipant>,
    pub target: Option<CallParticipant>,
    pub participants: Vec<CallParticipant>,
    /// `TwoParty` or `MultiParty`.
    pub call_type: Option<String>,
    pub group_chat_thread_id: Option<String>,
    pub is_deleted: bool,
    /// Decoded `call-log` object, as sent.
    pub raw: Value,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn participant(value: Option<&Value>) -> Option<CallParticipant> {
    let value = value?;
    Some(CallParticipant {
        id: text(value, "id")?,
        display_name: text(value, "displayName"),
    })
}

fn time(value: &Option<String>) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.as_deref()?).ok()
}

impl CallLog {
    pub fn from_message(message: LogMessage<'_>) -> Option<Self> {
        let raw = match message.call_log? {
            Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
            object @ Value::Object(_) => object.clone(),
            _ => return None,
        };
        let direction = match text(&raw, "callDirection").as_deref() {
            Some(d) if d.eq_ignore_ascii_case("incoming") => CallDirection::Incoming,
            Some(d) if d.eq_ignore_ascii_case("outgoing") => CallDirection::Outgoing,
            _ => CallDirection::Unknown,
        };
        let state = match text(&raw, "callState") {
            Some(s) if s.eq_ignore_ascii_case("missed") => CallState::Missed,
            Some(s) if s.eq_ignore_ascii_case("accepted") => CallState::Accepted,
            Some(s) if s.eq_ignore_ascii_case("declined") || s.eq_ignore_ascii_case("rejected") => {
                CallState::Declined
            }
            other => CallState::Other(other.unwrap_or_default()),
        };
        let participants = raw
            .get("participants")
            .or_else(|| raw.get("participantList"))
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|p| participant(Some(p))).collect())
            .unwrap_or_default();
        Some(Self {
            message_id: message.id.to_owned(),
            call_id: text(&raw, "callId").unwrap_or_else(|| message.id.to_owned()),
            direction,
            state,
            start_time: text(&raw, "startTime")
                .or_else(|| message.original_arrival_time.map(str::to_owned)),
            connect_time: text(&raw, "connectTime"),
            end_time: text(&raw, "endTime"),
            originator: participant(raw.get("originatorParticipant")),
            target: participant(raw.get("targetParticipant")),
            participants,
            call_type: text(&raw, "callType"),
            group_chat_thread_id: text(&raw, "groupChatThreadId"),
            is_deleted: raw.get("isDeleted").and_then(Value::as_bool) == Some(true)
                || message.is_deleted,
            raw,
        })
    }

    /// Connected duration in seconds; `None` for unanswered calls.
    pub fn duration_secs(&self) -> Option<i64> {
        let start = time(&self.connect_time)?;
        let end = time(&self.end_time)?;
        Some((end - start).num_seconds().max(0))
    }

    /// The other side of the call as seen by `me`.
    pub fn counterpart(&self, me: &str) -> Option<&CallParticipant> {
        let side = match self.direction {
            CallDirection::Incoming => self.originator.as_ref(),
            CallDirection::Outgoing => self.target.as_ref(),
            CallDirection::Unknown => None,
        };
        side.filter(|p| p.id != me)
            .or_else(|| {
                [self.originator.as_ref(), self.target.as_ref()]
                    .into_iter()
                    .flatten()
                    .find(|p| p.id != me)
            })
            .or_else(|| self.participants.iter().find(|p| p.id != me))
    }

    fn hidden(&self) -> bool {
        [self.originator.as_ref(), self.target.as_ref()]
            .into_iter()
            .flatten()
            .any(|p| HIDDEN.contains(&p.id.as_str()))
    }

    /// The `call-log` property value marking this log deleted (`deleteCallHistoryLog`).
    pub fn deleted_property(&self) -> Value {
        let mut raw = self.raw.clone();
        if let Some(object) = raw.as_object_mut() {
            object.insert("isDeleted".into(), json!(true));
        }
        json!(raw.to_string())
    }

    /// Message ids to rewrite when deleting: one log may aggregate several messages.
    pub fn message_ids(&self) -> impl Iterator<Item = &str> {
        self.message_id.split('_').filter(|s| !s.is_empty())
    }
}

/// Visible call logs of a page, newest first, without duplicates.
pub fn visible<'a>(messages: impl IntoIterator<Item = LogMessage<'a>>) -> Vec<CallLog> {
    let mut logs: Vec<CallLog> = messages
        .into_iter()
        .filter_map(CallLog::from_message)
        .filter(|log| !log.is_deleted && !log.hidden())
        .collect();
    logs.sort_by_key(|log| std::cmp::Reverse(time(&log.start_time)));
    let mut seen = std::collections::HashSet::new();
    logs.retain(|log| seen.insert(log.call_id.clone()));
    logs
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stored(String, Value);
    impl Stored {
        fn log(&self) -> LogMessage<'_> {
            LogMessage {
                id: &self.0,
                original_arrival_time: Some("2026-10-13T09:00:00.000Z"),
                call_log: Some(&self.1),
                is_deleted: false,
            }
        }
    }

    fn message(id: &str, log: Value) -> Stored {
        Stored(id.into(), json!(log.to_string()))
    }

    #[test]
    fn parses_call_logs_and_computes_the_counterpart_and_duration() {
        let log = CallLog::from_message(
            message(
                "1",
                json!({
                    "callId": "c1", "callDirection": "outgoing", "callState": "accepted",
                    "startTime": "2026-10-13T09:00:00Z", "connectTime": "2026-10-13T09:00:05Z",
                    "endTime": "2026-10-13T09:02:05Z", "callType": "TwoParty",
                    "originatorParticipant": {"id": "8:orgid:me", "displayName": "Moi"},
                    "targetParticipant": {"id": "8:orgid:bob", "displayName": "Bob"},
                    "futureField": 1
                }),
            )
            .log(),
        )
        .unwrap();
        assert_eq!(log.direction, CallDirection::Outgoing);
        assert_eq!(log.state, CallState::Accepted);
        assert_eq!(log.duration_secs(), Some(120));
        assert_eq!(log.counterpart("8:orgid:me").unwrap().id, "8:orgid:bob");
        assert_eq!(log.raw["futureField"], 1);
    }

    #[test]
    fn deleted_hidden_and_duplicate_logs_are_not_visible() {
        let page = [
            message(
                "1",
                json!({"callId": "a", "callState": "missed", "callDirection": "incoming",
                "startTime": "2026-10-13T08:00:00Z", "originatorParticipant": {"id": "8:orgid:bob"}}),
            ),
            message("2", json!({"callId": "b", "isDeleted": true})),
            message(
                "3",
                json!({"callId": "c", "targetParticipant": {"id": "4:*11"}}),
            ),
            message(
                "4",
                json!({"callId": "a", "startTime": "2026-10-13T07:00:00Z"}),
            ),
            message(
                "5",
                json!({"callId": "d", "startTime": "2026-10-13T10:00:00Z"}),
            ),
        ];
        let logs = visible(page.iter().map(Stored::log));
        assert_eq!(
            logs.iter().map(|l| l.call_id.as_str()).collect::<Vec<_>>(),
            ["d", "a"]
        );
        assert_eq!(logs[1].state, CallState::Missed);
        assert_eq!(logs[1].duration_secs(), None);
    }

    #[test]
    fn deletion_rewrites_the_json_string_property_for_each_message() {
        let mut log = CallLog::from_message(message("1_2", json!({"callId": "a"})).log()).unwrap();
        let property = log.deleted_property();
        let decoded: Value = serde_json::from_str(property.as_str().unwrap()).unwrap();
        assert_eq!(decoded["isDeleted"], true);
        assert_eq!(log.message_ids().collect::<Vec<_>>(), ["1", "2"]);
        log.message_id = "3".into();
        assert_eq!(log.message_ids().count(), 1);
    }
}
