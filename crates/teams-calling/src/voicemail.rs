//! Voicemail, stored in Exchange and read through Outlook REST v2.0.
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};
use teams_i18n::t;
use url::Url;

use crate::Error;
use crate::http::Http;

const CLASSES: [&str; 5] = [
    "IPM.Note.Microsoft.Voicemail.UM.CA",
    "IPM.Note.rpmsg.Microsoft.Voicemail.UM.CA",
    "IPM.Note.Microsoft.Voicemail.UM",
    "IPM.Note.Microsoft.Exchange.Voice.UM",
    "IPM.Note.Microsoft.Exchange.Voice.UM.CA",
];
const DURATION: &str = "Integer {00020328-0000-0000-c000-000000000046} Id 0x6801";
const TRANSCRIPTION: &str =
    "String {00020386-0000-0000-C000-000000000046} Name X-VoiceMessageTranscription";
const CONFIDENCE: &str =
    "String {00020386-0000-0000-C000-000000000046} Name X-VoiceMessageConfidenceLevel";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voicemail {
    pub id: String,
    pub sender_name: Option<String>,
    /// Caller address: SIP/SMTP address or phone number.
    pub sender_address: Option<String>,
    pub received: Option<String>,
    pub is_read: bool,
    pub duration_secs: Option<u32>,
    pub transcription: Option<String>,
    pub call_id: Option<String>,
}

impl Voicemail {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let header = |name: &str| {
            value
                .get("InternetMessageHeaders")?
                .as_array()?
                .iter()
                .find(|h| {
                    h.get("Name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .and_then(|h| text(h.get("Value")))
        };
        let property = |id: &str| {
            value
                .get("SingleValueExtendedProperties")?
                .as_array()?
                .iter()
                .find(|p| {
                    p.get("PropertyId")
                        .and_then(Value::as_str)
                        .is_some_and(|p| p.eq_ignore_ascii_case(id))
                })
                .and_then(|p| text(p.get("Value")))
        };
        let address = value.get("From").and_then(|f| f.get("EmailAddress"));
        Some(Self {
            id: text(value.get("Id"))?,
            sender_name: header("X-VoiceMessageSenderName")
                .or_else(|| text(address.and_then(|a| a.get("Name")))),
            sender_address: header("X-CallingTelephoneNumber")
                .or_else(|| text(address.and_then(|a| a.get("Address")))),
            received: text(value.get("ReceivedDateTime")),
            is_read: value
                .get("IsRead")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            duration_secs: property(DURATION).and_then(|d| d.parse().ok()),
            transcription: property(TRANSCRIPTION),
            call_id: header("CallID"),
        })
    }
}

pub struct VoicemailClient<A> {
    http: Http<A>,
}
impl<A> Clone for VoicemailClient<A> {
    fn clone(&self) -> Self {
        Self {
            http: self.http.clone(),
        }
    }
}

impl<A: Authenticator> VoicemailClient<A> {
    pub fn new(auth: Arc<A>) -> Self {
        Self {
            http: Http::new(auth),
        }
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.http
            .auth
            .base_url(Service::Outlook)?
            .join("api/v2.0/me/")
            .and_then(|base| base.join(path))
            .map_err(|_| Error::Invalid(t("calling.voicemail.invalid_outlook_url").into()))
    }

    fn message_url(&self, id: &str, suffix: &str) -> Result<Url, Error> {
        if id.is_empty() {
            return Err(Error::Invalid(
                t("calling.voicemail.voicemail_not_found").into(),
            ));
        }
        let mut url = self.url("messages")?;
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                Error::Invalid(t("calling.voicemail.invalid_outlook_url").into())
            })?;
            segments.push(id);
            if !suffix.is_empty() {
                segments.push(suffix);
            }
        }
        Ok(url)
    }

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "accept",
            HeaderValue::from_static("application/json;odata.metadata=none"),
        );
        if let Ok(id) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
            headers.insert("client-request-id", id);
        }
        headers
    }

    /// Latest voicemails, newest first.
    pub async fn list(&self, top: u32) -> Result<Vec<Voicemail>, Error> {
        let classes = CLASSES
            .iter()
            .map(|c| {
                format!(
                    "SingleValueExtendedProperties/Any(ep: ep/PropertyId eq 'String 0x1a' and ep/Value eq '{c}')"
                )
            })
            .collect::<Vec<_>>()
            .join(" or ");
        let mut url = self.url("messages")?;
        url.query_pairs_mut()
            .append_pair(
                "$expand",
                &format!(
                    "SingleValueExtendedProperties($filter=PropertyId eq '{DURATION}' or PropertyId eq '{TRANSCRIPTION}' or PropertyId eq '{CONFIDENCE}')"
                ),
            )
            .append_pair("$top", &top.to_string())
            .append_pair(
                "$select",
                "From,Body,IsRead,Id,ReceivedDateTime,InternetMessageHeaders,InternetMessageId",
            )
            .append_pair(
                "$filter",
                &format!("({classes}) and parentFolderId ne 'deleteditems'"),
            );
        let value = self
            .http
            .json(Service::Outlook, Method::GET, url, None, Self::headers())
            .await?;
        let mut list: Vec<Voicemail> = value
            .get("value")
            .and_then(Value::as_array)
            .ok_or(Error::Decode)?
            .iter()
            .filter_map(Voicemail::from_value)
            .collect();
        // ISO-8601 UTC timestamps sort lexicographically.
        list.sort_by(|a, b| b.received.cmp(&a.received));
        Ok(list)
    }

    /// Audio of a voicemail: its first audio attachment, decoded.
    pub async fn audio(&self, id: &str) -> Result<(String, Vec<u8>), Error> {
        let url = self.message_url(id, "attachments")?;
        let value = self
            .http
            .json(Service::Outlook, Method::GET, url, None, Self::headers())
            .await?;
        let attachments = value
            .get("value")
            .and_then(Value::as_array)
            .ok_or(Error::Decode)?;
        let audio = attachments
            .iter()
            .find(|a| {
                a.get("ContentType")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.starts_with("audio/"))
            })
            .or_else(|| attachments.first())
            .ok_or_else(|| {
                Error::Invalid(
                    t("calling.voicemail.this_voicemail_has_no_audio")
                    .into(),
                )
            })?;
        let content_type = audio
            .get("ContentType")
            .and_then(Value::as_str)
            .unwrap_or("audio/mpeg")
            .to_owned();
        let bytes = audio
            .get("ContentBytes")
            .and_then(Value::as_str)
            .and_then(|b| STANDARD.decode(b).ok())
            .ok_or(Error::Decode)?;
        Ok((content_type, bytes))
    }

    pub async fn set_read(&self, id: &str, read: bool) -> Result<(), Error> {
        let url = self.message_url(id, "")?;
        self.http
            .json(
                Service::Outlook,
                Method::PATCH,
                url,
                Some(json!({"IsRead": read})),
                Self::headers(),
            )
            .await
            .map(drop)
    }

    /// Moves the voicemail to Deleted Items, like the web client.
    pub async fn delete(&self, id: &str) -> Result<(), Error> {
        let url = self.message_url(id, "move")?;
        self.http
            .json(
                Service::Outlook,
                Method::POST,
                url,
                Some(json!({"DestinationId": "DeletedItems"})),
                Self::headers(),
            )
            .await
            .map(drop)
    }
}
