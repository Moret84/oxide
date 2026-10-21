//! Checks the calling protocol against a real tenant (gates G0–G3); own Keychain entry since shared rotating refresh tokens would sign the app out.
use std::process::ExitCode;
use std::sync::Arc;

use http::HeaderMap;
use teams_auth::{
    AuthError, Authenticator, LoginAttempt, RefreshTokenStore, Service, TeamsAuthenticator,
};
use teams_calling::history::{self, CALL_LOGS};
use teams_calling::session::{CallKind, CallSession, Input, Phase};
use teams_calling::{CallState, VoicemailClient};
use teams_chat::{ChatClient, MessagesQuery};
use tokio::io::{AsyncBufReadExt, BufReader};

const USAGE: &str = "Usage : teams-probe <commande>

  login       connexion Microsoft (coller l’URL de retour)
  logout      efface la session du probe
  discovery   URL des services d’appel (regionGtms ou repli)
  tokens      obtention des jetons par service (sans les afficher)
  history     historique d’appels (CSA, puis repli 48:calllogs)
  voicemail   5 derniers messages vocaux
  join <lien> [minutes]
              rejoint une réunion sans média (roster seul), puis la quitte
  incoming    attend un appel entrant et le refuse (vérifie attach + reject)
  trouter [minutes]
              canal d’appels (Trouter + registrar) ; appelez ce compte pour voir
              arriver la notification (10 min par défaut, Ctrl-C pour arrêter)";

#[tokio::main]
async fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_default();
    let result = match command.as_str() {
        "login" => login().await,
        "logout" => Keychain.clear().map_err(|e| e.to_string()),
        "discovery" => with_session(discovery).await,
        "tokens" => with_session(tokens).await,
        "history" => with_session(call_history).await,
        "voicemail" => with_session(voicemail).await,
        "trouter" => with_session(trouter).await,
        "join" => with_session(join).await,
        "incoming" => with_session(incoming).await,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("échec : {error}");
            ExitCode::FAILURE
        }
    }
}

async fn login() -> Result<(), String> {
    teams_auth::client_id::resolve()
        .await
        .map_err(|e| e.to_string())?;
    let attempt = LoginAttempt::new().map_err(|e| e.to_string())?;
    println!("1. Ouvrez cette URL dans un navigateur et connectez-vous :\n");
    println!("{}\n", attempt.authorization_url());
    println!(
        "2. Après la redirection vers teams.microsoft.com/v2, copiez l’URL complète de la barre d’adresse et collez-la ici :"
    );
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    let auth = attempt.complete(&line).await.map_err(|e| e.to_string())?;
    auth.remember_session(Arc::new(Keychain))
        .map_err(|e| e.to_string())?;
    println!("Connecté : {} ({})", auth.display_name(), auth.user_id());
    Ok(())
}

async fn with_session<F, Fut>(run: F) -> Result<(), String>
where
    F: FnOnce(Arc<TeamsAuthenticator>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let token = Keychain
        .read()?
        .ok_or("Aucune session : lancez d’abord `teams-probe login`.")?;
    let auth = Arc::new(
        TeamsAuthenticator::restore(&token)
            .await
            .map_err(|e| e.to_string())?,
    );
    auth.remember_session(Arc::new(Keychain))
        .map_err(|e| e.to_string())?;
    run(auth).await
}

const CALLING_SERVICES: [Service; 7] = [
    Service::CallingConversation,
    Service::CallingConversationBase,
    Service::Trouter,
    Service::Registrar,
    Service::Trap,
    Service::Outlook,
    Service::Csa,
];

/// A message of `48:calllogs`, as the calling crate reads it.
fn log_message(m: &teams_chat::Message) -> history::LogMessage<'_> {
    history::LogMessage {
        id: &m.id,
        original_arrival_time: m.originalarrivaltime.as_deref(),
        call_log: m.properties.get("call-log"),
        is_deleted: m.is_deleted(),
    }
}

