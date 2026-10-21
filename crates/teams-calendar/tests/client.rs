use http::{HeaderMap, HeaderValue};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use teams_auth::{AuthError, Authenticator, Service};
use teams_calendar::{CalendarClient, Error, Event, Response};
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path, query_param},
};
struct Auth {
    base: Url,
    refreshes: AtomicUsize,
}
impl Authenticator for Auth {
    fn base_url(&self, service: Service) -> Result<Url, AuthError> {
        assert_eq!(service, Service::Calendar);
        Ok(self.base.clone())
    }
    async fn authorize(&self, service: Service, h: &mut HeaderMap) -> Result<(), AuthError> {
        assert_eq!(service, Service::Calendar);
        h.insert(
            "authorization",
            HeaderValue::from_static(if self.refreshes.load(Ordering::SeqCst) == 0 {
                "Bearer first"
            } else {
                "Bearer refreshed"
            }),
        );
        // The calendar client also strips legacy tokens from test authenticators.
        h.insert("x-skypetoken", HeaderValue::from_static("legacy"));
        Ok(())
    }
    async fn on_unauthorized(&self, service: Service) -> bool {
        assert_eq!(service, Service::Calendar);
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        true
    }
}
fn setup(server: &MockServer) -> (CalendarClient<Auth>, Arc<Auth>) {
    let auth = Arc::new(Auth {
        base: Url::parse(&format!("{}/mt/", server.uri())).unwrap(),
        refreshes: AtomicUsize::new(0),
    });
    (CalendarClient::new(auth.clone()), auth)
}
fn event() -> Event {
    serde_json::from_value(json!({"objectId":"a/b+=?","subject":"Projet","startTime":"2026-10-25T00:30:00Z","endTime":"2026-10-25T02:30:00Z","isOrganizer":true,"eventType":"Single","bodyContent":"Texte","bodyContentType":"Text","skypeTeamsData":"{\"cid\":\"19:meeting@thread.v2\"}","schedulingServiceUpdateUrl":"keep-me","attendees":[{"address":"alice@example.com","type":"Required","status":{"response":"Accepted"}}]})).unwrap()
}
#[tokio::test]
async fn list_uses_stm_headers_query_and_regional_base() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    Mock::given(method("GET"))
        .and(path("/mt/v2.0/me/calendars/default/calendarView"))
        .and(query_param("StartDate", "2026-10-25T00:00:00Z"))
        .and(query_param("EndDate", "2026-10-26T00:00:00Z"))
        .and(query_param("shouldDecryptData", "true"))
        .and(header("x-ms-migration", "True"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value":[event()]})))
        .expect(1)
        .mount(&server)
        .await;
    let events = client
        .list("2026-10-25T00:00:00Z", "2026-10-26T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    let requests = server.received_requests().await.unwrap();
    let r = &requests[0];
    assert!(!r.headers.contains_key("x-skypetoken"));
    assert_eq!(r.headers["authorization"], "Bearer first");
}
#[tokio::test]
async fn get_encodes_event_id_and_update_preserves_metadata() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    let e = event();
    Mock::given(method("GET"))
        .and(path("/mt/v2.0/me/calendars/events/a%2Fb+=%3F"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&e))
        .expect(1)
        .mount(&server)
        .await;
    let loaded = client.get(&e.object_id).await.unwrap();
    assert_eq!(loaded, e);
    Mock::given(method("PUT")).and(path("/mt/v2.0/me/calendars/events/a%2Fb+=%3F")).and(body_partial_json(json!({"updatedCalendarEvent":{"schedulingServiceUpdateUrl":"keep-me","attendees":[{"address":"alice@example.com","type":"Required","status":{"response":"Accepted"}}]}}))).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    client.update(&loaded).await.unwrap();
}
#[tokio::test]
async fn create_links_scheduling_event_before_exchange() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    Mock::given(method("POST")).and(path("/mt/beta/me/calendarEvents/privateMeeting/schedulingService/create")).and(body_partial_json(json!({"meetingType":"Scheduled","threadId":null,"subject":"Projet"}))).respond_with(ResponseTemplate::new(201).set_body_json(json!({"value":{"links":{"join":"https://teams.microsoft.com/l/meetup-join/example","update":"schedule-update"},"groupContext":{"threadId":"thread-id"}}}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/mt/v2.0/me/calendars/default/events")).and(query_param("isOnlineMeeting","true")).and(body_partial_json(json!({"calendarEvent":{"schedulingServiceUpdateUrl":"schedule-update","joinOnlineMeetingUrl":"https://teams.microsoft.com/l/meetup-join/example"},"schedulingServiceEvent":{"groupContext":{"threadId":"thread-id"}},"meetingOptions":[]}))).respond_with(ResponseTemplate::new(201).set_body_json(json!({"value":event()}))).expect(1).mount(&server).await;
    let mut e = event();
    e.object_id.clear();
    e.is_online_meeting = true;
    client.create(&e).await.unwrap();
}
#[tokio::test]
async fn partial_creation_is_reported_and_never_retried() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    Mock::given(method("POST"))
        .and(path(
            "/mt/beta/me/calendarEvents/privateMeeting/schedulingService/create",
        ))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"value":{"links":{"join":"url"}}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mt/v2.0/me/calendars/default/events"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let mut e = event();
    e.is_online_meeting = true;
    assert!(matches!(
        client.create(&e).await,
        Err(Error::PartialCreation)
    ));
}
#[tokio::test]
async fn mutations_handle_empty_responses_and_rsvp_wire_values() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    let mut e = event();
    e.object_id = "id".into();
    Mock::given(method("DELETE"))
        .and(path("/mt/v2.0/me/calendars/events/id"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/mt/v2.0/me/calendars/events/id/cancel"))
        .and(body_partial_json(
            json!({"conversationId":"19:meeting@thread.v2"}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client.delete("id").await.unwrap();
    client.cancel(&e).await.unwrap();
    for response in [Response::Accept, Response::Tentative, Response::Decline] {
        Mock::given(method("PUT"))
            .and(path("/mt/v2.0/me/calendars/events/id/respond"))
            .and(body_partial_json(
                json!({"responseType":response.wire(),"sendResponse":true}),
            ))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        client.respond("id", response).await.unwrap();
    }
}
#[tokio::test]
async fn unauthorized_refreshes_once_and_rate_limit_prevents_more_requests() {
    let server = MockServer::start().await;
    let (client, auth) = setup(&server);
    Mock::given(header("authorization", "Bearer first"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(header("authorization", "Bearer refreshed"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        client.get("id").await,
        Err(Error::RateLimited(120))
    ));
    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
    assert!(matches!(client.get("id").await, Err(Error::RateLimited(_))));
}
#[tokio::test]
async fn invalid_drafts_and_attendee_updates_never_reach_network() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    let mut e = event();
    e.end_time = e.start_time.clone();
    assert!(client.create(&e).await.is_err());
    e = event();
    e.is_organizer = false;
    assert!(client.update(&e).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}
#[tokio::test]
async fn occurrences_go_without_the_series_pattern_and_masters_with_it() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    Mock::given(method("PUT"))
        .and(path("/mt/v2.0/me/calendars/events/a%2Fb+=%3F"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    let pattern = json!({"patternType": 1, "daily": {"interval": 1}});
    let mut occurrence = event();
    occurrence.event_type = "Occurrence".into();
    occurrence.extra.insert("eventRecurrencePattern".into(), pattern.clone());
    client.update(&occurrence).await.unwrap();
    let mut master = event();
    master.event_type = "RecurringMaster".into();
    master.extra.insert("eventRecurrencePattern".into(), pattern.clone());
    client.update(&master).await.unwrap();
    let bodies: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert!(bodies[0]["updatedCalendarEvent"].get("eventRecurrencePattern").is_none());
    assert_eq!(bodies[1]["updatedCalendarEvent"]["eventRecurrencePattern"], pattern);
}

#[test]
fn nullable_fields_and_unknown_metadata_are_tolerated() {
    let e:Event=serde_json::from_value(json!({"objectId":"id","subject":null,"location":null,"attendees":null,"isOnlineMeeting":null,"sensitivityLabelId":"retain"})).unwrap();
    assert_eq!(e.subject, "");
    assert!(e.attendees.is_empty());
    assert_eq!(
        serde_json::to_value(e).unwrap()["sensitivityLabelId"],
        "retain"
    );
}
#[tokio::test]
async fn offline_creation_skips_scheduling_and_mailbox_denial_is_explicit() {
    let server = MockServer::start().await;
    let (client, _) = setup(&server);
    Mock::given(method("POST"))
        .and(path("/mt/v2.0/me/calendars/default/events"))
        .and(query_param("isOnlineMeeting", "false"))
        .and(body_partial_json(json!({"schedulingServiceEvent":null})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"value":event()})))
        .expect(1)
        .mount(&server)
        .await;
    client.create(&event()).await.unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(client.get("id").await, Err(Error::Http(403))));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
