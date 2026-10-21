//! In-process Trouter and registrar speaking [`crate::run`]'s wire protocol, for tests and demo mode.
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use url::Url;

/// What the fake has seen, for assertions.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    /// Query strings of each allocation.
    pub allocations: Vec<String>,
    /// Authorization (or X-Skypetoken) header of each HTTP request.
    pub credentials: Vec<String>,
    pub registrations: Vec<Value>,
    pub unregistrations: Vec<String>,
    /// Text frames the client sent, in order.
    pub frames: Vec<String>,
    pub sockets: u32,
    /// Long polls answered (polling mode).
    pub polls: u32,
    /// `Origin` header of each handshake.
    pub origins: Vec<String>,
}

struct Inner {
    base: Url,
    seen: Mutex<Seen>,
    commands: broadcast::Sender<String>,
    ack_pings: Mutex<bool>,
    /// Handshake offers only `xhr-polling`, like the real service did for us.
    polling_only: Mutex<bool>,
    /// Frames waiting for the next long poll.
    queue: Mutex<std::collections::VecDeque<String>>,
    queued: tokio::sync::Notify,
    next_request: Mutex<u64>,
}

#[derive(Clone)]
pub struct FakeTrouter(Arc<Inner>);

impl FakeTrouter {
    /// Serves on a random local port from the current tokio runtime.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake trouter");
        let addr: SocketAddr = listener.local_addr().expect("fake trouter address");
        let (commands, _) = broadcast::channel(64);
        let fake = Self(Arc::new(Inner {
            base: Url::parse(&format!("http://{addr}/")).unwrap(),
            seen: Mutex::default(),
            commands,
            ack_pings: Mutex::new(true),
            polling_only: Mutex::new(false),
            queue: Mutex::default(),
            queued: tokio::sync::Notify::new(),
            next_request: Mutex::new(0),
        }));
        let app = Router::new()
            .route("/v4/a", post(allocate))
            .route("/socket.io/1/", get(handshake))
            .route("/socket.io/1/websocket/{sid}", get(upgrade))
            .route("/socket.io/1/xhr-polling/{sid}", get(poll).post(poll_send))
            .route("/registrations", post(register))
            .route("/registrations/{id}", delete(unregister))
            .with_state(fake.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        fake
    }

    pub fn trouter_url(&self) -> Url {
        self.0.base.join("v4/a").unwrap()
    }
    pub fn registrar_url(&self) -> Url {
        self.0.base.join("registrations").unwrap()
    }
    /// Public session URL handed out at allocation.
    pub fn surl(&self) -> String {
        format!("{}session/abc/", self.0.base)
    }
    pub fn seen(&self) -> Seen {
        self.0.seen.lock().unwrap().clone()
    }
    /// Whether pings are acknowledged; off makes the client reconnect.
    pub fn ack_pings(&self, ack: bool) {
        *self.0.ack_pings.lock().unwrap() = ack;
    }
    /// Offer only `xhr-polling` at the next handshakes.
    pub fn polling_only(&self, only: bool) {
        *self.0.polling_only.lock().unwrap() = only;
    }
    /// Sends a raw socket.io frame to connected clients.
    pub fn send(&self, frame: impl Into<String>) {
        let frame = frame.into();
        if *self.0.polling_only.lock().unwrap() {
            self.0.queue.lock().unwrap().push_back(frame);
            self.0.queued.notify_one();
        } else {
            let _ = self.0.commands.send(frame);
        }
    }
    /// Pushes an HTTP request under the session URL, as a service would through Trouter.
    pub fn push(&self, method: &str, path: &str, headers: Value, body: &str) -> u64 {
        let id = {
            let mut next = self.0.next_request.lock().unwrap();
            *next += 1;
            *next
        };
        // Real Trouter request targets are origin-relative, not necessarily URLs.
        let surl = Url::parse(&self.surl()).unwrap();
        let url = format!("{}{}", surl.path().trim_end_matches('/'), path);
        self.send(format!(
            "3:::{}",
            json!({"id": id, "method": method, "url": url, "headers": headers, "body": body})
        ));
        id
    }

