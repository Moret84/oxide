use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use teams_auth::{Service, StaticAuthenticator};
use teams_realtime::fake::FakeTrouter;
use teams_realtime::{RealtimeConfig, RealtimeEvent, Timing};
use tokio::sync::{mpsc, watch};

struct Harness {
    fake: FakeTrouter,
    in_call: Arc<teams_realtime::CallFlag>,
    events: mpsc::UnboundedReceiver<RealtimeEvent>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

async fn start(timing: Timing) -> Harness {
    start_with(timing, false).await
}

async fn start_with(timing: Timing, polling_only: bool) -> Harness {
    let fake = FakeTrouter::start().await;
    fake.polling_only(polling_only);
    let auth = StaticAuthenticator::new()
        .with_service(Service::Trouter, fake.trouter_url())
        .with_service(Service::Registrar, fake.registrar_url())
        .with_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer ic3"),
        );
    let mut config = RealtimeConfig::calling("endpoint-1".into(), "session-1".into());
    config.timing = timing;
    let in_call = config.in_call.clone();
    let (tx, events) = mpsc::unbounded_channel();
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(teams_realtime::run(
        Arc::new(auth),
        config,
        Arc::new(move |event| {
            let _ = tx.send(event);
        }),
        stopped,
    ));
    Harness {
        fake,
        in_call,
        events,
        stop,
        task,
    }
}

fn fast() -> Timing {
    Timing {
        ping_interval: Duration::from_millis(150),
        pong_timeout: Duration::from_millis(150),
        max_backoff: Duration::from_millis(50),
        connect_timeout: Duration::from_secs(2),
        ..Timing::default()
    }
}

impl Harness {
    async fn next(&mut self, wanted: impl Fn(&RealtimeEvent) -> bool) -> RealtimeEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = self.events.recv().await.expect("run loop ended");
                if wanted(&event) {
                    return event;
                }
            }
        })
        .await
        .expect("event did not arrive")
    }

    async fn frame(&self, wanted: impl Fn(&str) -> bool) -> String {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(frame) = self.fake.seen().frames.into_iter().find(|f| wanted(f)) {
                    return frame;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("frame not sent")
    }
}

#[tokio::test]
async fn connects_registers_and_answers_pushed_requests() {
    let mut h = start(Timing::default()).await;
    let connected = h
        .next(|e| matches!(e, RealtimeEvent::Connected { .. }))
        .await;
    assert_eq!(
        connected,
        RealtimeEvent::Connected {
            surl: h.fake.surl().trim_end_matches('/').to_owned()
        }
    );
    assert_eq!(
        h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
            .await,
        RealtimeEvent::Registered { ttl: 3600 }
    );

    let seen = h.fake.seen();
    let allocation = &seen.allocations[0];
    for part in [
        "cor_id=session-1",
        "epid=endpoint-1",
        "con_num=",
        "tc=%7B%22cv%22",
    ] {
        assert!(allocation.contains(part), "{allocation} lacks {part}");
    }
    assert!(seen.credentials.iter().all(|c| c == "Bearer ic3"));
    let registration = &seen.registrations[0];
    assert_eq!(registration["registrationId"], "endpoint-1");
    assert_eq!(registration["clientDescription"]["appId"], "SkypeSpacesWeb");
    assert_eq!(
        registration["clientDescription"]["templateKey"],
        "SkypeSpacesWeb_2.6"
    );
    assert_eq!(
        registration["transports"]["TROUTER"][0]["path"],
        h.fake.surl()
    );
    assert_eq!(registration["transports"]["TROUTER"][0]["ttl"], 3600);
    h.frame(|f| f.contains("user.activity")).await;

    let id = h.fake.push(
        "POST",
        "/callAgent/s1/u1/call/progress/",
        json!({"MS-CV": "abc", "Content-Type": "application/json"}),
        "{\"callNotification\":{}}",
    );
    let RealtimeEvent::Request(request) = h.next(|e| matches!(e, RealtimeEvent::Request(_))).await
    else {
        unreachable!()
    };
    assert_eq!(request.path, "/callAgent/s1/u1/call/progress/");
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.body, "{\"callNotification\":{}}");
    let answer = h
        .frame(|f| f.starts_with("3:::") && f.contains("\"status\""))
        .await;
    let answer: Value = serde_json::from_str(&answer[4..]).unwrap();
    assert_eq!(answer["id"], id);
    assert_eq!(answer["status"], 200);
    assert_eq!(answer["headers"]["MS-CV"], "abc.0");

    h.stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), h.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h.fake.seen().unregistrations, ["endpoint-1"]);
}

