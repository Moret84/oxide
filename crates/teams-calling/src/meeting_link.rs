//! Meeting links: `…/l/meetup-join/{thread}/{message}?context=` is complete; `…/meet/{id}?p=` needs a tenant lookup.
use percent_encoding::percent_decode_str;
use serde_json::Value;
use url::Url;

const HOSTS: [&str; 10] = [
    "teams.microsoft.com",
    "teams.cloud.microsoft",
    "gov.teams.microsoft.us",
    "dod.teams.microsoft.us",
    "teams.live.com",
    "teams.microsoftonline.cn",
    "teams.sovcloud.fr",
    "teams.sovcloud.de",
    "teams.eaglex.ic.gov",
    "teams.microsoft.scloud",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingCoordinates {
    /// `19:meeting_…@thread.v2`.
    pub thread_id: String,
    /// `0` for a private meeting, the post id for a channel meeting.
    pub message_id: String,
    pub tenant_id: Option<String>,
    pub organizer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingLink {
    Coordinates(MeetingCoordinates),
    /// New-style link: the tenant must be resolved before joining.
    Code {
        id: String,
        passcode: Option<String>,
    },
}

impl MeetingLink {
    pub fn parse(link: &str) -> Option<Self> {
        let url = Url::parse(link.trim()).ok()?;
        if url.scheme() != "https" || !HOSTS.contains(&url.host_str()?) {
            return None;
        }
        let segments: Vec<String> = url
            .path_segments()?
            .filter(|s| !s.is_empty())
            .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
            .collect();
        let query = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
                .filter(|v| !v.is_empty())
        };
        match segments.as_slice() {
            [l, join, thread, message, ..] if l == "l" && join == "meetup-join" => {
                if !thread.starts_with("19:") {
                    return None;
                }
                let context: Value = query("context")
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or_default();
                let field = |key: &str| {
                    context
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                };
                Some(Self::Coordinates(MeetingCoordinates {
                    thread_id: thread.clone(),
                    message_id: message.clone(),
                    tenant_id: field("Tid"),
                    organizer_id: field("Oid"),
                }))
            }
            [meet, id] if meet == "meet" && !id.is_empty() => Some(Self::Code {
                id: id.clone(),
                passcode: query("p"),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meetup_join_links_with_context() {
        let link = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=%7b%22Tid%22%3a%22t1%22%2c%22Oid%22%3a%22o1%22%7d";
        assert_eq!(
            MeetingLink::parse(link),
            Some(MeetingLink::Coordinates(MeetingCoordinates {
                thread_id: "19:meeting_abc@thread.v2".into(),
                message_id: "0".into(),
                tenant_id: Some("t1".into()),
                organizer_id: Some("o1".into()),
            }))
        );
    }

    #[test]
    fn parses_meeting_codes_and_rejects_foreign_or_malformed_links() {
        assert_eq!(
            MeetingLink::parse("https://teams.microsoft.com/meet/12345?p=secret"),
            Some(MeetingLink::Code {
                id: "12345".into(),
                passcode: Some("secret".into())
            })
        );
        for link in [
            "https://evil.example/l/meetup-join/19%3ameeting_abc%40thread.v2/0",
            "http://teams.microsoft.com/meet/1",
            "https://teams.microsoft.com/l/meetup-join/not-a-thread/0",
            "https://teams.microsoft.com/l/meetup-join/",
            "not a url",
        ] {
            assert_eq!(MeetingLink::parse(link), None, "{link}");
        }
    }
}
