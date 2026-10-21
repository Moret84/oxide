//! socket.io 0.9 packets (`type:id[+]:endpoint[:data]`), as spoken by Trouter; no I/O.
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Disconnect = 0,
    Connect = 1,
    Heartbeat = 2,
    Message = 3,
    Json = 4,
    Event = 5,
    Ack = 6,
    Error = 7,
    Noop = 8,
}

impl Kind {
    fn from_digit(c: &str) -> Option<Self> {
        Some(match c {
            "0" => Self::Disconnect,
            "1" => Self::Connect,
            "2" => Self::Heartbeat,
            "3" => Self::Message,
            "4" => Self::Json,
            "5" => Self::Event,
            "6" => Self::Ack,
            "7" => Self::Error,
            "8" => Self::Noop,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub kind: Kind,
    pub id: Option<u64>,
    /// `+` after the id: the sender wants the ack to carry data.
    pub ack_data: bool,
    pub endpoint: String,
    pub data: String,
}

impl Packet {
    pub fn new(kind: Kind, data: impl Into<String>) -> Self {
        Self {
            kind,
            id: None,
            ack_data: false,
            endpoint: String::new(),
            data: data.into(),
        }
    }

    /// `5:<id>+::{"name":…,"args":[…]}`; `args` is omitted when empty.
    pub fn event(id: Option<u64>, name: &str, args: &[Value]) -> Self {
        let data = if args.is_empty() {
            json!({ "name": name })
        } else {
            json!({ "name": name, "args": args })
        };
        Self {
            kind: Kind::Event,
            id,
            ack_data: id.is_some(),
            endpoint: String::new(),
            data: data.to_string(),
        }
    }

    pub fn encode(&self) -> String {
        let id = match self.id {
            Some(id) if self.ack_data => format!("{id}+"),
            Some(id) => id.to_string(),
            None => String::new(),
        };
        let mut out = format!("{}:{id}:{}", self.kind as u8, self.endpoint);
        if !self.data.is_empty() {
            out.push(':');
            out.push_str(&self.data);
        }
        out
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(4, ':');
        let kind = Kind::from_digit(parts.next()?)?;
        let id = parts.next().unwrap_or("");
        let endpoint = parts.next().unwrap_or("").to_owned();
        let data = parts.next().unwrap_or("").to_owned();
        let (id, ack_data) = match id.strip_suffix('+') {
            Some(id) => (id, true),
            None => (id, false),
        };
        let id = if id.is_empty() {
            None
        } else {
            Some(id.parse().ok()?)
        };
        Some(Self {
            kind,
            id,
            ack_data,
            endpoint,
            data,
        })
    }

    /// Name and arguments of an event packet.
    pub fn event_parts(&self) -> Option<(String, Vec<Value>)> {
        if self.kind != Kind::Event {
            return None;
        }
        let value: Value = serde_json::from_str(&self.data).ok()?;
        let name = value.get("name")?.as_str()?.to_owned();
        let args = value
            .get("args")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Some((name, args))
    }

    /// Acked message id of an ack packet (`6:::<id>[+data]`).
    pub fn acked(&self) -> Option<u64> {
        if self.kind != Kind::Ack {
            return None;
        }
        let id = self.data.split('+').next()?;
        id.parse().ok()
    }
}

/// Splits a frame that may concatenate packets as `\u{fffd}<len>\u{fffd}<packet>…`, lengths in UTF-16 units.
pub fn decode_frame(frame: &str) -> Vec<Packet> {
    const SEP: char = '\u{fffd}';
    if !frame.starts_with(SEP) {
        return Packet::decode(frame).into_iter().collect();
    }
    let mut packets = Vec::new();
    let mut rest = frame;
    while let Some(after) = rest.strip_prefix(SEP) {
        let Some(end) = after.find(SEP) else { break };
        let Ok(len) = after[..end].parse::<usize>() else {
            break;
        };
        let body = &after[end + SEP.len_utf8()..];
        let mut units = 0;
        let mut split = body.len();
        for (i, c) in body.char_indices() {
            if units == len {
                split = i;
                break;
            }
            units += c.len_utf16();
        }
        if units < len {
            break;
        }
        packets.extend(Packet::decode(&body[..split]));
        rest = &body[split..];
    }
    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_trouter_packets() {
        for raw in [
            "1::",
            "2::",
            "0::dup",
            "6:::12",
            "5:3+::{\"name\":\"ping\"}",
            "3:::{\"id\":1,\"method\":\"POST\"}",
            "3:7::{\"a\":\"x:y\"}",
        ] {
            let packet = Packet::decode(raw).unwrap();
            assert_eq!(packet.encode(), raw, "{raw}");
        }
        let ping = Packet::event(Some(3), "ping", &[]);
        assert_eq!(ping.encode(), "5:3+::{\"name\":\"ping\"}");
        assert_eq!(Packet::decode("6:::3").unwrap().acked(), Some(3));
        assert_eq!(Packet::decode("6:::3+[\"ok\"]").unwrap().acked(), Some(3));
    }

    #[test]
    fn data_keeps_its_colons_and_events_expose_their_arguments() {
        let raw = r#"5:::{"name":"trouter.connected","args":[{"surl":"https://h:3443/x/"}]}"#;
        let packet = Packet::decode(raw).unwrap();
        let (name, args) = packet.event_parts().unwrap();
        assert_eq!(name, "trouter.connected");
        assert_eq!(args[0]["surl"], "https://h:3443/x/");
    }

    #[test]
    fn rejects_garbage_and_splits_framed_batches_by_utf16_length() {
        assert!(Packet::decode("x::").is_none());
        assert!(Packet::decode("3:abc::").is_none());
        let a = "3:::{\"t\":\"é😀\"}";
        let b = "2::";
        let frame = format!(
            "\u{fffd}{}\u{fffd}{a}\u{fffd}{}\u{fffd}{b}",
            a.encode_utf16().count(),
            b.encode_utf16().count()
        );
        let packets = decode_frame(&frame);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].data, "{\"t\":\"é😀\"}");
        assert_eq!(packets[1].kind, Kind::Heartbeat);
    }
}
