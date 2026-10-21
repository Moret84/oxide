//! Unified Presence Service (UPS): other people's presence, the user's own status and status note.

use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde::Deserialize;
use serde_json::json;
use teams_auth::net::{self, Payload, Signer};
use teams_auth::{Authenticator, Service};
use url::Url;

use crate::client::{ChatClient, regional_base};
use crate::error::Error;

const GET_PRESENCE: &str = "v1/presence/getpresence/";
const FORCE_AVAILABILITY: &str = "v1/me/forceavailability/";
const PUBLISH_NOTE: &str = "v1/me/publishnote";
const ENDPOINTS: &str = "v1/me/endpoints/";
const REPORT_ACTIVITY: &str = "v1/me/reportmyactivity/";
/// `deviceType` of a native client; the web client uses `Web` in a browser.
const DEVICE_TYPE: &str = "Desktop";
/// `statusNoteLimit`.
pub const NOTE_LIMIT: usize = 2000;
/// `batchedUsersPresenceChunkSize`.
const CHUNK: usize = 650;
/// `redirectPolicy.maxRedirects`.
const MAX_REDIRECTS: usize = 10;
/// `status` of an entry whose presence is known (`20000`).
const OK: u32 = 20000;

/// Presence of one person, as UPS reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presence {
    pub mri: String,
    /// `Available`, `AvailableIdle`, `Away`, `Busy`, `BusyIdle`, `DoNotDisturb`, `BeRightBack`, `Offline` or `PresenceUnknown`.
    pub availability: String,
    /// `InAMeeting`, `InACall`, `Presenting`, `OffWork`…
    pub activity: Option<String>,
    /// `calendarData.isOutOfOffice`.
    pub out_of_office: bool,
    /// The status note, sanitized HTML, when one is set.
    pub note: Option<String>,
    /// When the note ends (ISO 8601); far in the future for "never".
    pub note_expiry: Option<String>,
    /// When a status chosen by the user ends (`forcedAvailability.expiry`), if one is set.
    pub forced_expiry: Option<String>,
    /// `calendarData.outOfOfficeNote.message`.
    pub out_of_office_note: Option<String>,
}

#[derive(Deserialize)]
struct Entry {
    mri: String,
    #[serde(default)]
    status: Option<u32>,
    #[serde(default)]
    presence: Option<EntryPresence>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryPresence {
    availability: Option<String>,
    activity: Option<String>,
    calendar_data: Option<CalendarData>,
    note: Option<Note>,
    forced_availability: Option<Forced>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarData {
    #[serde(default)]
    is_out_of_office: bool,
    out_of_office_note: Option<Note>,
}
#[derive(Deserialize)]
struct Note {
    message: Option<String>,
    expiry: Option<String>,
}
#[derive(Deserialize)]
struct Forced {
    expiry: Option<String>,
}

/// A status the user can choose, as `forceavailability` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChosenAvailability {
    Available,
    Busy,
    DoNotDisturb,
    BeRightBack,
    Away,
    /// "Appear offline": sent with `activity: OffWork`.
    Offline,
}
impl ChosenAvailability {
    fn wire(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Busy => "Busy",
            Self::DoNotDisturb => "DoNotDisturb",
            Self::BeRightBack => "BeRightBack",
            Self::Away => "Away",
            Self::Offline => "Offline",
        }
    }
}
/// Body of `forceavailability`: `activity` only for Offline, no `desiredExpirationTime` when it never ends.
fn force_availability_body(
    availability: ChosenAvailability,
    expires: Option<&str>,
) -> serde_json::Value {
    let mut body = json!({ "availability": availability.wire() });
    if availability == ChosenAvailability::Offline {
        body["activity"] = json!("OffWork");
    }
    if let Some(expires) = expires {
        body["desiredExpirationTime"] = json!(expires);
    }
    body
}

