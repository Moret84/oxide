//! One Trouter v4a connection at a time (allocate, handshake, websocket, register, serve), reconnecting with backoff.
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Map, Value, json};
use teams_auth::{Authenticator, Service};
use teams_i18n::{t, tf};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

use crate::sio::{self, Kind, Packet};
use crate::transport::{Link, Polling};
use crate::{PushedRequest, RealtimeConfig, RealtimeEvent, Sink};

/// Browser-shaped user agent, identifying this client.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) TeamsRust/0.1";
/// Version of the web Trouter library whose behaviour this mirrors.
const LIB_VERSION: &str = "2026.05.01.1";

enum Failure {
    /// Worth retrying after a backoff.
    Retry(String),
    Fatal(String),
}

enum Exit {
    Stopped,
    /// Reconnect; `immediate` skips the backoff (server-requested reconnection).
    Reconnect {
        reason: String,
        immediate: bool,
    },
}

/// What survives between connections.
#[derive(Default)]
struct Cache {
    connectparams: Option<Map<String, Value>>,
    reconnect_url: Option<Url>,
    ccid: String,
    last_dup: Option<Instant>,
}

struct Allocation {
    socketio: Url,
    surl: String,
    ttl: u64,
    connectparams: Map<String, Value>,
}

struct Connection<'a, A> {
    auth: &'a A,
    config: &'a RealtimeConfig,
    http: reqwest::Client,
    client_id: u128,
    attempt: u32,
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// `1000 × 2^n × (1 ± 0.2)`, capped.
fn backoff(failures: u32, max: Duration) -> Duration {
    let base = 1000f64 * 2f64.powi(failures.min(16) as i32);
    let random = uuid::Uuid::new_v4().as_u64_pair().0 as f64 / u64::MAX as f64;
    let jitter = 0.8 + 0.4 * random;
    Duration::from_millis((base * jitter) as u64).min(max)
}

/// Uptime after which a drop stops counting as a failure, so a flapping link keeps backing off.
const STABLE_UPTIME: Duration = Duration::from_secs(30);

/// Consecutive failures since the last stable connection, and the delay before the next try.
#[derive(Default)]
struct Retries {
    failures: u32,
}

impl Retries {
    /// After a connection that ended (on its own or on server request) once up for `uptime`.
    fn reconnect(&mut self, immediate: bool, uptime: Duration, max: Duration) -> Duration {
        if uptime >= STABLE_UPTIME {
            self.failures = 0;
            return if immediate {
                Duration::ZERO
            } else {
                backoff(0, max)
            };
        }
        self.failures += 1;
        if immediate && self.failures == 1 {
            Duration::ZERO
        } else {
            backoff(self.failures, max)
        }
    }

    /// After a connection that could not be set up.
    fn failure(&mut self, max: Duration) -> Duration {
        self.failures += 1;
        backoff(self.failures, max)
    }
}

fn param(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Replays the allocation's opaque `connectparams` in a query.
fn append_params(
    query: &mut url::form_urlencoded::Serializer<url::UrlQuery>,
    params: &Map<String, Value>,
) {
    for (key, value) in params {
        if let Some(value) = param(value) {
            query.append_pair(key, &value);
        }
    }
}

fn tls() -> Option<tokio_tungstenite::Connector> {
    static CONFIG: OnceLock<Option<Arc<rustls::ClientConfig>>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            use rustls_platform_verifier::ConfigVerifierExt;
            rustls::ClientConfig::with_platform_verifier()
                .ok()
                .map(Arc::new)
        })
        .clone()
        .map(tokio_tungstenite::Connector::Rustls)
}