#[tokio::test]
async fn reconnects_with_cached_parameters_after_a_disconnect_or_a_lost_ping() {
    let mut h = start(fast()).await;
    h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
        .await;
    // Heartbeats are echoed and pings acknowledged: the session stays up.
    h.fake.send("2::");
    h.frame(|f| f == "2::").await;
    h.frame(|f| f.contains("\"ping\"")).await;

    h.fake.send("0::dup");
    let RealtimeEvent::Disconnected { reason, .. } = h
        .next(|e| matches!(e, RealtimeEvent::Disconnected { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(reason.contains("dup"), "{reason}");
    h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
        .await;
    let seen = h.fake.seen();
    assert_eq!(seen.allocations.len(), 2);
    // The second allocation replays the opaque connect parameters.
    assert!(seen.allocations[1].contains("se=9999999999999"));
    assert!(seen.allocations[1].contains("v=v4"));
    assert_eq!(seen.registrations.len(), 2);

    h.fake.ack_pings(false);
    let RealtimeEvent::Disconnected { reason, .. } = h
        .next(|e| matches!(e, RealtimeEvent::Disconnected { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(reason.contains("Ping"), "{reason}");
    h.fake.ack_pings(true);
    h.next(|e| matches!(e, RealtimeEvent::Connected { .. }))
        .await;
    h.stop.send(true).unwrap();
}

#[tokio::test]
async fn server_requested_reconnection_and_message_loss() {
    let mut h = start(fast()).await;
    h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
        .await;
    h.fake.send(format!(
        "5:::{}",
        json!({"name": "trouter.message_loss", "args": [{"droppedIndicators": [{"tag": "messaging", "etag": "e"}]}]})
    ));
    assert_eq!(
        h.next(|e| matches!(e, RealtimeEvent::MessageLoss { .. }))
            .await,
        RealtimeEvent::MessageLoss {
            tags: vec!["messaging".into()]
        }
    );
    h.frame(|f| f.contains("trouter.processed_message_loss"))
        .await;

    h.fake.send(format!(
        "5:::{}",
        json!({"name": "trouter.reconnect", "args": [{"target": "self"}]})
    ));
    let RealtimeEvent::Disconnected { retry_in, .. } = h
        .next(|e| matches!(e, RealtimeEvent::Disconnected { .. }))
        .await
    else {
        unreachable!()
    };
    assert_eq!(retry_in, Duration::ZERO);
    h.next(|e| matches!(e, RealtimeEvent::Connected { .. }))
        .await;
    assert_eq!(h.fake.seen().sockets, 2);

    h.fake.send("0::{\"reason\":\"skypetoken-deprecated\"}");
    assert!(matches!(
        h.next(|e| matches!(e, RealtimeEvent::Fatal(_))).await,
        RealtimeEvent::Fatal(_)
    ));
    tokio::time::timeout(Duration::from_secs(5), h.task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn pings_every_few_seconds_while_a_call_is_active() {
    let mut h = start(Timing::default()).await;
    h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
        .await;
    // Default interval is 40 s: no ping yet.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!h.fake.seen().frames.iter().any(|f| f.contains("\"ping\"")));
    h.in_call.set(true);
    tokio::time::timeout(Duration::from_secs(7), h.frame(|f| f.contains("\"ping\"")))
        .await
        .expect("no ping within the in-call interval");
    h.stop.send(true).unwrap();
}

#[tokio::test]
async fn falls_back_to_xhr_polling_when_websocket_is_not_offered() {
    let mut h = start_with(Timing::default(), true).await;
    h.next(|e| matches!(e, RealtimeEvent::Connected { .. }))
        .await;
    h.next(|e| matches!(e, RealtimeEvent::Registered { .. }))
        .await;
    let seen = h.fake.seen();
    assert_eq!(seen.origins[0], "https://teams.microsoft.com");
    assert!(seen.polls >= 1);
    assert!(
        seen.frames.iter().all(|f| !f.contains("user.activity")),
        "no events over polling"
    );

    // Heartbeats are answered over POST, and pushed requests still arrive and get answered.
    h.fake.send("2::");
    h.frame(|f| f == "2::").await;
    let id = h
        .fake
        .push("POST", "/callAgent/s/c/call/progress/", json!({}), "{}");
    let RealtimeEvent::Request(request) = h.next(|e| matches!(e, RealtimeEvent::Request(_))).await
    else {
        unreachable!()
    };
    assert_eq!(request.path, "/callAgent/s/c/call/progress/");
    let answer = h
        .frame(|f| f.starts_with("3:::") && f.contains("\"status\""))
        .await;
    let answer: Value = serde_json::from_str(&answer[4..]).unwrap();
    assert_eq!(answer["id"], id);
    h.stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), h.task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        h.fake.seen().frames.iter().any(|f| f.starts_with("0::")),
        "disconnect sent on stop"
    );
}
