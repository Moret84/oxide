//! Typing indicators and read receipts (the other members' `consumptionhorizon`s).
use http::Method;
use serde_json::{Value, json};
use teams_auth::Authenticator;

use crate::client::ChatClient;
use crate::error::Error;
use crate::path::enc;

pub const TYPING: &str = "Control/Typing";
pub const CLEAR_TYPING: &str = "Control/ClearTyping";
pub const HORIZON_UPDATE: &str = "ThreadActivity/MemberConsumptionHorizonUpdate";

/// One member's read position in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberHorizon {
    /// MRI of the member.
    pub id: String,
    /// `originalarrivaltime` (epoch ms) of the last message they read.
    pub read_up_to: i64,
    /// They only see messages from this time on (joined later); 0 when unknown.
    pub visible_from: i64,
}

/// `{oat};{now};{clientmessageid}`: the first field is the last read message's arrival.
pub fn horizon_time(value: &str) -> Option<i64> {
    value.split(';').next()?.trim().parse().ok().filter(|t: &i64| *t > 0)
}

fn number(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(n)) => n.as_i64().unwrap_or_default(),
        Some(Value::String(s)) => s.parse().unwrap_or_default(),
        _ => 0,
    }
}

/// The thread's `consumptionhorizons` list (unverified shape: member as `id` or `user`).
pub fn parse_member_horizons(value: &Value) -> Vec<MemberHorizon> {
    value
        .get("consumptionhorizons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(member_horizon)
        .collect()
}

fn member_horizon(item: &Value) -> Option<MemberHorizon> {
    let id = item
        .get("id")
        .or_else(|| item.get("user"))?
        .as_str()?
        .to_owned();
    let read_up_to = horizon_time(item.get("consumptionhorizon")?.as_str()?)?;
    Some(MemberHorizon {
        id,
        read_up_to,
        visible_from: number(item.get("messageVisibilityTime")),
    })
}

/// Content of a pushed `MemberConsumptionHorizonUpdate`: `{"user", "consumptionhorizon", "messageVisibilityTime"}`.
pub fn parse_horizon_update(content: &str) -> Option<MemberHorizon> {
    member_horizon(&serde_json::from_str::<Value>(content).ok()?)
}

impl<A: Authenticator> ChatClient<A> {
    /// Tells the other members that the user is typing (`setTypingIndicator`).
    pub async fn send_typing(&self, conv: &str) -> Result<(), Error> {
        let path = format!("v1/users/ME/conversations/{}/messages", enc(conv));
        self.send_json(
            "message",
            Method::POST,
            path,
            &json!({"content": "", "contenttype": "Application/Message", "messagetype": TYPING}),
        )
        .await
        .map(drop)
    }

    /// Other members' read positions, for read receipts.
    pub async fn member_horizons(&self, conv: &str) -> Result<Vec<MemberHorizon>, Error> {
        self.consumption_horizons(conv)
            .await
            .map(|value| parse_member_horizons(&value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizons_read_the_arrival_time_of_the_last_read_message() {
        let list = parse_member_horizons(&json!({
            "id": "19:x@thread.v2",
            "consumptionhorizons": [
                {"id": "8:orgid:a", "consumptionhorizon": "1700000000123;1700000005000;42"},
                {"id": "8:orgid:b", "consumptionhorizon": "0;0;0"},
                {"user": "8:orgid:c", "consumptionhorizon": "1700000000999;1;1", "messageVisibilityTime": "17"}
            ]
        }));
        assert_eq!(
            list,
            vec![
                MemberHorizon { id: "8:orgid:a".into(), read_up_to: 1_700_000_000_123, visible_from: 0 },
                MemberHorizon { id: "8:orgid:c".into(), read_up_to: 1_700_000_000_999, visible_from: 17 },
            ]
        );
        assert_eq!(
            parse_horizon_update(r#"{"user":"8:orgid:a","consumptionhorizon":"5;6;7"}"#),
            Some(MemberHorizon { id: "8:orgid:a".into(), read_up_to: 5, visible_from: 0 })
        );
    }
}
