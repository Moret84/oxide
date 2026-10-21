//! Stateful calendar fixtures served through the real calendar client's HTTP routes.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{Duration, TimeZone, Utc};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
type Db = Arc<Mutex<Vec<Value>>>;
pub fn router() -> Router {
    Router::new()
        .route("/v2.0/me/calendars/default/calendarView", get(list))
        .route("/v2.0/me/calendars/default/events", post(create))
        .route(
            "/v2.0/me/calendars/events/{id}",
            get(detail).put(update).delete(delete),
        )
        .route("/v2.0/me/calendars/events/{id}/cancel", put(cancel))
        .route("/v2.0/me/calendars/events/{id}/respond", put(respond))
        .route(
            "/beta/me/calendarEvents/privateMeeting/schedulingService/create",
            post(schedule),
        )
        .with_state(Arc::new(Mutex::new(seed())))
}
fn seed() -> Vec<Value> {
    let day = chrono::Local::now().date_naive();
    let at = |offset: i64, hour: u32| {
        chrono::Local
            .from_local_datetime(
                &(day + Duration::days(offset))
                    .and_hms_opt(hour, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
            .with_timezone(&Utc)
            .to_rfc3339()
    };
    let mut events = vec![
        json!({"objectId":"demo-planning","subject":"Planning produit","startTime":at(0,9),"endTime":at(0,10),"isOrganizer":true,"isOnlineMeeting":true,"joinOnlineMeetingUrl":"https://teams.microsoft.com/l/meetup-join/19%3ameeting_demo%40thread.v2/0?context=%7b%22Tid%22%3a%22demo-tenant%22%2c%22Oid%22%3a%22demo-organizer%22%7d","organizerName":"John Doe","bodyContent":"Préparer les prochaines étapes du projet.","bodyContentType":"Text","location":"Salle Indigo","eventType":"Single","attendees":[{"address":"alice@example.com","name":"Alice Martin","type":"Required"}]}),
        json!({"objectId":"demo-review","subject":"Revue de conception","startTime":at(0,9),"endTime":at(0,11),"isOrganizer":false,"isResponseRequested":true,"organizerName":"Alice Martin","myResponseType":"NotResponded","eventType":"Single","attendees":[]}),
        json!({"objectId":"demo-allday","subject":"Journée équipe","startTime":at(1,0),"endTime":at(2,0),"isAllDayEvent":true,"isOrganizer":true,"eventType":"Single","attendees":[]}),
        json!({"objectId":"demo-multiday","subject":"Conférence développeurs","startTime":at(2,14),"endTime":at(4,16),"isOrganizer":false,"eventType":"Single","attendees":[]}),
        json!({"objectId":"demo-recurring","subject":"Point hebdomadaire","startTime":at(1,14),"endTime":at(1,15),"isOrganizer":true,"eventType":"Occurrence","attendees":[]}),
    ];
    // Explicit fixtures on the Paris DST transition, in addition to relative dates.
    {
        let (id, start, end) = ("dst", "2026-10-25T00:30:00Z", "2026-10-25T02:30:00Z");
        events.push(json!({"objectId":id,"subject":"Changement d’heure","startTime":start,"endTime":end,"eventType":"Single","attendees":[]}));
    }
    events
}
async fn list(State(db): State<Db>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let start = q
        .get("StartDate")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let end = q
        .get("EndDate")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let events: Vec<_> = db
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            let s = e["startTime"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            let t = e["endTime"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            s.zip(t)
                .zip(start.zip(end))
                .is_some_and(|((s, t), (from, to))| s < to && t > from)
        })
        .cloned()
        .collect();
    Json(json!({"value":events}))
}
async fn detail(State(db): State<Db>, Path(id): Path<String>) -> Result<Json<Value>, StatusCode> {
    db.lock()
        .unwrap()
        .iter()
        .find(|e| e["objectId"] == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
async fn create(State(db): State<Db>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut e = body["calendarEvent"].clone();
    let mut db = db.lock().unwrap();
    e["objectId"] = json!(format!(
        "new-{}-{}",
        db.len(),
        Utc::now().timestamp_millis()
    ));
    db.push(e.clone());
    (StatusCode::CREATED, Json(json!({"value":e})))
}
async fn update(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> StatusCode {
    let mut db = db.lock().unwrap();
    if let Some(e) = db.iter_mut().find(|e| e["objectId"] == id) {
        *e = body["updatedCalendarEvent"].clone();
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
async fn delete(State(db): State<Db>, Path(id): Path<String>) -> StatusCode {
    db.lock().unwrap().retain(|e| e["objectId"] != id);
    StatusCode::NO_CONTENT
}
async fn cancel(State(db): State<Db>, Path(id): Path<String>) -> StatusCode {
    if let Some(e) = db.lock().unwrap().iter_mut().find(|e| e["objectId"] == id) {
        e["isCancelled"] = json!(true);
    }
    StatusCode::NO_CONTENT
}
async fn respond(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> StatusCode {
    if let Some(e) = db.lock().unwrap().iter_mut().find(|e| e["objectId"] == id) {
        e["myResponseType"] = json!(match body["responseType"].as_str() {
            Some("Accept") => "Accepted",
            Some("Decline") => "Declined",
            _ => "Tentative",
        });
    }
    StatusCode::NO_CONTENT
}
async fn schedule(Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::CREATED,
        Json(
            json!({"value":{"subject":body["subject"],"startTime":body["startTime"],"endTime":body["endTime"],"groupContext":{"threadId":"19:meeting_demo@thread.v2"},"links":{"join":"https://teams.microsoft.com/v2/#/calendarv2","update":"demo-update"}}}),
        ),
    )
}
