//! Requests pushed through Trouter: `/callAgent/{session}/{call}/…` callbacks and incoming-call notifications.
use std::collections::BTreeMap;
use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Call,
    Conversation,
}

/// A callback addressed to one call: `/callAgent/{session}/{call}/{scope}/{name}/…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackPath {
    pub session: String,
    pub call: String,
    pub scope: Scope,
    /// `rosterUpdate`, `conversationEnd`, `progress`, `mediaAnswer`…
    pub name: String,
}

impl CallbackPath {
    pub fn parse(path: &str) -> Option<Self> {
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        if segments.next()? != "callAgent" {
            return None;
        }
        let session = segments.next()?.to_owned();
        let call = segments.next()?.to_owned();
        let scope = match segments.next()? {
            "call" => Scope::Call,
            "conversation" => Scope::Conversation,
            _ => return None,
        };
        let name = segments.next()?.to_owned();
        Some(Self {
            session,
            call,
            scope,
            name,
        })
    }
}

fn inflate(bytes: &[u8]) -> Option<String> {
    let mut out = String::new();
    // pako's `inflate` expects zlib; fall back to raw deflate and gzip.
    if flate2::read::ZlibDecoder::new(bytes)
        .read_to_string(&mut out)
        .is_ok()
    {
        return Some(out);
    }
    out.clear();
    if flate2::read::DeflateDecoder::new(bytes)
        .read_to_string(&mut out)
        .is_ok()
    {
        return Some(out);
    }
    out.clear();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut out)
        .ok()
        .map(|_| out)
}

/// JSON of a pushed body, ungzipping it when `X-Microsoft-Skype-Content-Encoding: gzip`.
pub fn decode_body(body: &str, headers: &BTreeMap<String, String>) -> Option<Value> {
    let gzip = headers
        .get("x-microsoft-skype-content-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("gzip"));
    if gzip {
        let bytes = STANDARD.decode(body.trim()).ok()?;
        return serde_json::from_str(&inflate(&bytes)?).ok();
    }
    if body.trim().is_empty() {
        return Some(Value::Null);
    }
    serde_json::from_str(body).ok()
}

/// The notification inside `gp` (JSON, or base64 UTF-8 JSON) or `cp` (base64 zlib JSON).
pub fn unwrap_notification(value: &Value) -> Option<Value> {
    match value.get("gp") {
        Some(object @ Value::Object(_)) => return Some(object.clone()),
        Some(Value::String(encoded)) => {
            let bytes = STANDARD.decode(encoded).ok()?;
            return serde_json::from_slice(&bytes).ok();
        }
        _ => {}
    }
    if let Some(Value::String(encoded)) = value.get("cp") {
        let bytes = STANDARD.decode(encoded).ok()?;
        return serde_json::from_str(&inflate(&bytes)?).ok();
    }
    value.get("callNotification").map(|_| value.clone())
}

/// Event ids of call notifications.
pub const NGC_CALL: u64 = 107;
pub const GVC_CALL: u64 = 109;
pub const PSTN_CALL: u64 = 111;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingCall {
    pub event_id: u64,
    pub caller_id: String,
    pub caller_name: Option<String>,
    /// Link to acknowledge the call and fetch the caller's media offer.
    pub attach: String,
    pub conversation_controller: Option<String>,
    pub thread_id: Option<String>,
    pub multiparty: bool,
    pub call_id: Option<String>,
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// An incoming call from a pushed notification, whatever path it arrived on.
pub fn incoming_call(body: &Value) -> Option<IncomingCall> {
    let event_id = body
        .get("eventId")
        .or_else(|| body.get("evt"))
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()));
    if event_id.is_some_and(|id| ![NGC_CALL, GVC_CALL, PSTN_CALL].contains(&id)) {
        return None;
    }
    let notification = unwrap_notification(body)?;
    let call = notification.get("callNotification")?;
    let invitation = notification.get("conversationInvitation");
    Some(IncomingCall {
        event_id: event_id.unwrap_or(NGC_CALL),
        caller_id: text(call.get("from").and_then(|f| f.get("id")))?,
        caller_name: text(call.get("from").and_then(|f| f.get("displayName"))),
        attach: text(call.get("links").and_then(|l| l.get("attach")))?,
        conversation_controller: text(invitation.and_then(|i| i.get("conversationController"))),
        thread_id: text(
            notification
                .get("groupChat")
                .and_then(|g| g.get("threadId")),
        ),
        multiparty: invitation
            .and_then(|i| i.get("isMultiParty"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        call_id: text(
            notification
                .get("debugContent")
                .and_then(|d| d.get("callId")),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn notification() -> Value {
        json!({
            "callNotification": {"from": {"id": "8:orgid:bob", "displayName": "Bob"},
                                 "links": {"attach": "https://conv.example/attach/1"}},
            "conversationInvitation": {"conversationController": "https://conv.example/c1", "isMultiParty": false},
            "debugContent": {"callId": "call-1"}
        })
    }

    #[test]
    fn parses_callback_paths() {
        assert_eq!(
            CallbackPath::parse("/callAgent/s/c/conversation/conversationEnd/join/"),
            Some(CallbackPath {
                session: "s".into(),
                call: "c".into(),
                scope: Scope::Conversation,
                name: "conversationEnd".into()
            })
        );
        assert_eq!(
            CallbackPath::parse("/callAgent/s/c/call/progress/")
                .unwrap()
                .scope,
            Scope::Call
        );
        assert!(CallbackPath::parse("/messaging").is_none());
        assert!(CallbackPath::parse("/callAgent/s/c/other/x").is_none());
    }

    #[test]
    fn incoming_calls_decode_from_plain_base64_and_deflated_payloads() {
        let plain = json!({"eventId": 107, "gp": notification()});
        let base64 = json!({"eventId": "107", "gp": STANDARD.encode(notification().to_string())});
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(notification().to_string().as_bytes())
            .unwrap();
        let deflated = json!({"eventId": 107, "cp": STANDARD.encode(zlib.finish().unwrap())});
        for body in [plain, base64, deflated] {
            let call = incoming_call(&body).unwrap();
            assert_eq!(call.caller_id, "8:orgid:bob");
            assert_eq!(call.caller_name.as_deref(), Some("Bob"));
            assert_eq!(call.attach, "https://conv.example/attach/1");
            assert_eq!(
                call.conversation_controller.as_deref(),
                Some("https://conv.example/c1")
            );
            assert_eq!(call.call_id.as_deref(), Some("call-1"));
        }
        assert!(incoming_call(&json!({"eventId": 128, "gp": notification()})).is_none());
        assert!(incoming_call(&json!({"eventId": 107})).is_none());
    }

    #[test]
    fn gzip_bodies_are_base64_decoded_and_inflated() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"{\"a\":1}").unwrap();
        let headers = BTreeMap::from([(
            "x-microsoft-skype-content-encoding".to_owned(),
            "gzip".to_owned(),
        )]);
        let body = STANDARD.encode(gz.finish().unwrap());
        assert_eq!(decode_body(&body, &headers), Some(json!({"a": 1})));
        assert_eq!(
            decode_body("{\"b\":2}", &BTreeMap::new()),
            Some(json!({"b": 2}))
        );
        assert_eq!(decode_body("", &BTreeMap::new()), Some(Value::Null));
    }
}