impl<A: Authenticator> Connection<'_, A> {
    fn tc(&self) -> String {
        json!({"cv": LIB_VERSION, "ua": self.config.user_agent, "hr": "", "v": self.config.client_version})
            .to_string()
    }

    fn con_num(&self) -> String {
        format!("{}_{}", self.client_id, self.attempt)
    }

    async fn headers(&self, service: Service) -> Result<HeaderMap, Failure> {
        let mut headers = HeaderMap::new();
        self.auth
            .authorize(service, &mut headers)
            .await
            .map_err(|e| Failure::Retry(e.to_string()))?;
        headers.insert("x-ms-test-user", HeaderValue::from_static("False"));
        // Like the web client in a browser: Trouter may pick the transports it offers from these.
        headers.insert(
            "origin",
            HeaderValue::from_static("https://teams.microsoft.com"),
        );
        headers.insert("user-agent", HeaderValue::from_static(USER_AGENT));
        Ok(headers)
    }

    /// Sends with credentials, retrying once after a 401 challenge.
    async fn send(
        &self,
        service: Service,
        method: Method,
        url: Url,
        body: Option<&Value>,
        content_type: &'static str,
    ) -> Result<reqwest::Response, Failure> {
        let name = format!("{service:?}");
        for attempt in 0..2 {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .headers(self.headers(service).await?)
                .timeout(self.config.timing.fetch_timeout)
                .header("content-type", content_type);
            if let Some(body) = body {
                request = request
                    .header("accept", "application/json, text/javascript")
                    .body(body.to_string());
            }
            let response = request.send().await.map_err(|e| {
                Failure::Retry(tf!("realtime.trouter.couldn_t_reach", service = name, without_url = e.without_url()))
            })?;
            if response
                .headers()
                .get("x-trouter-skypetoken-deprecated")
                .is_some_and(|v| v == "1")
            {
                return Err(Failure::Fatal(
                    t("realtime.trouter.token_refused_by_trouter_skypetoken_deprecated")
                    .into(),
                ));
            }
            if response.status() == 401
                && attempt == 0
                && self.auth.on_challenge(service, response.headers()).await
            {
                continue;
            }
            if !response.status().is_success() {
                return Err(Failure::Retry(tf!("realtime.trouter.http", service = name, status = response.status().as_u16())));
            }
            return Ok(response);
        }
        Err(Failure::Retry(tf!("realtime.trouter.http_401", service = name)))
    }

    async fn allocate(&self, cache: &Cache) -> Result<Allocation, Failure> {
        let mut url = match &cache.reconnect_url {
            Some(url) => url.clone(),
            None => self
                .auth
                .base_url(Service::Trouter)
                .map_err(|e| Failure::Fatal(e.to_string()))?,
        };
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("cor_id", &self.config.session_id)
                .append_pair("con_num", &self.con_num())
                .append_pair("epid", &self.config.endpoint_id)
                .append_pair("tc", &self.tc());
            if let Some(params) = &cache.connectparams {
                append_params(&mut query, params);
                query.append_pair("v", "v4");
            }
        }
        let response = self
            .send(Service::Trouter, Method::POST, url, None, "text/plain")
            .await?;
        let value: Value = response.json().await.map_err(|_| {
            Failure::Retry(
                t("realtime.trouter.unreadable_trouter_allocation")
                .into(),
            )
        })?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let socketio = text("socketio")
            .and_then(|s| Url::parse(&s).ok())
            .filter(|u| matches!(u.scheme(), "https" | "http"))
            .ok_or_else(|| {
                Failure::Retry(
                    t("realtime.trouter.trouter_allocation_has_no_socket_io")
                    .into(),
                )
            })?;
        let surl = text("surl").ok_or_else(|| {
            Failure::Retry(
                t("realtime.trouter.trouter_allocation_has_no_surl")
                .into(),
            )
        })?;
        Ok(Allocation {
            socketio,
            surl,
            ttl: value.get("ttl").and_then(Value::as_u64).unwrap_or(3600),
            connectparams: value
                .get("connectparams")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn socket_query(&self, url: &mut Url, allocation: &Allocation, ccid: &str) {
        let mut query = url.query_pairs_mut();
        append_params(&mut query, &allocation.connectparams);
        let timeout = self.config.timing.ping_interval.as_secs().to_string();
        query
            .append_pair("v", "v4")
            .append_pair("tc", &self.tc())
            .append_pair("timeout", &timeout)
            .append_pair("auth", "true")
            .append_pair("epid", &self.config.endpoint_id)
            .append_pair("ccid", ccid)
            .append_pair("cor_id", &self.config.session_id)
            .append_pair("con_num", &self.con_num());
    }

    /// socket.io 0.9 handshake: `sid:heartbeat:close:transports`.
    async fn handshake(&self, allocation: &Allocation, ccid: &str) -> Result<Handshake, Failure> {
        let mut url = allocation.socketio.join("socket.io/1/").map_err(|_| {
            Failure::Retry(t("realtime.trouter.invalid_socket_io_url").into())
        })?;
        self.socket_query(&mut url, allocation, ccid);
        url.query_pairs_mut()
            .append_pair("t", &now_ms().to_string());
        let body = self
            .send(Service::Trouter, Method::GET, url, None, "text/plain")
            .await?
            .text()
            .await
            .map_err(|_| {
                Failure::Retry(
                    t("realtime.trouter.unreadable_socket_io_handshake")
                    .into(),
                )
            })?;
        parse_handshake(&body).map_err(|reason| {
            Failure::Retry(tf!("realtime.trouter.socket_io_handshake_refused", reason = reason))
        })
    }

    /// Websocket when offered and reachable, else `xhr-polling`.
    async fn open(
        &self,
        allocation: &Allocation,
        ccid: &str,
        handshake: &Handshake,
    ) -> Result<Link, Failure> {
        let sid = &handshake.sid;
        if handshake.websocket {
            match self.websocket(allocation, ccid, sid).await {
                Ok(socket) => return Ok(Link::WebSocket(Box::new(socket))),
                Err(error) if !handshake.polling => return Err(error),
                Err(_) => {}
            }
        }
        let mut url = allocation
            .socketio
            .join(&format!("socket.io/1/xhr-polling/{sid}"))
            .map_err(|_| {
                Failure::Retry(t("realtime.trouter.invalid_socket_io_url").into())
            })?;
        self.socket_query(&mut url, allocation, ccid);
        let headers = self.headers(Service::Trouter).await?;
        Ok(Link::Polling(Polling::start(
            self.http.clone(),
            self.config.timing.fetch_timeout,
            url,
            headers,
        )))
    }

    async fn websocket(
        &self,
        allocation: &Allocation,
        ccid: &str,
        sid: &str,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, Failure> {
        let mut url = allocation.socketio.clone();
        let _ = url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" });
        let mut url = url
            .join(&format!("socket.io/1/websocket/{sid}"))
            .map_err(|_| {
                Failure::Retry(t("realtime.trouter.invalid_websocket_url").into())
            })?;
        self.socket_query(&mut url, allocation, ccid);
        let mut request = url.as_str().into_client_request().map_err(|_| {
            Failure::Retry(t("realtime.trouter.invalid_websocket_url").into())
        })?;
        request
            .headers_mut()
            .extend(self.headers(Service::Trouter).await?);
        let connector = if url.scheme() == "wss" { tls() } else { None };
        let (socket, _) = tokio::time::timeout(
            self.config.timing.connect_timeout,
            tokio_tungstenite::connect_async_tls_with_config(request, None, true, connector),
        )
        .await
        .map_err(|_| {
            Failure::Retry(
                t("realtime.trouter.websocket_connection_timed_out")
                .into(),
            )
        })?
        .map_err(|e| Failure::Retry(tf!("realtime.trouter.trouter_websocket", error = e)))?;
        Ok(socket)
    }

    fn registrar_url(&self) -> Result<Url, Failure> {
        let url = self
            .auth
            .base_url(Service::Registrar)
            .map_err(|e| Failure::Fatal(e.to_string()))?;
        Url::parse(url.as_str().trim_end_matches('/')).map_err(|_| {
            Failure::Fatal(t("realtime.trouter.invalid_registrar_url").into())
        })
    }

    async fn register(&self, surl: &str, ttl: u64) -> Result<u64, Failure> {
        let ttl = ttl.min(self.config.timing.registration_ttl).max(60);
        let body = json!({
            "clientDescription": {
                "appId": self.config.app_id,
                "aesKey": "",
                "languageId": "en-US",
                "platform": self.config.platform,
                "templateKey": self.config.template_key,
                "platformUIVersion": self.config.client_version,
            },
            "registrationId": self.config.endpoint_id,
            "nodeId": "",
            "transports": {"TROUTER": [{"context": "", "path": surl, "ttl": ttl}]},
        });
        self.send(
            Service::Registrar,
            Method::POST,
            self.registrar_url()?,
            Some(&body),
            "application/json",
        )
        .await?;
        Ok(ttl)
    }

    async fn unregister(&self) {
        if let Ok(mut url) = self.registrar_url() {
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.push(&self.config.endpoint_id);
            }
            let _ = self
                .send(
                    Service::Registrar,
                    Method::DELETE,
                    url,
                    None,
                    "application/json",
                )
                .await;
        }
    }
}

/// Parses `sid:heartbeat:close[:transports]`; like the web client, no list means "use our own"; errors only describe the shape (the body may hold secrets).
fn parse_handshake(body: &str) -> Result<Handshake, String> {
    let body = body.trim();
    let fields: Vec<&str> = body.split(':').collect();
    let sid = fields[0];
    let valid_sid = !sid.is_empty()
        && sid.len() <= 256
        && sid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.~+/=".contains(c));
    if !valid_sid || fields.len() < 3 {
        let start: String = body.chars().take(1).collect();
        return Err(tf!("realtime.trouter.unexpected_response_bytes_fields_starts_with", body = body.len(), fields = fields.len(), start = format!("{:?}", start)));
    }
    let transports = fields.get(3).copied().unwrap_or_default();
    let offers =
        |name: &str| transports.is_empty() || transports.split(',').any(|t| t.trim() == name);
    let handshake = Handshake {
        sid: sid.to_owned(),
        websocket: offers("websocket"),
        polling: offers("xhr-polling"),
    };
    if !handshake.websocket && !handshake.polling {
        return Err(tf!("realtime.trouter.offered_transports", transports = transports));
    }
    Ok(handshake)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Handshake {
    sid: String,
    websocket: bool,
    polling: bool,
}

/// Answer to a pushed request.
fn response(request: &Value, status: u16) -> Packet {
    let header = |name: &str| {
        request
            .get("headers")
            .and_then(Value::as_object)
            .and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_owned)
    };
    let mut headers = Map::new();
    if let Some(cv) = header("MS-CV") {
        headers.insert("MS-CV".into(), json!(format!("{cv}.0")));
    }
    for name in ["trouter-request", "trouter-is-broadcast"] {
        if let Some(value) = header(name) {
            headers.insert(name.into(), json!(value));
        }
    }
    headers.insert("trouter-client".into(), json!("{\"cd\":0}"));
    if status == 404 {
        headers.insert("Trouter-Responder".into(), json!("ClientLib"));
    }
    Packet::new(
        Kind::Message,
        json!({"id": request.get("id"), "status": status, "headers": headers, "body": ""})
            .to_string(),
    )
}

fn pushed(request: &Value, surl: &str) -> Option<PushedRequest> {
    let url = request.get("url")?.as_str()?.to_owned();
    let base = Url::parse(surl).ok()?;
    // Live pushes also use relative targets: resolve them against the session directory, not its last segment.
    let target = Url::parse(&url)
        .or_else(|_| {
            let mut directory = base.clone();
            directory.set_path(&format!("{}/", base.path().trim_end_matches('/')));
            directory.join(&url)
        })
        .ok()?;
    let prefix = base.path().trim_end_matches('/');
    let path = target.path().strip_prefix(prefix).unwrap_or(target.path());
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let headers = request
        .get("headers")
        .and_then(Value::as_object)
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| Some((k.to_ascii_lowercase(), param(v)?)))
                .collect()
        })
        .unwrap_or_default();
    let body = match request.get("body") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Some(PushedRequest {
        method: request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("POST")
            .to_owned(),
        url,
        path,
        headers,
        body,
    })
}

