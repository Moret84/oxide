//! Meeting join without media, end to end over the wire against in-process fakes.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use teams_auth::{CallingContext, Service, StaticAuthenticator};
use teams_calling::MeetingLink;
use teams_calling::client::ConversationClient;
use teams_calling::driver::{self, Event};
use teams_calling::fake::FakeConversation;
use teams_calling::payload::{Callbacks, Local};
use teams_calling::session::{CallKind, CallSession, Input, Phase};
use teams_realtime::fake::FakeTrouter;
use teams_realtime::{RealtimeConfig, RealtimeEvent};
use tokio::sync::{mpsc, watch};

type Route = Arc<Mutex<Option<(String, mpsc::UnboundedSender<Input>)>>>;

const LINK: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_demo%40thread.v2/0?context=%7b%22Tid%22%3a%22tenant%22%2c%22Oid%22%3a%22organizer%22%7d";

#[derive(Debug, Clone)]
struct Snapshot {
    phase: Phase,
    roster: Vec<String>,
}

struct Meeting {
    conversation: FakeConversation,
    inputs: mpsc::UnboundedSender<Input>,
    snapshots: mpsc::UnboundedReceiver<Snapshot>,
    driver: tokio::task::JoinHandle<CallSession>,
    _stop: watch::Sender<bool>,
}

async fn join(lobby: bool) -> Meeting {
    let trouter = FakeTrouter::start().await;
    let conversation = FakeConversation::start(trouter.clone(), lobby).await;
    let auth = Arc::new(
        StaticAuthenticator::new()
            .with_service(Service::Trouter, trouter.trouter_url())
            .with_service(Service::Registrar, trouter.registrar_url())
            .with_service(Service::CallingConversation, conversation.url())
            .with_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_static("Bearer ic3"),
            )
            .with_calling_context(CallingContext {
                user_mri: "8:orgid:me".into(),
                display_name: "Moi".into(),
                tenant_id: Some("tenant".into()),
                region: Some("emea".into()),
                ..Default::default()
            }),
    );

    // Push channel: remember the session URL, route callbacks to the call.
    let (inputs, receiver) = mpsc::unbounded_channel::<Input>();
    let route: Route = Arc::default();
    let (surl_tx, mut surl_rx) = mpsc::unbounded_channel();
    let routing = route.clone();
    let (stop, stopped) = watch::channel(false);
    let config = RealtimeConfig::calling("endpoint-1".into(), "session-1".into());
    tokio::spawn(teams_realtime::run(
        auth.clone(),
        config,
        Arc::new(move |event| match event {
            RealtimeEvent::Registered { .. } => {
                let _ = surl_tx.send(());
            }
            RealtimeEvent::Request(request) => {
                if let Some((call, tx)) = routing.lock().unwrap().as_ref()
                    && let Some(input) =
                        driver::route(call, &request.path, &request.headers, &request.body)
                {
                    let _ = tx.send(input);
                }
            }
            _ => {}
        }),
        stopped,
    ));
    tokio::time::timeout(Duration::from_secs(5), surl_rx.recv())
        .await
        .expect("push channel not registered");

    let Some(MeetingLink::Coordinates(meeting)) = MeetingLink::parse(LINK) else {
        panic!("link")
    };
    let client = ConversationClient::new(auth);
    let session = CallSession::new(
        "call-1".into(),
        Local {
            id: "8:orgid:me".into(),
            display_name: "Moi".into(),
            endpoint_id: "endpoint-1".into(),
            participant_id: "participant-1".into(),
            language_id: "fr-fr".into(),
        },
        Callbacks::new(&trouter.surl(), "session-1", "call-1"),
        CallKind::Meeting(meeting),
        None,
        client.conversation_service().unwrap(),
    );
    *route.lock().unwrap() = Some(("call-1".into(), inputs.clone()));
    let (snapshot_tx, snapshots) = mpsc::unbounded_channel();
    let driver = tokio::spawn(driver::drive(
        client,
        session,
        receiver,
        inputs.clone(),
        move |event| {
            if let Event::Changed(session) = event {
                let _ = snapshot_tx.send(Snapshot {
                    phase: session.phase.clone(),
                    roster: session.roster.keys().cloned().collect(),
                });
            }
        },
    ));
    Meeting {
        conversation,
        inputs,
        snapshots,
        driver,
        _stop: stop,
    }
}

impl Meeting {
    async fn until(&mut self, wanted: impl Fn(&Snapshot) -> bool) -> Snapshot {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = self.snapshots.recv().await.expect("driver stopped");
                if wanted(&snapshot) {
                    return snapshot;
                }
            }
        })
        .await
        .expect("state never reached")
    }
}

#[tokio::test]
async fn joins_from_the_lobby_follows_the_roster_survives_a_redirect_and_leaves() {
    let mut meeting = join(true).await;
    meeting.until(|s| s.phase == Phase::Lobby).await;

    let seen = meeting.conversation.seen();
    let body = &seen.joins[0];
    assert_eq!(body["subscribe"], true);
    assert_eq!(body["groupChat"]["threadId"], "19:meeting_demo@thread.v2");
    assert_eq!(body["meetingInfo"]["tenantId"], "tenant");
    assert_eq!(body["meetingInfo"]["organizerId"], "organizer");
    let headers = &seen.join_headers[0];
    assert_eq!(headers["x-microsoft-skype-chain-id"], "call-1");
    assert_eq!(headers["authorization"], "Bearer ic3");
    assert_eq!(headers["ms-teams-region"], "emea");
    assert!(headers.contains_key("x-microsoft-skype-message-id"));

    meeting.conversation.admit();
    meeting
        .until(|s| s.phase == Phase::Connected && s.roster.contains(&"8:orgid:organizer".into()))
        .await;
    meeting.conversation.roster("8:orgid:bob", Some("Bob"));
    meeting
        .until(|s| s.roster.contains(&"8:orgid:bob".into()))
        .await;
    meeting.conversation.roster("8:orgid:bob", None);
    meeting
        .until(|s| !s.roster.contains(&"8:orgid:bob".into()))
        .await;

    meeting.conversation.redirect();
    meeting.until(|s| s.phase == Phase::Connecting).await;
    meeting.until(|s| s.phase == Phase::Connected).await;
    assert_eq!(meeting.conversation.seen().joins.len(), 2);

    meeting.inputs.send(Input::Hangup).unwrap();
    let session = tokio::time::timeout(Duration::from_secs(5), meeting.driver)
        .await
        .unwrap()
        .unwrap();
    let Phase::Ended(reason) = &session.phase else {
        panic!("{:?}", session.phase)
    };
    assert!(reason.local);
    let leaves = meeting.conversation.seen().leaves;
    assert_eq!(leaves.len(), 1);
    assert_eq!(
        leaves[0]["participants"]["from"]["endpointId"],
        "endpoint-1"
    );
}

#[tokio::test]
async fn the_organizer_ending_the_meeting_ends_the_session() {
    let mut meeting = join(false).await;
    meeting
        .until(|s| s.phase == Phase::Connected && !s.roster.is_empty())
        .await;
    meeting.conversation.end();
    let session = tokio::time::timeout(Duration::from_secs(5), meeting.driver)
        .await
        .unwrap()
        .unwrap();
    let Phase::Ended(reason) = &session.phase else {
        panic!()
    };
    assert!(!reason.local);
    assert_eq!(reason.phrase, "MeetingEnded");
    assert!(meeting.conversation.seen().leaves.is_empty());
}
