//! Demo notification services (properties, NSS channel settings, RCM follow states) in memory; replies push through the fake Trouter.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{Map, Value, json};
use teams_chat::Message;

#[derive(Default)]
struct Store {
    user: Map<String, Value>,
    conversations: HashMap<String, Map<String, Value>>,
    channels: HashMap<String, Value>,
    follows: HashMap<(String, String), i64>,
}

#[derive(Clone)]
struct Mock {
    store: Arc<Mutex<Store>>,
    db: crate::mock::Db,
}

pub(crate) fn router(db: crate::mock::Db) -> Router {
    let mock = Mock {
        store: Arc::default(),
        db,
    };
    Router::new()
        .route("/v1/users/ME/properties", get(user).put(set_user))
        .route("/v1/users/ME/conversations/{conv}", get(conversation))
        .route(
            "/v1/users/ME/conversations/{conv}/properties",
            put(set_conversation),
        )
        .route(
            "/v1/me/notificationSettings/team/{team}/channel/{channel}",
            get(channel).post(set_channel),
        )
        .route("/v1/me/notificationSettings/channels/batch", post(batch))
        .route(
            "/v1/users/{user}/conversations/{conv}/rcmetadata/{root}",
            put(set_follow),
        )
        .route(
            "/v1/users/{user}/conversations/{conv}/getrcmetadata",
            post(follows),
        )
        .with_state(mock)
}

async fn user(State(mock): State<Mock>) -> Json<Value> {
    Json(Value::Object(mock.store.lock().unwrap().user.clone()))
}

async fn set_user(State(mock): State<Mock>, Json(body): Json<Value>) -> StatusCode {
    if let Some(body) = body.as_object() {
        mock.store.lock().unwrap().user.extend(body.clone());
    }
    StatusCode::OK
}

async fn conversation(State(mock): State<Mock>, Path(conv): Path<String>) -> Json<Value> {
    let properties = mock
        .store
        .lock()
        .unwrap()
        .conversations
        .get(&conv)
        .cloned()
        .unwrap_or_default();
    Json(json!({"id": conv, "properties": properties}))
}

async fn set_conversation(
    State(mock): State<Mock>,
    Path(conv): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> StatusCode {
    if query.get("name").map(String::as_str) == Some("consumptionhorizon") {
        return crate::mock::mark_read(axum::extract::State(mock.db), Path(conv)).await;
    }
    if let Some(body) = body.as_object() {
        mock.store
            .lock()
            .unwrap()
            .conversations
            .entry(conv)
            .or_default()
            .extend(body.clone());
    }
    StatusCode::OK
}

async fn channel(
    State(mock): State<Mock>,
    Path((_, channel)): Path<(String, String)>,
) -> Json<Value> {
    Json(
        mock.store
            .lock()
            .unwrap()
            .channels
            .get(&channel)
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
}

async fn set_channel(
    State(mock): State<Mock>,
    Path((_, channel)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> StatusCode {
    mock.store.lock().unwrap().channels.insert(channel, body);
    StatusCode::OK
}

async fn batch(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
    let store = mock.store.lock().unwrap();
    let results: Vec<Value> = body["channelIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| {
            json!({
                "channelId": id,
                "success": true,
                "settings": store.channels.get(id).cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect();
    Json(json!({ "results": results }))
}

async fn set_follow(
    State(mock): State<Mock>,
    Path((_, conv, root)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> StatusCode {
    let state = body["f"].as_i64().unwrap_or(0);
    mock.store
        .lock()
        .unwrap()
        .follows
        .insert((conv, root), state);
    StatusCode::OK
}

async fn follows(
    State(mock): State<Mock>,
    Path((_, conv)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let store = mock.store.lock().unwrap();
    let items: Vec<Value> = body["rootMessageIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| {
            let id = match id {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => return None,
            };
            let state = store.follows.get(&(conv.clone(), id.clone()))?;
            Some(json!({"rootMessageId": id, "f": state}))
        })
        .collect();
    Json(Value::Array(items))
}

/// The demo peer starts typing (`Control/Typing`, pushed like any message).
pub(crate) fn push_typing(conv: &str, mri: &str, name: &str) {
    let now = chrono::Utc::now();
    let typing = Message {
        id: now.timestamp_millis().to_string(),
        messagetype: teams_chat::receipts::TYPING.into(),
        contenttype: Some("Application/Message".into()),
        from: format!("https://mock/v1/users/ME/contacts/{mri}"),
        imdisplayname: Some(name.into()),
        originalarrivaltime: Some(now.to_rfc3339()),
        conversation_link: Some(format!("https://mock/v1/users/ME/conversations/{conv}")),
        ..Default::default()
    };
    push_new_message(conv, &typing);
}

/// The demo peer has read up to `read` (`MemberConsumptionHorizonUpdate`).
pub(crate) fn push_horizon(conv: &str, mri: &str, read: &Message) {
    let at = read
        .originalarrivaltime
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or_default();
    let update = Message {
        id: format!("{at}1"),
        messagetype: teams_chat::receipts::HORIZON_UPDATE.into(),
        content: json!({"user": mri, "consumptionhorizon": format!("{at};{at};{}", read.id)})
            .to_string(),
        from: format!("https://mock/v1/users/ME/contacts/{mri}"),
        originalarrivaltime: read.originalarrivaltime.clone(),
        conversation_link: Some(format!("https://mock/v1/users/ME/conversations/{conv}")),
        ..Default::default()
    };
    push_new_message(conv, &update);
}

/// Pushes a new message to the demo push channel, as the chat service does through Trouter (`/messaging`, one `EventMessage`).
pub(crate) fn push_new_message(conv: &str, message: &Message) {
    let body = json!({
        "id": message.id,
        "time": message.originalarrivaltime,
        "type": "EventMessage",
        "resourceType": "NewMessage",
        "resourceLink": format!("https://mock/v1/users/ME/conversations/{conv}/messages/{}", message.id),
        "resource": message,
    });
    crate::calls_mock::trouter().push(
        "POST",
        "/messaging",
        json!({"Content-Type": "application/json"}),
        &body.to_string(),
    );
}
