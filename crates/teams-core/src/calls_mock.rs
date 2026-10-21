//! Demo call history and voicemail, served through the real clients' HTTP routes.
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    routing::{get, post},
};
use base64::Engine;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Deleted call logs and voicemails, shared with the chat mock, which owns the route that deletes a call log.
fn deleted() -> &'static Mutex<HashSet<String>> {
    static DELETED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DELETED.get_or_init(Default::default)
}

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/calling/users/me/callhistory", get(history))
        .route("/api/v2.0/me/messages", get(voicemails))
        .route("/api/v2.0/me/messages/{id}", axum::routing::patch(ok))
        .route("/api/v2.0/me/messages/{id}/attachments", get(attachments))
        .route("/api/v2.0/me/messages/{id}/move", post(delete_voicemail))
}

/// `PUT …/48:calllogs/messages/{id}/properties?name=call-log` in the chat mock.
pub fn delete_log(id: &str) -> StatusCode {
    deleted().lock().unwrap().insert(id.to_owned());
    StatusCode::OK
}

fn at(minutes_ago: i64) -> String {
    (Utc::now() - Duration::minutes(minutes_ago)).to_rfc3339()
}

async fn history() -> Json<Value> {
    let me = json!({"id": crate::mock::ME, "displayName": crate::mock::MY_NAME});
    let person = |id: &str, name: &str| json!({"id": format!("8:orgid:{id}"), "displayName": name});
    let logs = [
        (
            "demo-call-1",
            json!({"callDirection": "incoming", "callState": "missed", "startTime": at(25),
            "originatorParticipant": person("alice", "Alice Martin"), "targetParticipant": me}),
        ),
        (
            "demo-call-2",
            json!({"callDirection": "outgoing", "callState": "accepted", "startTime": at(180),
            "connectTime": at(180), "endTime": at(168), "callType": "TwoParty",
            "originatorParticipant": me, "targetParticipant": person("bob", "Bruno Lefèvre")}),
        ),
        (
            "demo-call-3",
            json!({"callDirection": "incoming", "callState": "accepted", "startTime": at(60 * 26),
            "connectTime": at(60 * 26), "endTime": at(60 * 26 - 3), "callType": "TwoParty",
            "originatorParticipant": person("claire", "Claire Dubois"), "targetParticipant": me}),
        ),
        (
            "demo-call-4",
            json!({"callDirection": "outgoing", "callState": "accepted", "startTime": at(60 * 50),
            "connectTime": at(60 * 50), "endTime": at(60 * 50 - 42), "callType": "MultiParty",
            "originatorParticipant": me, "participants": [me, person("alice", "Alice Martin"),
            person("david", "David Moreau")]}),
        ),
        (
            "demo-call-5",
            json!({"callDirection": "incoming", "callState": "declined", "startTime": at(60 * 75),
            "originatorParticipant": {"id": "4:+33612345678"}, "targetParticipant": me}),
        ),
    ];
    let deleted = deleted().lock().unwrap();
    let messages: Vec<Value> = logs
        .into_iter()
        .filter(|(id, _)| !deleted.contains(*id))
        .map(|(id, mut log)| {
            log["callId"] = json!(id);
            json!({"id": id, "messageType": "Text", "originalArrivalTime": log["startTime"],
                "properties": {"call-log": log.to_string()}})
        })
        .collect();
    Json(json!({"messages": messages, "_metadata": {}}))
}

async fn voicemails() -> Json<Value> {
    let deleted = deleted().lock().unwrap();
    let list: Vec<Value> = [
        json!({"Id": "demo-voicemail-1", "ReceivedDateTime": at(24), "IsRead": false,
            "From": {"EmailAddress": {"Name": "Alice Martin", "Address": "alice@example.test"}},
            "SingleValueExtendedProperties": [
                {"PropertyId": "Integer {00020328-0000-0000-c000-000000000046} Id 0x6801", "Value": "2"},
                {"PropertyId": "String {00020386-0000-0000-C000-000000000046} Name X-VoiceMessageTranscription",
                 "Value": "Salut, c’est Alice. Rappelle-moi quand tu peux pour la revue de demain."}]}),
        json!({"Id": "demo-voicemail-2", "ReceivedDateTime": at(60 * 75), "IsRead": true,
            "InternetMessageHeaders": [{"Name": "X-CallingTelephoneNumber", "Value": "+33612345678"}],
            "SingleValueExtendedProperties": [
                {"PropertyId": "Integer {00020328-0000-0000-c000-000000000046} Id 0x6801", "Value": "2"}]}),
    ]
    .into_iter()
    .filter(|v| !deleted.contains(v["Id"].as_str().unwrap_or_default()))
    .collect();
    Json(json!({ "value": list }))
}

/// Two seconds of a soft 440 Hz tone, as 8 kHz 8-bit mono WAV.
fn tone() -> Vec<u8> {
    let rate = 8000u32;
    let samples: Vec<u8> = (0..rate * 2)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let fade = (1.0 - t / 2.0).max(0.0);
            (128.0 + 40.0 * fade * (t * 440.0 * std::f32::consts::TAU).sin()) as u8
        })
        .collect();
    let mut wav = Vec::with_capacity(44 + samples.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

async fn attachments(Path(_id): Path<String>) -> Json<Value> {
    let bytes = base64::engine::general_purpose::STANDARD.encode(tone());
    Json(
        json!({"value": [{"Name": "message.wav", "ContentType": "audio/wav", "ContentBytes": bytes}]}),
    )
}

async fn ok(Path(_id): Path<String>) -> Json<Value> {
    Json(json!({}))
}

async fn delete_voicemail(Path(id): Path<String>) -> (StatusCode, Json<Value>) {
    deleted().lock().unwrap().insert(id);
    (StatusCode::CREATED, Json(json!({})))
}

/// The demo push channel: an in-process Trouter and registrar on its own runtime.
pub fn trouter() -> &'static teams_realtime::fake::FakeTrouter {
    static FAKE: OnceLock<teams_realtime::fake::FakeTrouter> = OnceLock::new();
    FAKE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("demo trouter runtime");
            rt.block_on(async move {
                tx.send(teams_realtime::fake::FakeTrouter::start().await)
                    .expect("demo trouter started");
                std::future::pending::<()>().await;
            });
        });
        rx.recv().expect("demo trouter started")
    })
}

/// The demo conversation service, delivering callbacks through the demo Trouter.
pub fn conversation() -> &'static teams_calling::fake::FakeConversation {
    static FAKE: OnceLock<teams_calling::fake::FakeConversation> = OnceLock::new();
    FAKE.get_or_init(|| {
        let trouter = trouter().clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("demo conversation runtime");
            rt.block_on(async move {
                tx.send(teams_calling::fake::FakeConversation::start(trouter, false).await)
                    .expect("demo conversation started");
                std::future::pending::<()>().await;
            });
        });
        rx.recv().expect("demo conversation started")
    })
}