    fn credential(&self, headers: &HeaderMap) {
        let value = headers
            .get("authorization")
            .or_else(|| headers.get("x-skypetoken"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        self.0.seen.lock().unwrap().credentials.push(value);
    }
}

async fn allocate(
    State(fake): State<FakeTrouter>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Json<Value> {
    fake.credential(&headers);
    fake.0
        .seen
        .lock()
        .unwrap()
        .allocations
        .push(query.unwrap_or_default());
    Json(json!({
        "socketio": fake.0.base.as_str(),
        "surl": fake.surl(),
        "ccid": "fake-ccid",
        "id": "fake-id",
        "ttl": 3600,
        "connectparams": {"se": "9999999999999", "sig": "opaque"}
    }))
}

async fn handshake(State(fake): State<FakeTrouter>, headers: HeaderMap) -> String {
    fake.credential(&headers);
    fake.0.seen.lock().unwrap().origins.push(
        headers
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    if *fake.0.polling_only.lock().unwrap() {
        "fake-sid:70:70:xhr-polling".into()
    } else {
        "fake-sid:70:70:websocket,xhr-polling".into()
    }
}

async fn register(
    State(fake): State<FakeTrouter>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> StatusCode {
    fake.credential(&headers);
    fake.0.seen.lock().unwrap().registrations.push(body);
    StatusCode::ACCEPTED
}

async fn unregister(State(fake): State<FakeTrouter>, Path(id): Path<String>) -> StatusCode {
    fake.0.seen.lock().unwrap().unregistrations.push(id);
    StatusCode::OK
}

async fn upgrade(
    State(fake): State<FakeTrouter>,
    Path(_sid): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| serve(fake, socket))
}

async fn serve(fake: FakeTrouter, mut socket: WebSocket) {
    fake.0.seen.lock().unwrap().sockets += 1;
    let mut commands = fake.0.commands.subscribe();
    if socket.send(Message::text("1::")).await.is_err() {
        return;
    }
    let _ = socket.send(Message::text(connected_event())).await;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Ok(frame) = command else { return };
                let closing = frame.starts_with("0::");
                if socket.send(Message::text(frame)).await.is_err() || closing {
                    return;
                }
            }
            frame = socket.recv() => {
                let Some(Ok(Message::Text(text))) = frame else { return };
                let text = text.to_string();
                fake.0.seen.lock().unwrap().frames.push(text.clone());
                // Ack pings, like Trouter: `5:<n>+::{"name":"ping"}` → `6:::<n>`.
                if let Some(packet) = crate::sio::Packet::decode(&text)
                    && packet.event_parts().is_some_and(|(name, _)| name == "ping")
                    && *fake.0.ack_pings.lock().unwrap()
                    && let Some(id) = packet.id
                {
                    let _ = socket.send(Message::text(format!("6:::{id}"))).await;
                }
            }
        }
    }
}

fn connected_event() -> String {
    let connected =
        json!({"name": "trouter.connected", "args": [{"ccid": "fake-ccid", "ttl": 3600}]});
    format!("5:::{connected}")
}

/// `\u{fffd}<len>\u{fffd}<packet>` for each packet, lengths in UTF-16 units.
fn framed(packets: &[String]) -> String {
    if packets.len() == 1 {
        return packets[0].clone();
    }
    packets
        .iter()
        .map(|p| format!("\u{fffd}{}\u{fffd}{p}", p.encode_utf16().count()))
        .collect()
}

/// Long poll: the first one opens the session, later ones return queued frames or a noop.
async fn poll(State(fake): State<FakeTrouter>, Path(_sid): Path<String>) -> String {
    let first = {
        let mut seen = fake.0.seen.lock().unwrap();
        seen.polls += 1;
        seen.polls == 1
    };
    if first {
        fake.0.seen.lock().unwrap().sockets += 1;
        return framed(&["1::".into(), connected_event()]);
    }
    let waiting = fake.0.queue.lock().unwrap().is_empty();
    if waiting {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            fake.0.queued.notified(),
        )
        .await;
    }
    let packets: Vec<String> = fake.0.queue.lock().unwrap().drain(..).collect();
    if packets.is_empty() {
        "8::".into()
    } else {
        framed(&packets)
    }
}

async fn poll_send(
    State(fake): State<FakeTrouter>,
    Path(_sid): Path<String>,
    body: String,
) -> StatusCode {
    for packet in crate::sio::decode_frame(&body) {
        let text = packet.encode();
        fake.0.seen.lock().unwrap().frames.push(text);
    }
    StatusCode::OK
}