async fn discovery(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let context = auth.calling_context().unwrap_or_default();
    println!("utilisateur : {}", context.user_mri);
    println!(
        "tenant      : {}",
        context.tenant_id.as_deref().unwrap_or("?")
    );
    println!(
        "routage     : region={} partition={} ring={}",
        context.region.as_deref().unwrap_or("?"),
        context.partition.as_deref().unwrap_or("?"),
        context.ring.as_deref().unwrap_or("?")
    );
    for service in CALLING_SERVICES {
        match auth.base_url(service) {
            Ok(mut url) => {
                url.set_query(None);
                println!("{:<26} {url}", service.key());
            }
            Err(e) => println!("{:<26} indisponible ({e})", service.key()),
        }
    }
    Ok(())
}

async fn tokens(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    for service in CALLING_SERVICES {
        let mut headers = HeaderMap::new();
        match auth.authorize(service, &mut headers).await {
            Ok(()) => {
                let names: Vec<_> = headers.keys().map(|k| k.as_str()).collect();
                println!("{:<26} ok : {}", service.key(), names.join(", "));
            }
            Err(e) => println!("{:<26} ÉCHEC : {}", service.key(), redact(&e)),
        }
    }
    Ok(())
}

async fn call_history(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let me = auth.user_id().to_owned();
    let chat = ChatClient::new(auth);
    let messages = match chat.call_history(None, 20).await {
        Ok(page) => {
            println!(
                "CSA callhistory : {} messages, continuation={}",
                page.messages.len(),
                page.continuation_token.is_some()
            );
            page.messages
        }
        Err(e) => {
            println!("CSA callhistory : ÉCHEC ({e}) ; repli sur {CALL_LOGS}");
            let query = MessagesQuery {
                page_size: Some(20),
                ..Default::default()
            };
            chat.messages(CALL_LOGS, &query)
                .await
                .map_err(|e| e.to_string())?
                .messages
        }
    };
    let logs = history::visible(messages.iter().map(log_message));
    println!("{} appels visibles", logs.len());
    for log in logs {
        let who = log
            .counterpart(&me)
            .map(|p| p.display_name.clone().unwrap_or_else(|| p.id.clone()))
            .unwrap_or_else(|| "?".into());
        let state = match &log.state {
            CallState::Missed => "manqué".to_owned(),
            CallState::Accepted => "décroché".to_owned(),
            CallState::Declined => "refusé".to_owned(),
            CallState::Other(s) => s.clone(),
        };
        println!(
            "{} {:?} {state:<9} {who} {}",
            log.start_time.as_deref().unwrap_or("?"),
            log.direction,
            log.duration_secs()
                .map(|s| format!("{s} s"))
                .unwrap_or_default()
        );
    }
    let unknown: std::collections::BTreeSet<String> = messages
        .iter()
        .map(log_message)
        .filter_map(teams_calling::CallLog::from_message)
        .filter_map(|l| {
            l.raw
                .as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
        })
        .flatten()
        .collect();
    println!(
        "champs call-log observés : {}",
        unknown.into_iter().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}

async fn voicemail(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let list = VoicemailClient::new(auth)
        .list(5)
        .await
        .map_err(|e| e.to_string())?;
    println!("{} messages vocaux", list.len());
    for v in list {
        println!(
            "{} {} {} {}{}",
            v.received.as_deref().unwrap_or("?"),
            if v.is_read { "lu    " } else { "non lu" },
            v.sender_name.as_deref().unwrap_or("?"),
            v.duration_secs
                .map(|d| format!("{d} s"))
                .unwrap_or_default(),
            if v.transcription.is_some() {
                " (transcrit)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// Error texts never embed tokens, but an `access_token=` query could leak through a URL.
fn redact(error: &AuthError) -> String {
    let text = error.to_string();
    match text.find("access_token=") {
        Some(i) => format!("{}access_token=[REDACTED]", &text[..i]),
        None => text,
    }
}

struct Keychain;
#[cfg(target_os = "macos")]
impl Keychain {
    fn entry() -> Result<keyring::Entry, AuthError> {
        keyring::Entry::new("oxide.probe", "current-account")
            .map_err(|_| AuthError::Failed("Trousseau macOS indisponible.".into()))
    }
    fn read(&self) -> Result<Option<String>, String> {
        match Self::entry().map_err(|e| e.to_string())?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Lecture du Trousseau impossible.".into()),
        }
    }
}
#[cfg(target_os = "macos")]
impl RefreshTokenStore for Keychain {
    fn save(&self, token: &str) -> Result<(), AuthError> {
        Self::entry()?
            .set_password(token)
            .map_err(|_| AuthError::Failed("Écriture du Trousseau impossible.".into()))
    }
    fn clear(&self) -> Result<(), AuthError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(AuthError::Failed(
                "Effacement du Trousseau impossible.".into(),
            )),
        }
    }
}
#[cfg(not(target_os = "macos"))]
impl Keychain {
    fn read(&self) -> Result<Option<String>, String> {
        Ok(std::env::var("TEAMS_PROBE_REFRESH_TOKEN").ok())
    }
}
#[cfg(not(target_os = "macos"))]
impl RefreshTokenStore for Keychain {
    fn save(&self, _token: &str) -> Result<(), AuthError> {
        Ok(())
    }
    fn clear(&self) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Gate G1: connect, register, then print every pushed request without its secrets.
async fn trouter(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let minutes = minutes_arg(2, 10);
    let config = calling_config();
    println!(
        "endpoint {} ; écoute pendant {minutes} min",
        config.endpoint_id
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let sink: teams_realtime::Sink = Arc::new(|event| {
        use teams_realtime::RealtimeEvent::*;
        match event {
            Request(request) => {
                let body: Option<serde_json::Value> = serde_json::from_str(&request.body).ok();
                let event_id = body
                    .as_ref()
                    .and_then(|b| b.get("eventId").or_else(|| b.get("evt")))
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "-".into());
                let keys = body
                    .as_ref()
                    .and_then(|b| b.as_object())
                    .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                    .unwrap_or_default();
                let headers: Vec<_> = request.headers.keys().cloned().collect();
                println!(
                    "← {} {} eventId={event_id} corps={} octets clés=[{keys}] headers=[{}]",
                    request.method,
                    redact_path(&request.path),
                    request.body.len(),
                    headers.join(",")
                );
            }
            Connected { surl } => println!("connecté : {}", redact_path(&surl)),
            other => println!("{other:?}"),
        }
    });
    let run = tokio::spawn(teams_realtime::run(auth, config, sink, stopped));
    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_secs(minutes * 60)) => {}
        _ = tokio::signal::ctrl_c() => println!("arrêt…"),
    }
    let _ = stop.send(true);
    let _ = run.await;
    Ok(())
}

/// Minutes given as command-line argument `index`, else `default`.
fn minutes_arg(index: usize, default: u64) -> u64 {
    std::env::args()
        .nth(index)
        .and_then(|m| m.parse().ok())
        .unwrap_or(default)
}

/// A fresh endpoint and session for the calling push channel.
fn calling_config() -> teams_realtime::RealtimeConfig {
    teams_realtime::RealtimeConfig::calling(
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    )
}

/// Session URLs may carry opaque routing tokens: keep only the shape.
fn redact_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.len() > 24 {
                format!("{}…", segment.chars().take(8).collect::<String>())
            } else {
                segment.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

type Routes = Arc<std::sync::Mutex<Option<(String, tokio::sync::mpsc::UnboundedSender<Input>)>>>;

/// Starts the push channel and waits for its registration; requests not routed to the call go to `other`.
async fn channel(
    auth: Arc<TeamsAuthenticator>,
    routes: Routes,
    other: impl Fn(teams_realtime::PushedRequest) + Send + Sync + 'static,
) -> Result<
    (
        teams_realtime::RealtimeConfig,
        String,
        tokio::sync::watch::Sender<bool>,
    ),
    String,
> {
    let config = calling_config();
    let (ready_tx, mut ready) = tokio::sync::mpsc::unbounded_channel();
    let surl = Arc::new(std::sync::Mutex::new(String::new()));
    let seen_surl = surl.clone();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let sink: teams_realtime::Sink = Arc::new(move |event| {
        use teams_realtime::RealtimeEvent::*;
        match event {
            Connected { surl } => *seen_surl.lock().unwrap() = surl,
            Registered { .. } => {
                let _ = ready_tx.send(Ok(()));
            }
            Fatal(reason) => {
                let _ = ready_tx.send(Err(reason));
            }
            Request(request) => {
                let routed = routes.lock().unwrap().as_ref().and_then(|(call, tx)| {
                    let input = teams_calling::driver::route(
                        call,
                        &request.path,
                        &request.headers,
                        &request.body,
                    )?;
                    tx.send(input).ok()
                });
                if routed.is_none() {
                    other(request);
                }
            }
            other => println!("{other:?}"),
        }
    });
    tokio::spawn(teams_realtime::run(auth, config.clone(), sink, stopped));
    match tokio::time::timeout(std::time::Duration::from_secs(60), ready.recv()).await {
        Ok(Some(Ok(()))) => {}
        Ok(Some(Err(reason))) => return Err(reason),
        _ => return Err("Canal d’appels non enregistré au bout de 60 s.".into()),
    }
    let surl = surl.lock().unwrap().clone();
    Ok((config, surl, stop))
}

fn local(auth: &TeamsAuthenticator, endpoint: &str) -> teams_calling::payload::Local {
    teams_calling::payload::Local {
        id: auth.user_id().to_owned(),
        display_name: auth.display_name().to_owned(),
        endpoint_id: endpoint.to_owned(),
        participant_id: uuid::Uuid::new_v4().to_string(),
        language_id: "fr-fr".into(),
    }
}

fn describe(session: &CallSession) -> String {
    let phase = match &session.phase {
        Phase::Ended(reason) => format!(
            "terminé : {} (code {}/{})",
            reason.message(),
            reason.code,
            reason.sub_code
        ),
        other => format!("{other:?}"),
    };
    let names: Vec<_> = session
        .roster
        .values()
        .map(|p| p.display_name.clone().unwrap_or_else(|| p.id.clone()))
        .collect();
    format!(
        "{phase} ; {} participant(s) : {}",
        names.len(),
        names.join(", ")
    )
}

/// Gate G2: roster-only meeting join, then leave.
async fn join(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let link = std::env::args()
        .nth(2)
        .ok_or("Usage : teams-probe join <lien de réunion> [minutes]")?;
    let minutes = minutes_arg(3, 2);
    let meeting = match teams_calling::MeetingLink::parse(&link) {
        Some(teams_calling::MeetingLink::Coordinates(meeting)) => meeting,
        Some(teams_calling::MeetingLink::Code { .. }) => return Err(
            "Les liens /meet/ ne sont pas encore pris en charge : utilisez un lien meetup-join."
                .into(),
        ),
        None => return Err("Lien de réunion Teams non reconnu.".into()),
    };
    let routes: Routes = Arc::default();
    let (config, surl, stop) = channel(auth.clone(), routes.clone(), |request| {
        println!(
            "← non routé : {} {}",
            request.method,
            redact_path(&request.path)
        );
    })
    .await?;
    let client = teams_calling::client::ConversationClient::new(auth.clone());
    let service = client
        .conversation_service()
        .ok_or("Service de conversation inconnu.")?;
    let call = uuid::Uuid::new_v4().to_string();
    let session = CallSession::new(
        call.clone(),
        local(&auth, &config.endpoint_id),
        teams_calling::payload::Callbacks::new(&surl, &config.session_id, &call),
        CallKind::Meeting(meeting),
        None,
        service,
    );
    let (inputs, receiver) = tokio::sync::mpsc::unbounded_channel();
    *routes.lock().unwrap() = Some((call, inputs.clone()));
    let mut last = String::new();
    let mut driver = tokio::spawn(teams_calling::driver::drive(
        client,
        session,
        receiver,
        inputs.clone(),
        move |event| {
            if let teams_calling::driver::Event::Changed(session) = event {
                let now = describe(session);
                if now != last {
                    println!("{now}");
                    last = now;
                }
            }
        },
    ));
    tokio::select! {
        _ = &mut driver => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(minutes * 60)) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    if !driver.is_finished() {
        println!("on quitte la réunion…");
        let _ = inputs.send(Input::Hangup);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(15), driver).await;
    }
    let _ = stop.send(true);
    Ok(())
}

/// Gate G2: wait for an incoming call, attach, then decline it.
async fn incoming(auth: Arc<TeamsAuthenticator>) -> Result<(), String> {
    let routes: Routes = Arc::default();
    let (calls_tx, mut calls) = tokio::sync::mpsc::unbounded_channel();
    let (config, surl, stop) = channel(auth.clone(), routes.clone(), move |request| {
        let body = teams_calling::callbacks::decode_body(&request.body, &request.headers);
        match body
            .as_ref()
            .and_then(teams_calling::callbacks::incoming_call)
        {
            Some(call) => {
                println!(
                    "☎ appel de {} sur {}",
                    call.caller_name.clone().unwrap_or(call.caller_id.clone()),
                    redact_path(&request.path)
                );
                let _ = calls_tx.send(call);
            }
            None => println!(
                "← non routé : {} {} ({} octets)",
                request.method,
                redact_path(&request.path),
                request.body.len()
            ),
        }
    })
    .await?;
    println!("Prêt : appelez ce compte (Ctrl-C pour arrêter).");
    let call = tokio::select! {
        call = calls.recv() => call.ok_or("Canal fermé.")?,
        _ = tokio::signal::ctrl_c() => {
            let _ = stop.send(true);
            return Ok(());
        }
    };
    let client = teams_calling::client::ConversationClient::new(auth.clone());
    let id = uuid::Uuid::new_v4().to_string();
    let session = CallSession::new(
        id.clone(),
        local(&auth, &config.endpoint_id),
        teams_calling::payload::Callbacks::new(&surl, &config.session_id, &id),
        CallKind::Incoming(call),
        None,
        String::new(),
    );
    let (inputs, receiver) = tokio::sync::mpsc::unbounded_channel();
    *routes.lock().unwrap() = Some((id, inputs.clone()));
    let decline = inputs.clone();
    let mut declined = false;
    let session =
        teams_calling::driver::drive(
            client,
            session,
            receiver,
            inputs,
            move |event| match event {
                teams_calling::driver::Event::Changed(session) => {
                    println!("{}", describe(session));
                    if !declined && session.link("reject").is_some() {
                        declined = true;
                        println!("on refuse l’appel…");
                        let _ = decline.send(Input::Decline);
                    }
                }
                teams_calling::driver::Event::RemoteMedia(offer) => {
                    println!("offre média reçue ({} octets)", offer.len());
                }
                teams_calling::driver::Event::RemoteProvisionalMedia(offer) => {
                    println!("média provisoire reçu ({} octets)", offer.len());
                }
                teams_calling::driver::Event::RemoteOffer(offer) => {
                    println!("renégociation reçue ({} octets)", offer.len());
                }
            },
        )
        .await;
    println!("fin : {}", describe(&session));
    let _ = stop.send(true);
    Ok(())
}