impl<A: Authenticator> ChatClient<A> {
    /// Presence of `mris`, in batches of 650; entries UPS has no presence for are left out.
    pub async fn presences(&self, mris: &[String]) -> Result<Vec<Presence>, Error> {
        let mut found = Vec::new();
        for chunk in mris.chunks(CHUNK) {
            let body = chunk
                .iter()
                .map(|mri| json!({ "mri": mri, "source": "ups" }))
                .collect();
            let bytes = self
                .presence_request(
                    Method::POST,
                    GET_PRESENCE,
                    Some(body),
                    None,
                )
                .await?;
            let entries: Vec<Entry> =
                serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
                    endpoint: "getPresence",
                    source,
                })?;
            found.extend(entries.into_iter().filter_map(|entry| {
                if entry.status.is_some_and(|s| s != OK) {
                    return None;
                }
                let presence = entry.presence?;
                Some(Presence {
                    mri: entry.mri,
                    availability: presence.availability?,
                    activity: presence.activity,
                    out_of_office: presence
                        .calendar_data
                        .as_ref()
                        .is_some_and(|c| c.is_out_of_office),
                    out_of_office_note: presence
                        .calendar_data
                        .and_then(|c| c.out_of_office_note?.message)
                        .filter(|m| !m.trim().is_empty()),
                    note_expiry: presence.note.as_ref().and_then(|n| n.expiry.clone()),
                    note: presence
                        .note
                        .and_then(|n| n.message)
                        .filter(|m| !m.trim().is_empty()),
                    forced_expiry: presence.forced_availability.and_then(|f| f.expiry),
                })
            }));
        }
        Ok(found)
    }

    /// Registers a presence endpoint: UPS keeps the user online and a chosen status only while one lives.
    pub async fn publish_endpoint(&self, endpoint: &str) -> Result<(), Error> {
        let body = json!({
            "id": endpoint,
            "availability": "Available",
            "activity": "Available",
            "activityReporting": "Direct",
            "deviceType": DEVICE_TYPE,
        });
        self.presence_request(
            Method::PUT,
            ENDPOINTS,
            Some(body),
            Some(endpoint),
        )
        .await
        .map(drop)
    }
    /// The endpoint's heartbeat; a 404 means UPS forgot the endpoint, which must be published again.
    pub async fn report_my_activity(&self, endpoint: &str, active: bool) -> Result<(), Error> {
        let body = json!({ "endpointId": endpoint, "isActive": active });
        self.presence_request(
            Method::POST,
            REPORT_ACTIVITY,
            Some(body),
            Some(endpoint),
        )
        .await
        .map(drop)
    }
    /// Removes the endpoint when the app signs out.
    pub async fn delete_presence_endpoint(&self, endpoint: &str) -> Result<(), Error> {
        let path = format!("{ENDPOINTS}{endpoint}");
        self.presence_request(Method::DELETE, &path, None, Some(endpoint))
            .await
            .map(drop)
    }
    /// Sets the user's own status until `expires` (ISO 8601), or for good with `None`.
    pub async fn set_my_availability(
        &self,
        availability: ChosenAvailability,
        expires: Option<&str>,
        endpoint: Option<&str>,
    ) -> Result<(), Error> {
        let body = force_availability_body(availability, expires);
        self.presence_request(
            Method::PUT,
            FORCE_AVAILABILITY,
            Some(body),
            endpoint,
        )
        .await
        .map(drop)
    }
    /// "Reset status" (computed from activity and calendar): the web client's PUT without a body.
    pub async fn reset_my_availability(&self, endpoint: Option<&str>) -> Result<(), Error> {
        self.presence_request(Method::PUT, FORCE_AVAILABILITY, None, endpoint)
            .await
            .map(drop)
    }
    /// Publishes the status note (HTML) until `expiry`; an empty message clears it.
    pub async fn publish_note(
        &self,
        message: &str,
        expiry: Option<&str>,
        endpoint: Option<&str>,
    ) -> Result<(), Error> {
        let mut body = json!({ "message": message });
        if let Some(expiry) = expiry.filter(|_| !message.is_empty()) {
            body["expiry"] = json!(expiry);
        }
        self.presence_request(
            Method::PUT,
            PUBLISH_NOTE,
            Some(body),
            endpoint,
        )
        .await
        .map(drop)
    }

    /// Follows Front Door redirects like the web client, keeping the redirect's `/ups/<x>/` base.
    async fn presence_request(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        endpoint: Option<&str>,
    ) -> Result<Vec<u8>, Error> {
        let body = body.map(|body| serde_json::to_vec(&body).unwrap());
        let headers = || {
            let mut headers = HeaderMap::new();
            if body.is_some() {
                headers.insert("content-type", HeaderValue::from_static("application/json"));
            }
            headers.insert("x-ms-client-type", HeaderValue::from_static("web"));
            headers.insert(
                "x-ms-client-version",
                HeaderValue::from_static(teams_auth::CLIENT_BUILD),
            );
            headers.insert(
                "x-ms-session-id",
                HeaderValue::from_str(&self.session_id).unwrap(),
            );
            headers.insert(
                "x-ms-client-user-agent",
                HeaderValue::from_static(if path == GET_PRESENCE {
                    "Teams-V2-Web"
                } else {
                    "Teams-V2-Desktop"
                }),
            );
            if let Some(endpoint) = endpoint.and_then(|e| HeaderValue::from_str(e).ok()) {
                headers.insert("x-ms-endpoint-id", endpoint);
            }
            headers.insert(
                "x-ms-correlation-id",
                HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()).unwrap(),
            );
            headers.insert(
                "behavioroverride",
                HeaderValue::from_static("redirectAs404"),
            );
            headers
        };
        let url = || {
            let base = match self.presence_base.read().unwrap().clone() {
                Some(base) => base,
                None => self.auth.base_url(Service::UnifiedPresence)?,
            };
            Ok(Url::parse(&format!("{}/{path}", base.as_str().trim_end_matches('/')))?)
        };
        let follow = |status: StatusCode, headers: &HeaderMap| {
            if !status.is_redirection() && status != StatusCode::NOT_FOUND {
                return Ok(false);
            }
            let Some(base) = headers
                .get("location")
                .and_then(|v| v.to_str().ok())
                .and_then(ups_base)
            else {
                return Ok(false);
            };
            *self.presence_base.write().unwrap() = Some(base);
            Ok(true)
        };
        let request = net::Exchange::new(Signer::Service(Service::UnifiedPresence), method, &url)
            .headers(&headers)
            .payload(body.as_deref().map_or(Payload::Empty, Payload::Bytes))
            .timeout(Some(std::time::Duration::from_secs(30)))
            .redirects(MAX_REDIRECTS, &follow);
        let response = self.exchange(Service::UnifiedPresence, request).await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(Error::Status {
                endpoint: match path {
                    GET_PRESENCE => "getPresence",
                    REPORT_ACTIVITY => "reportMyActivity",
                    FORCE_AVAILABILITY => "setMyStatus",
                    PUBLISH_NOTE => "publishNote",
                    _ => "endpoints",
                },
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(bytes.to_vec())
    }
}

