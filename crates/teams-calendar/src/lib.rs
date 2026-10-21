//! Personal Exchange Online calendar, using the same regional MT protocol as Teams Web.
use http::{HeaderValue, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use teams_auth::{Authenticator, Service, net};
use teams_i18n::{t, tf};
use url::Url;

pub mod recurrence;
mod time_zones;

pub use recurrence::{Pattern, Recurrence};

/// Windows name of a time zone (`Europe/Paris` → `Romance Standard Time`), as Teams Web sends it.
pub fn windows_time_zone(name: &str) -> &str {
    time_zones::IANA_TO_WINDOWS
        .binary_search_by(|(iana, _)| iana.cmp(&name))
        .map_or(name, |i| time_zones::IANA_TO_WINDOWS[i].1)
}

/// An occurrence goes without the series' recurrence so that Exchange makes it an exception.
fn wire(event: &Event) -> Result<Value, Error> {
    let mut wire = serde_json::to_value(event).map_err(|_| Error::Decode)?;
    wire["eventTimeZone"] = json!(windows_time_zone(&event.event_time_zone));
    if let Some(map) = wire.as_object_mut() {
        if event.is_occurrence() {
            map.remove("eventRecurrencePattern");
            map.remove("eventRecurrenceRange");
        }
        map.retain(|key, value| {
            !(value.is_null() && (key == "eventRecurrencePattern" || key == "eventRecurrenceRange"))
        });
    }
    Ok(wire)
}

fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Attendee {
    #[serde(deserialize_with = "null_default")]
    pub address: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Event {
    #[serde(deserialize_with = "null_default")]
    pub object_id: String,
    #[serde(deserialize_with = "null_default")]
    pub subject: String,
    #[serde(deserialize_with = "null_default")]
    pub start_time: String,
    #[serde(deserialize_with = "null_default")]
    pub end_time: String,
    #[serde(deserialize_with = "null_default")]
    pub event_time_zone: String,
    #[serde(deserialize_with = "null_default")]
    pub event_type: String,
    #[serde(deserialize_with = "null_default")]
    pub is_all_day_event: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_organizer: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_cancelled: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_online_meeting: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_response_requested: bool,
    #[serde(deserialize_with = "null_default")]
    pub organizer_name: String,
    #[serde(deserialize_with = "null_default")]
    pub organizer_address: String,
    #[serde(deserialize_with = "null_default")]
    pub location: String,
    #[serde(deserialize_with = "null_default")]
    pub body_content: String,
    #[serde(deserialize_with = "null_default")]
    pub body_content_type: String,
    #[serde(deserialize_with = "null_default")]
    pub join_online_meeting_url: String,
    #[serde(deserialize_with = "null_default")]
    pub my_response_type: String,
    #[serde(deserialize_with = "null_default")]
    pub attendees: Vec<Attendee>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}
impl Event {
    pub fn recurring(&self) -> bool {
        matches!(
            self.event_type.as_str(),
            "Occurrence" | "Exception" | "RecurringMaster" | "SeriesMaster"
        ) || self
            .extra
            .get("eventRecurrencePattern")
            .is_some_and(|v| !v.is_null())
    }
    /// One date of a series, changed alone or not.
    pub fn is_occurrence(&self) -> bool {
        matches!(self.event_type.as_str(), "Occurrence" | "Exception")
    }
    pub fn is_series_master(&self) -> bool {
        matches!(self.event_type.as_str(), "RecurringMaster" | "SeriesMaster")
    }
    /// The id that addresses the whole series: `seriesMasterId` of an occurrence, the master's own id.
    pub fn series_id(&self) -> Option<&str> {
        if self.is_series_master() {
            return Some(&self.object_id).filter(|id| !id.is_empty()).map(String::as_str);
        }
        self.extra
            .get("seriesMasterId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }
    /// Sets, or with `None` removes, the recurrence of a new event or a series master.
    pub fn set_recurrence(&mut self, recurrence: Option<&Recurrence>) {
        match recurrence {
            Some(r) => {
                let offset = chrono::DateTime::parse_from_rfc3339(&self.start_time)
                    .map(|t| *t.offset())
                    .unwrap_or_else(|_| chrono::FixedOffset::east_opt(0).unwrap());
                self.extra
                    .insert("eventRecurrencePattern".into(), r.pattern_json());
                self.extra
                    .insert("eventRecurrenceRange".into(), r.range_json(offset));
            }
            None => {
                self.extra.remove("eventRecurrencePattern");
                self.extra.remove("eventRecurrenceRange");
            }
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        let start = chrono::DateTime::parse_from_rfc3339(&self.start_time).map_err(|_| {
            Error::Invalid(t("calendar.lib.invalid_start_date").into())
        })?;
        let end = chrono::DateTime::parse_from_rfc3339(&self.end_time)
            .map_err(|_| Error::Invalid(t("calendar.lib.invalid_end_date").into()))?;
        if self.subject.trim().is_empty() || end <= start {
            return Err(Error::Invalid(
                t("calendar.lib.enter_a_title_and_an_end")
                .into(),
            ));
        }
        if self
            .attendees
            .iter()
            .any(|a| !a.address.contains('@') || a.address.chars().any(char::is_whitespace))
        {
            return Err(Error::Invalid(
                t("calendar.lib.invalid_attendee_address")
                .into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Response {
    Accept,
    Tentative,
    Decline,
}
impl Response {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Accept => "Accept",
            Self::Tentative => "TentativelyAccept",
            Self::Decline => "Decline",
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Auth(#[from] teams_auth::AuthError),
    #[error("{}", tf!("calendar.lib.the_calendar_is_temporarily_throttled_try", seconds = .0))]
    RateLimited(u64),
    #[error("{}", tf!("calendar.lib.calendar_unavailable_http_the_mailbox_must", status = .0))]
    Http(u16),
    #[error("{}", tf!("calendar.lib.couldn_t_connect_to_the_calendar", detail = .0))]
    Transport(String),
    #[error("{}", t("calendar.lib.invalid_calendar_response"))]
    Decode,
    #[error(
        "{}",
        t("calendar.lib.the_outcome_of_the_operation_is")
    )]
    Uncertain,
    #[error(
        "{}",
        t("calendar.lib.the_online_meeting_was_created_but")
    )]
    PartialCreation,
}
teams_auth::from_http_error!(Error);
impl Error {
    pub fn uncertain(&self) -> bool {
        matches!(self, Self::Uncertain | Self::PartialCreation)
    }
}
fn invalid_url() -> Error {
    Error::Invalid(t("calendar.lib.invalid_calendar_url").into())
}
pub struct CalendarClient<A> {
    auth: Arc<A>,
    http: reqwest::Client,
    backoff: net::Backoff,
}
impl<A> Clone for CalendarClient<A> {
    fn clone(&self) -> Self {
        Self {
            auth: self.auth.clone(),
            http: self.http.clone(),
            backoff: self.backoff.clone(),
        }
    }
}
impl<A: Authenticator> CalendarClient<A> {
    pub fn new(auth: Arc<A>) -> Self {
        Self {
            auth,
            http: net::client(),
            backoff: net::Backoff::default(),
        }
    }
    fn url(&self, path: &str) -> Result<Url, Error> {
        self.auth
            .base_url(Service::Calendar)?
            .join(path)
            .map_err(|_| invalid_url())
    }
    fn event_url(&self, id: &str, suffix: &str) -> Result<Url, Error> {
        if id.is_empty() {
            return Err(Error::Invalid(
                t("calendar.lib.missing_event_id").into(),
            ));
        }
        let mut url = self.url("v2.0/me/calendars/events/")?;
        url.path_segments_mut()
            .map_err(|_| invalid_url())?
            .pop_if_empty()
            .push(id);
        if !suffix.is_empty() {
            url.path_segments_mut().unwrap().push(suffix);
        }
        Ok(url)
    }
    async fn request(&self, method: Method, url: Url, body: Option<Value>) -> Result<Value, Error> {
        let mutation = method != Method::GET;
        let request = net::Request::new(Service::Calendar, method, url).json(body.as_ref());
        let reply = net::send(
            &self.http,
            self.auth.as_ref(),
            &self.backoff,
            request,
            |headers| {
                headers.remove("x-skypetoken");
                headers.insert("x-ms-migration", HeaderValue::from_static("True"));
                headers.insert("x-ms-client-type", HeaderValue::from_static("web"));
                headers.insert(
                    "x-ms-client-version",
                    HeaderValue::from_static(teams_auth::CLIENT_VERSION),
                );
                headers.insert("x-client-ui-language", HeaderValue::from_static("fr-FR"));
            },
        )
        .await?;
        if reply.status == 204 {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&reply.body).map_err(|_| {
            if mutation {
                Error::Uncertain
            } else {
                Error::Decode
            }
        })
    }
    pub async fn list(&self, start: &str, end: &str) -> Result<Vec<Event>, Error> {
        let mut url = self.url("v2.0/me/calendars/default/calendarView")?;
        url.query_pairs_mut().extend_pairs([
            ("StartDate", start),
            ("EndDate", end),
            ("shouldDecryptData", "true"),
        ]);
        let value = self.request(Method::GET, url, None).await?;
        serde_json::from_value(value.get("value").cloned().ok_or(Error::Decode)?)
            .map_err(|_| Error::Decode)
    }
    pub async fn get(&self, id: &str) -> Result<Event, Error> {
        let mut url = self.event_url(id, "")?;
        url.query_pairs_mut()
            .append_pair("shouldDecryptData", "true");
        serde_json::from_value(self.request(Method::GET, url, None).await?)
            .map_err(|_| Error::Decode)
    }
    pub async fn create(&self, event: &Event) -> Result<Event, Error> {
        event.validate()?;
        let scheduling = if event.is_online_meeting {
            let value = self.request(Method::POST, self.url("beta/me/calendarEvents/privateMeeting/schedulingService/create")?, Some(json!({"isStreamEnabled":false,"meetingType":"Scheduled","subject":event.subject,"startTime":event.start_time,"endTime":event.end_time,"threadId":null,"meetingTemplateId":"default","useLiveEventAuthorizationV2":true}))).await?;
            Some(value.get("value").cloned().ok_or(Error::PartialCreation)?)
        } else {
            None
        };
        let mut wire = wire(event)?;
        wire.as_object_mut().unwrap().remove("objectId");
        if let Some(s) = &scheduling {
            if let Some(join) = s.pointer("/links/join") {
                wire["joinOnlineMeetingUrl"] = join.clone();
                wire["skypeTeamsMeetingUrl"] = join.clone();
            }
            if let Some(update) = s.pointer("/links/update") {
                wire["schedulingServiceUpdateUrl"] = update.clone();
            }
            if let Some(cid) = s.pointer("/groupContext/threadId") {
                wire["skypeTeamsData"] =
                    json!(json!({"cid":cid,"rid":0,"mid":0,"private":true,"type":0}).to_string());
            }
        }
        let mut url = self.url("v2.0/me/calendars/default/events")?;
        url.query_pairs_mut().extend_pairs([
            (
                "isOnlineMeeting",
                if scheduling.is_some() {
                    "true"
                } else {
                    "false"
                },
            ),
            ("shouldDecryptData", "true"),
        ]);
        let result = self.request(Method::POST, url, Some(json!({"calendarEvent":wire,"schedulingServiceEvent":scheduling,"meetingOptions":[]}))).await;
        let value = result.map_err(|e| {
            if scheduling.is_some() {
                Error::PartialCreation
            } else {
                e
            }
        })?;
        serde_json::from_value(value.get("value").cloned().ok_or(Error::Uncertain)?)
            .map_err(|_| Error::Uncertain)
    }
    pub async fn update(&self, event: &Event) -> Result<(), Error> {
        event.validate()?;
        if !event.is_organizer || event.is_cancelled {
            return Err(Error::Invalid(
                t("calendar.lib.this_event_can_only_be_managed")
                .into(),
            ));
        }
        let mut url = self.event_url(&event.object_id, "")?;
        url.query_pairs_mut().append_pair(
            "isOnlineMeeting",
            if event.is_online_meeting {
                "true"
            } else {
                "false"
            },
        );
        self.request(
            Method::PUT,
            url,
            Some(json!({"updatedCalendarEvent": wire(event)?})),
        )
        .await
        .map(drop)
    }
    pub async fn delete(&self, id: &str) -> Result<(), Error> {
        self.request(Method::DELETE, self.event_url(id, "")?, None)
            .await
            .map(drop)
    }
    pub async fn cancel(&self, event: &Event) -> Result<(), Error> {
        let mut body = json!({"cancellationMessage":""});
        if let Some(cid) = event
            .extra
            .get("skypeTeamsData")
            .and_then(Value::as_str)
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .and_then(|data| data.get("cid").cloned())
        {
            body["conversationId"] = cid;
        }
        self.request(
            Method::PUT,
            self.event_url(&event.object_id, "cancel")?,
            Some(body),
        )
        .await
        .map(drop)
    }
    pub async fn respond(&self, id: &str, response: Response) -> Result<(), Error> {
        self.respond_with(id, response, true).await
    }
    /// `notify: false` doesn't tell the organizer, as the web client does for the exception it answers before its series.
    pub async fn respond_with(&self, id: &str, response: Response, notify: bool) -> Result<(), Error> {
        self.request(Method::PUT, self.event_url(id, "respond")?, Some(json!({"responseType":response.wire(),"sendResponse":notify,"comment":"","attendance":"None"}))).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iana_time_zones_become_windows_names() {
        assert!(time_zones::IANA_TO_WINDOWS.is_sorted_by_key(|(iana, _)| *iana));
        assert_eq!(windows_time_zone("Europe/Paris"), "Romance Standard Time");
        assert_eq!(
            windows_time_zone("America/New_York"),
            "Eastern Standard Time"
        );
        assert_eq!(windows_time_zone("Etc/UTC"), "UTC");
        assert_eq!(
            windows_time_zone("Romance Standard Time"),
            "Romance Standard Time"
        );
    }
}
