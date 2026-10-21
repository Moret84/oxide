//! Times the demo core over a long conversation: `TEAMS_CORE_TIMING=1 TEAMS_DEMO_HISTORY=600 cargo test -p teams-core --release demo_long_conversation -- --ignored --nocapture`.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Meter {
    updates: AtomicU64,
    bytes: AtomicU64,
    conversation_bytes: AtomicU64,
    last: Mutex<Option<Instant>>,
}
impl CoreListener for Meter {
    fn update(&self, update: Update) {
        let conversation =
            update.conversation.is_some() || update.feed.is_some() || update.typing.is_some();
        // What Swift receives: the update lowered into one buffer.
        let buffer = <Update as uniffi::Lower<crate::UniFfiTag>>::lower(update);
        let len = buffer.len() as u64;
        buffer.destroy();
        self.updates.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(len, Ordering::Relaxed);
        if conversation {
            self.conversation_bytes.fetch_add(len, Ordering::Relaxed);
        }
        *lock(&self.last) = Some(Instant::now());
    }
    fn effect(&self, _: Effect) {}
}
impl Meter {
    /// Waits until no update arrived for `quiet`.
    fn settle(&self, quiet: Duration) {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let last = *lock(&self.last);
            if last.is_some_and(|t| t.elapsed() >= quiet) {
                return;
            }
        }
    }
    fn phase(&self, name: &str, run: impl FnOnce()) {
        let (u, b, c) = (
            self.updates.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
            self.conversation_bytes.load(Ordering::Relaxed),
        );
        eprintln!("phase-start {name}");
        run();
        self.settle(Duration::from_millis(1200));
        eprintln!(
            "phase-end {name}: {} updates, {} bytes, {} bytes in updates carrying the conversation, its messages or typing",
            self.updates.load(Ordering::Relaxed) - u,
            self.bytes.load(Ordering::Relaxed) - b,
            self.conversation_bytes.load(Ordering::Relaxed) - c,
        );
    }
}

fn typing_push(conv: &str, generation: u64) -> Msg {
    let body = serde_json::json!({
        "resourceType": "NewMessage",
        "resourceLink": format!("https://mock/v1/users/ME/conversations/{conv}/messages/0"),
        "resource": {
            "id": "0",
            "messagetype": teams_chat::receipts::TYPING,
            "content": "",
            "from": "https://mock/v1/users/ME/contacts/8:orgid:bob",
            "imdisplayname": "Bruno Lefèvre",
            "conversationLink": format!("https://mock/v1/users/ME/conversations/{conv}"),
        }
    });
    Msg::Notifications(app::notifications::NotificationsMsg::Realtime(
        generation,
        teams_realtime::RealtimeEvent::Request(teams_realtime::PushedRequest {
            method: "POST".into(),
            url: String::new(),
            path: String::new(),
            headers: Default::default(),
            body: body.to_string(),
        }),
    ))
}

#[test]
#[ignore = "measurement: run by hand with --nocapture"]
fn demo_long_conversation() {
    let meter = Arc::new(Meter::default());
    let core = OxideCore::new(meter.clone(), true);
    meter.settle(Duration::from_millis(1500));
    let conv = "19:projet-rust@thread.v2";
    let generation = lock(&core.inner.state).app.measured().0;
    let queue = |msg: Msg| core.inner.queue(Event::Msg(msg));
    meter.phase("open", || {
        core.send(Action::Select {
            target: Target {
                team: None,
                id: conv.into(),
            },
        })
    });
    let count = lock(&core.inner.state).app.measured().1;
    eprintln!("open conversation: {count} messages");
    meter.phase("tick x3", || {
        for _ in 0..3 {
            queue(Msg::Tick);
            std::thread::sleep(Duration::from_millis(400));
        }
    });
    meter.phase("typing push x10", || {
        for _ in 0..10 {
            queue(typing_push(conv, generation));
            std::thread::sleep(Duration::from_millis(30));
        }
    });
    meter.phase("draft x20", || {
        let mut draft = String::new();
        for c in "Bonjour tout le monde".chars().take(20) {
            draft.push(c);
            core.send(Action::DraftChanged {
                value: draft.clone(),
            });
        }
    });
    meter.phase("send", || core.send(Action::Send));
}