/// `https://teams.microsoft.com/ups/emea/v1/…` → `…/ups/emea/`, on a Teams host only (bearer).
fn ups_base(location: &str) -> Option<Url> {
    let trusted = regional_base(location)?;
    let url = Url::parse(location).ok()?;
    let mut segments = url.path().trim_start_matches('/').split('/');
    let (ups, region) = (segments.next()?, segments.next()?);
    if !ups.starts_with("ups") || region.is_empty() || segments.next().is_none() {
        return None;
    }
    trusted.join(&format!("/{ups}/{region}/")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chosen_status_bodies_follow_the_web_client_rules() {
        assert_eq!(
            force_availability_body(ChosenAvailability::Busy, Some("2026-10-14T18:30:00.000Z")),
            json!({"availability":"Busy","desiredExpirationTime":"2026-10-14T18:30:00.000Z"})
        );
        assert_eq!(
            force_availability_body(ChosenAvailability::Offline, None),
            json!({"availability":"Offline","activity":"OffWork"})
        );
    }
    #[test]
    fn front_door_redirects_keep_the_ups_prefix_on_teams_hosts_only() {
        assert_eq!(
            ups_base("https://teams.microsoft.com/ups/emea/v1/presence/getpresence/")
                .unwrap()
                .as_str(),
            "https://teams.microsoft.com/ups/emea/"
        );
        assert!(ups_base("https://teams.microsoft.com/api/chatsvc/emea/v1/").is_none());
        assert!(ups_base("https://attacker.example/ups/emea/v1/").is_none());
    }
}