/// Runs the push channel until `stop` turns true, reporting to `sink`.
pub async fn run<A: Authenticator>(
    auth: Arc<A>,
    config: RealtimeConfig,
    sink: Sink,
    mut stop: watch::Receiver<bool>,
) {
    let http = teams_auth::net::client();
    let mut connection = Connection {
        auth: auth.as_ref(),
        config: &config,
        http,
        client_id: now_ms(),
        attempt: 0,
    };
    let mut cache = Cache::default();
    let mut retries = Retries::default();
    loop {
        if *stop.borrow() {
            return;
        }
        connection.attempt += 1;
        let started = tokio::time::Instant::now();
        let (reason, delay) = match session(&connection, &mut cache, &sink, &mut stop).await {
            Ok(Exit::Stopped) => return,
            Ok(Exit::Reconnect { reason, immediate }) => {
                let delay =
                    retries.reconnect(immediate, started.elapsed(), config.timing.max_backoff);
                (reason, delay)
            }
            Err(Failure::Fatal(reason)) => {
                sink(RealtimeEvent::Fatal(reason));
                return;
            }
            Err(Failure::Retry(reason)) => {
                let delay = retries.failure(config.timing.max_backoff);
                if retries.failures >= 3 {
                    cache.connectparams = None;
                    cache.reconnect_url = None;
                }
                (reason, delay)
            }
        };
        sink(RealtimeEvent::Disconnected {
            reason,
            retry_in: delay,
        });
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = stop.changed() => return,
        }
    }
}

async fn session<A: Authenticator>(
    connection: &Connection<'_, A>,
    cache: &mut Cache,
    sink: &Sink,
    stop: &mut watch::Receiver<bool>,
) -> Result<Exit, Failure> {
    let timing = &connection.config.timing;
    let allocation = connection.allocate(cache).await?;
    cache.connectparams = Some(allocation.connectparams.clone());
    let ccid = cache.ccid.clone();
    let handshake = connection.handshake(&allocation, &ccid).await?;
    if let Ok(url) = allocation.socketio.join("v4/a") {
        cache.reconnect_url = Some(url);
    }
    let mut link = connection.open(&allocation, &ccid, &handshake).await?;

    let surl = allocation.surl.trim_end_matches('/').to_owned();
    let mut connected = false;
    let mut registered: Option<u64> = None;
    let mut next_id = 0u64;
    let ping_timing = || {
        if connection.config.in_call.get() {
            let fast = crate::Timing::in_call();
            (
                fast.ping_interval.min(timing.ping_interval),
                fast.pong_timeout.min(timing.pong_timeout),
            )
        } else {
            (timing.ping_interval, timing.pong_timeout)
        }
    };
    let mut last_ping = tokio::time::Instant::now();
    let mut pong: Option<(u64, tokio::time::Instant)> = None;
    let mut register_at: Option<tokio::time::Instant> = None;
    let connect_deadline = tokio::time::Instant::now() + timing.connect_timeout;
    let far = Duration::from_secs(86400 * 365);

    loop {
        let ping_at = last_ping + ping_timing().0;
        let pong_at = pong
            .map(|(_, at)| at)
            .unwrap_or_else(|| tokio::time::Instant::now() + far);
        let renew_at = register_at.unwrap_or_else(|| tokio::time::Instant::now() + far);
        let connect_at = if connected {
            tokio::time::Instant::now() + far
        } else {
            connect_deadline
        };
        tokio::select! {
            _ = stop.changed() => {
                if registered.is_some() {
                    connection.unregister().await;
                }
                link.close().await;
                return Ok(Exit::Stopped);
            }
            _ = tokio::time::sleep_until(connect_at) => {
                return Err(Failure::Retry(t("realtime.trouter.trouter_didn_t_confirm_the_connection").into()));
            }
            _ = tokio::time::sleep_until(pong_at) => {
                return Ok(Exit::Reconnect { reason: t("realtime.trouter.trouter_ping_got_no_response").into(), immediate: false });
            }
            // Re-read the ping interval: a call may have started since this sleep began.
            _ = connection.config.in_call.changed(), if connected => {}
            // Application pings are a websocket feature; polling relies on heartbeats.
            _ = tokio::time::sleep_until(ping_at), if connected && link.is_websocket() => {
                next_id += 1;
                let packet = Packet::event(Some(next_id), "ping", &[]);
                link.send(packet.encode()).await
                    .map_err(|e| Failure::Retry(tf!("realtime.trouter.sending_ping", error = e)))?;
                pong = Some((next_id, tokio::time::Instant::now() + ping_timing().1));
                last_ping = tokio::time::Instant::now();
            }
            _ = tokio::time::sleep_until(renew_at), if connected => {
                match connection.register(&allocation.surl, allocation.ttl).await {
                    Ok(ttl) => {
                        registered = Some(ttl);
                        register_at = Some(tokio::time::Instant::now() + Duration::from_secs(ttl.saturating_sub(30).max(30)));
                        sink(RealtimeEvent::Registered { ttl });
                    }
                    Err(Failure::Fatal(reason)) => return Err(Failure::Fatal(reason)),
                    Err(Failure::Retry(reason)) => {
                        register_at = Some(tokio::time::Instant::now() + timing.registration_retry);
                        sink(RealtimeEvent::RegistrationFailed { reason });
                    }
                }
            }
            frame = link.recv() => {
                let text = match frame {
                    None => return Ok(Exit::Reconnect { reason: t("realtime.trouter.trouter_closed_the_connection").into(), immediate: false }),
                    Some(Err(e)) => return Err(Failure::Retry(tf!("realtime.trouter.trouter_connection", error = e))),
                    Some(Ok(text)) => text,
                };
                for packet in sio::decode_frame(&text) {
                    let mut replies: Vec<Packet> = Vec::new();
                    match packet.kind {
                        Kind::Connect if !connected => {
                            connected = true;
                            sink(RealtimeEvent::Connected { surl: surl.clone() });
                            // Over polling, the web client sends activity in the query instead.
                            if link.is_websocket() {
                                next_id += 1;
                                replies.push(Packet::event(Some(next_id), "user.activity",
                                    &[json!({"state": "active", "cv": LIB_VERSION})]));
                            }
                            register_at = Some(tokio::time::Instant::now());
                        }
                        Kind::Heartbeat => replies.push(Packet::new(Kind::Heartbeat, "")),
                        Kind::Ack if pong.is_some_and(|(id, _)| packet.acked() == Some(id)) => {
                            pong = None;
                        }
                        Kind::Disconnect => {
                            // Trouter puts the reason where socket.io 0.9 puts the endpoint.
                            let reason = if packet.data.is_empty() {
                                packet.endpoint.clone()
                            } else {
                                format!("{}:{}", packet.endpoint, packet.data)
                            };
                            return Err(disconnect(&reason, cache));
                        }
                        Kind::Message | Kind::Json => {
                            if let Some(id) = packet.id {
                                replies.push(Packet::new(Kind::Ack, id.to_string()));
                            }
                            let Ok(request) = serde_json::from_str::<Value>(&packet.data) else { continue };
                            if connection.config.diagnostics {
                                sink(RealtimeEvent::ProtocolDiagnostic(format!("packet {:?} keys={:?} absolute-url={}",
                                    packet.kind,
                                    request.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                                    request.get("url").and_then(Value::as_str).is_some_and(|url| Url::parse(url).is_ok()))));
                            }
                            let status = match pushed(&request, &allocation.surl) {
                                Some(push) if connection.config.paths.iter().any(|p| push.path.starts_with(p.as_str())) => {
                                    sink(RealtimeEvent::Request(push));
                                    200
                                }
                                _ => 404,
                            };
                            replies.push(response(&request, status));
                        }
                        Kind::Event => {
                            let Some((name, args)) = packet.event_parts() else { continue };
                            match name.as_str() {
                                "trouter.connected" => {
                                    if let Some(ccid) = args.first().and_then(|a| a.get("ccid")).and_then(Value::as_str) {
                                        cache.ccid = ccid.to_owned();
                                    }
                                }
                                "trouter.reconnect" => {
                                    let target = args.first();
                                    if target.and_then(|t| t.get("target")).and_then(Value::as_str) == Some("host")
                                        && let Some(url) = target.and_then(|t| t.get("url")).and_then(Value::as_str).and_then(|u| Url::parse(u).ok())
                                    {
                                        cache.reconnect_url = Some(url);
                                    }
                                    return Ok(Exit::Reconnect { reason: t("realtime.trouter.trouter_requested_a_reconnection").into(), immediate: true });
                                }
                                "trouter.message_loss" => {
                                    let tags = args.first()
                                        .and_then(|a| a.get("droppedIndicators"))
                                        .and_then(Value::as_array)
                                        .map(|list| list.iter().filter_map(|d| d.get("tag")?.as_str().map(str::to_owned)).collect())
                                        .unwrap_or_default();
                                    sink(RealtimeEvent::MessageLoss { tags });
                                    next_id += 1;
                                    replies.push(Packet::event(Some(next_id), "trouter.processed_message_loss", &args));
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    for reply in replies {
                        link.send(reply.encode()).await
                            .map_err(|e| Failure::Retry(tf!("realtime.trouter.trouter_send", error = e)))?;
                    }
                }
            }
        }
    }
}

/// `0::<reason>`: `dup` twice within 10 s clears the cache; a deprecated token is final.
fn disconnect(reason: &str, cache: &mut Cache) -> Failure {
    let parsed: Option<Value> = serde_json::from_str(reason).ok();
    let reason = parsed
        .as_ref()
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or(reason);
    if reason == "skypetoken-deprecated" {
        return Failure::Fatal(
            t("realtime.trouter.token_refused_by_trouter_skypetoken_deprecated")
            .into(),
        );
    }
    if reason == "dup" {
        if cache
            .last_dup
            .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
        {
            *cache = Cache::default();
        } else {
            cache.last_dup = Some(Instant::now());
        }
    }
    Failure::Retry(if reason.is_empty() {
        t("realtime.trouter.trouter_dropped_the_connection")
        .into()
    } else {
        tf!("realtime.trouter.trouter_dropped_the_connection_2", reason = reason)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushed_requests_are_relative_to_the_session_url_and_answered_with_its_headers() {
        let request = json!({"id": 4, "method": "POST",
            "url": "https://h.example:3443/abc/callAgent/s/u/call/progress/",
            "headers": {"MS-CV": "cv1", "trouter-request": "r", "Content-Type": "application/json"},
            "body": "{\"x\":1}"});
        let push = pushed(&request, "https://h.example:3443/abc/").unwrap();
        assert_eq!(push.path, "/callAgent/s/u/call/progress/");
        assert_eq!(push.headers["content-type"], "application/json");
        assert_eq!(push.body, "{\"x\":1}");
        for url in [
            "/abc/callAgent/s/u/call/progress/",
            "/callAgent/s/u/call/progress/",
            "callAgent/s/u/call/progress/",
        ] {
            let mut relative = request.clone();
            relative["url"] = json!(url);
            let relative = pushed(&relative, "https://h.example:3443/abc").unwrap();
            assert_eq!(relative.path, push.path, "relative target {url}");
            assert_eq!(relative.body, push.body);
        }
        let answer: Value = serde_json::from_str(&response(&request, 200).data).unwrap();
        assert_eq!(answer["id"], 4);
        assert_eq!(answer["status"], 200);
        assert_eq!(answer["headers"]["MS-CV"], "cv1.0");
        assert_eq!(answer["headers"]["trouter-request"], "r");
        let missing: Value = serde_json::from_str(&response(&request, 404).data).unwrap();
        assert_eq!(missing["headers"]["Trouter-Responder"], "ClientLib");
    }

    #[test]
    fn the_platform_tls_configuration_builds() {
        assert!(tls().is_some());
    }

    #[test]
    fn handshakes_choose_among_the_offered_transports_like_the_web_client() {
        let offered = |body: &str| parse_handshake(body).map(|h| (h.sid, h.websocket, h.polling));
        assert_eq!(
            offered("abc123:70:70:websocket,xhr-polling"),
            Ok(("abc123".into(), true, true))
        );
        // No list: the client uses its own transports.
        assert_eq!(offered("abc123:70:70:"), Ok(("abc123".into(), true, true)));
        assert_eq!(
            offered("abc-1_2.x:40:40\n"),
            Ok(("abc-1_2.x".into(), true, true))
        );
        assert_eq!(
            offered("abc:70:70:xhr-polling"),
            Ok(("abc".into(), false, true))
        );
        assert!(
            offered("abc:70:70:flashsocket")
                .unwrap_err()
                .contains("flashsocket")
        );
        let html = parse_handshake("<html>secret-token</html>").unwrap_err();
        assert!(html.contains("commence par \"<\""), "{html}");
        assert!(!html.contains("secret"));
        assert!(parse_handshake("").is_err());
        assert!(parse_handshake("abc").is_err());
    }

    #[test]
    fn backoff_grows_with_jitter_and_is_capped() {
        let max = Duration::from_secs(30);
        let first = backoff(0, max);
        assert!(first >= Duration::from_millis(800) && first <= Duration::from_millis(1200));
        assert_eq!(backoff(20, max), max);
    }

    fn around(delay: Duration, nominal_ms: u64) -> bool {
        let ms = delay.as_millis() as u64;
        ms >= nominal_ms * 8 / 10 && ms <= nominal_ms * 12 / 10
    }

    #[test]
    fn drops_soon_after_connecting_keep_backing_off() {
        let max = Duration::from_secs(30);
        let short = Duration::from_secs(2);
        let mut retries = Retries::default();
        // A server-requested reconnection is immediate once, not in a loop.
        assert_eq!(retries.reconnect(true, short, max), Duration::ZERO);
        assert!(around(retries.reconnect(true, short, max), 4000));
        assert!(around(retries.reconnect(false, short, max), 8000));
        assert!(around(retries.failure(max), 16000));
        // 32 s with ±20 % jitter, capped at 30 s.
        let capped = retries.reconnect(false, short, max);
        assert!(capped >= Duration::from_millis(25_600) && capped <= max, "{capped:?}");
        assert_eq!(retries.failures, 5);
        // Only a connection that stayed up resets the count.
        assert!(around(retries.reconnect(false, STABLE_UPTIME, max), 1000));
        assert_eq!(retries.failures, 0);
        assert!(around(retries.failure(max), 2000));
        assert_eq!(
            retries.reconnect(true, Duration::from_secs(60), max),
            Duration::ZERO
        );
        assert_eq!(retries.failures, 0);
    }

    #[test]
    fn jitter_is_random_not_clock_driven() {
        let max = Duration::from_secs(30);
        let delays: std::collections::HashSet<_> = (0..20).map(|_| backoff(0, max)).collect();
        assert!(delays.len() > 1, "{delays:?}");
    }

    #[test]
    fn repeated_dup_disconnects_reset_the_connection_cache() {
        let mut cache = Cache {
            ccid: "c".into(),
            ..Default::default()
        };
        assert!(matches!(disconnect("dup", &mut cache), Failure::Retry(_)));
        assert_eq!(cache.ccid, "c");
        disconnect("dup", &mut cache);
        assert!(cache.ccid.is_empty());
        assert!(matches!(
            disconnect(r#"{"reason":"skypetoken-deprecated"}"#, &mut cache),
            Failure::Fatal(_)
        ));
    }
}
