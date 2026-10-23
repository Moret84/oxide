//! Demo fakes for status and automatic replies, GIFs, team creation and gallery, bot card actions and a message extension; data is fictitious.
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Me {
    availability: Option<String>,
    expiry: Option<String>,
    note: Option<(String, Option<String>)>,
    replies: Option<Value>,
}
fn me() -> &'static Mutex<Me> {
    static ME: OnceLock<Mutex<Me>> = OnceLock::new();
    ME.get_or_init(Mutex::default)
}
/// The demo user's presence entry for `getpresence`.
pub(crate) fn my_presence(mri: &str) -> Value {
    let me = me().lock().unwrap();
    let ooo = me
        .replies
        .as_ref()
        .is_some_and(|r| r["status"] != "disabled");
    let mut presence = json!({
        "availability": me.availability.as_deref().unwrap_or("Available"),
        "activity": me.availability.as_deref().unwrap_or("Available"),
        "calendarData": {"isOutOfOffice": ooo, "outOfOfficeNote": {"message": me
            .replies.as_ref().and_then(|r| r["internalReplyMessage"].as_str()).unwrap_or_default()}},
    });
    if let Some(expiry) = &me.expiry {
        presence["forcedAvailability"] = json!({"expiry": expiry});
    }
    if let Some((message, expiry)) = &me.note {
        presence["note"] = json!({"message": message, "expiry": expiry});
    }
    json!({"mri": mri, "source": "ups", "status": 20000, "presence": presence})
}
async fn force(body: Option<Json<Value>>) -> StatusCode {
    let mut me = me().lock().unwrap();
    match body {
        Some(Json(body)) => {
            me.availability = body["availability"].as_str().map(str::to_owned);
            me.expiry = body["desiredExpirationTime"]
                .as_str()
                .map(str::to_owned)
                .or(Some("9999-12-31T00:00:00.000Z".into()));
        }
        None => {
            me.availability = None;
            me.expiry = None;
        }
    }
    StatusCode::OK
}
async fn note(Json(body): Json<Value>) -> StatusCode {
    let message = body["message"].as_str().unwrap_or_default().to_owned();
    me().lock().unwrap().note =
        (!message.is_empty()).then(|| (message, body["expiry"].as_str().map(str::to_owned)));
    StatusCode::OK
}
async fn replies() -> Json<Value> {
    Json(me().lock().unwrap().replies.clone().unwrap_or_else(
        || json!({"status": "disabled", "internalReplyMessage": "", "externalReplyMessage": "", "externalAudience": "none"}),
    ))
}
async fn set_replies(Json(body): Json<Value>) -> StatusCode {
    me().lock().unwrap().replies = Some(body);
    StatusCode::OK
}
/// Stickers of the Teams CDN stand in for Giphy results.
async fn giphy(Path(_command): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let query = body
        .as_array()
        .and_then(|a| a.iter().find(|p| p["name"] == "query"))
        .and_then(|p| p["value"].as_str())
        .unwrap_or_default()
        .to_owned();
    let names = ["Clippy_Yeah", "Clippy_Hello", "Clippy_Thanks", "Clippy_Wow"];
    // The command's real shape: the GIFs sit in the first entry's `result`.
    Json(
        json!({"commandId": "giphy", "parameters": [], "schema": {}, "result": [{"result": names.iter().map(|n| {
        let url = format!("https://statics.teams.cdn.office.net/evergreen-assets/stickerassets/clippy-250x250/{n}.gif?v=5");
        json!({"composeValue": url, "composeType": "image", "fieldValues": [
            {"fieldName": "previewImage", "fieldValue": url},
            {"fieldName": "width", "fieldValue": "250"}, {"fieldName": "height", "fieldValue": "250"},
            {"fieldName": "title", "fieldValue": format!("{} {query}", n.replace('_', " ")).trim()}]})
    }).collect::<Vec<_>>()}]}),
    )
}
fn gallery() -> Value {
    json!({"value": [
        {"groupId": "g-sales", "displayName": "Ventes Europe", "description": "Pipeline, offres et retours clients.",
         "accessType": "Public", "relevantMembers": [{"smtpAddress": "alice@example.com"}]},
        {"groupId": "g-rust", "displayName": "Communauté Rust", "description": "Entraide et veille technique.",
         "accessType": "Public", "relevantMembers": []},
        {"groupId": "g-board", "displayName": "Comité de direction", "description": "Sur demande.",
         "accessType": "Private", "relevantMembers": []}
    ], "skipToken": null})
}
async fn create_team(Json(body): Json<Value>) -> Json<Value> {
    let name = body["displayName"].as_str().unwrap_or("Équipe");
    Json(
        json!({"value": {"skypeThreadId": format!("19:demo-{}@thread.tacv2", name.len()),
        "siteInfo": {"groupId": "g-new"}, "displayName": name}}),
    )
}
async fn create_channel(Path(_team): Path<String>) -> Json<Value> {
    Json(json!({"value": {"objectId": "19:demo-channel@thread.tacv2"}}))
}
async fn join_code() -> Json<Value> {
    Json(json!("x7k2p9q"))
}
async fn join_by_code(Json(code): Json<Value>) -> Result<Json<Value>, StatusCode> {
    if code.as_str() == Some("x7k2p9q") {
        Ok(Json(
            json!({"value": {"skypeThreadId": "19:product-team@thread.tacv2",
            "siteInfo": {"groupId": "g-product"}, "isTeamOwnerApprovalRequired": false}}),
        ))
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}
/// The demo's message extension: an app to search tickets and to open one.
const TICKETS_APP: &str = "demo-tickets-app";
async fn entitlements() -> Json<Value> {
    Json(json!({"appEntitlements": [{"id": TICKETS_APP, "state": "Installed"}]}))
}
fn tickets_definition() -> Value {
    json!({"id": TICKETS_APP, "name": "Tickets", "shortDescription": "Suivi des demandes",
            "largeImageUrl": "https://statics.teams.cdn.office.net/evergreen-assets/apps/tasks_largeimage.png",
            "inputExtensions": [{"botId": "demo-tickets-bot", "commands": [
                {"id": "searchTickets", "type": "query", "title": "Rechercher", "description": "Trouver une demande par mot-clé",
                 "initialRun": true, "context": ["compose", "commandBox"],
                 "parameters": [{"name": "keyword", "title": "Mot-clé", "inputType": "text"}]},
                {"id": "newTicket", "type": "action", "title": "Nouvelle demande", "fetchTask": true, "context": ["compose"]}
            ]}]})
}
/// As on a real account seen 2026-10-23: the batch answers nothing for the entitled apps.
async fn app_definitions() -> Json<Value> {
    Json(json!([]))
}
/// `definitionsv2`, where the flyout finds the apps.
async fn definitions_v2() -> Json<Value> {
    Json(json!({"value": [tickets_definition()]}))
}
fn ticket_card(id: u32, title: &str, state: &str) -> Value {
    json!({"type": "AdaptiveCard", "version": "1.4", "body": [
        {"type": "TextBlock", "text": format!("#{id} · {title}"), "weight": "Bolder", "size": "Medium"},
        {"type": "FactSet", "facts": [{"title": "État", "value": state}, {"title": "Équipe", "value": "Support"}]}
    ], "actions": [{"type": "Action.OpenUrl", "title": "Ouvrir", "url": format!("https://example.com/tickets/{id}")}]})
}
fn compose_extension(body: &Value) -> Value {
    let value = &body["value"];
    match body["name"].as_str().unwrap_or_default() {
        "composeExtension/query" => {
            let keyword = value["parameters"][0]["value"].as_str().unwrap_or_default();
            let keyword = if keyword == "true" { "" } else { keyword };
            let tickets = [(101, "Écran noir au démarrage", "Ouvert"), (102, "Accès VPN refusé", "En cours"),
                (103, "Commande de clavier", "Résolu")];
            let attachments: Vec<Value> = tickets.iter()
                .filter(|(_, t, _)| keyword.is_empty() || t.to_lowercase().contains(&keyword.to_lowercase()))
                .map(|(id, title, state)| json!({
                    "contentType": "application/vnd.microsoft.card.adaptive", "content": ticket_card(*id, title, state),
                    "preview": {"contentType": "application/vnd.microsoft.card.thumbnail", "content": {
                        "title": format!("#{id} · {title}"), "text": state,
                        // The last ticket's card comes from the bot when picked.
                        "tap": if *id == 103 { json!({"type": "invoke", "value": {"ticket": id}}) } else { Value::Null }}}}))
                .collect();
            json!({"composeExtension": {"type": "result", "attachmentLayout": "list", "attachments": attachments}})
        }
        "composeExtension/selectItem" => json!({"composeExtension": {"type": "result", "attachments": [
            {"contentType": "application/vnd.microsoft.card.adaptive", "content": ticket_card(103, "Commande de clavier", "Résolu")}]}}),
        "composeExtension/fetchTask" => json!({"task": {"type": "continue", "value": {"title": "Nouvelle demande",
            "card": {"contentType": "application/vnd.microsoft.card.adaptive", "content": {"type": "AdaptiveCard", "version": "1.4",
                "body": [{"type": "Input.Text", "id": "title", "label": "Titre", "isRequired": true},
                         {"type": "Input.ChoiceSet", "id": "state", "label": "Priorité", "value": "Normale",
                          "choices": [{"title": "Normale", "value": "Normale"}, {"title": "Haute", "value": "Haute"}]}],
                "actions": [{"type": "Action.Submit", "title": "Créer"}]}}}}}),
        "composeExtension/submitAction" => {
            let title = value["data"]["title"].as_str().unwrap_or("Sans titre");
            let priority = value["data"]["state"].as_str().unwrap_or("Normale");
            json!({"composeExtension": {"type": "result", "attachmentLayout": "list", "attachments": [
                {"contentType": "application/vnd.microsoft.card.adaptive", "content": ticket_card(104, title, priority)}]}})
        }
        _ => json!({}),
    }
}
/// `Action.Execute` answers with a replacement card; message extensions answer as the demo app; other invokes are acknowledged.
async fn invoke(Path(_bot): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    if body["name"].as_str().is_some_and(|n| n.starts_with("composeExtension/")) {
        return Json(compose_extension(&body));
    }
    if body["name"] == "adaptiveCard/action" {
        return Json(
            json!({"statusCode": 200, "type": "application/vnd.microsoft.card.adaptive",
            "value": {"type": "AdaptiveCard", "body": [{"type": "TextBlock", "text": "Merci, c’est noté.", "weight": "Bolder"}]}}),
        );
    }
    Json(json!({}))
}
async fn ok() -> StatusCode {
    StatusCode::OK
}
pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/me/forceavailability/", put(force))
        .route("/v1/me/endpoints/", put(ok))
        .route("/v1/me/endpoints/{id}", axum::routing::delete(ok))
        .route("/v1/me/reportmyactivity/", post(ok))
        .route("/v1/me/publishnote", put(note))
        .route("/beta/users/automaticRepliesSetting", get(replies))
        .route(
            "/beta/users/updateAutomaticRepliesSetting",
            post(set_replies),
        )
        .route("/beta/commands/{command}/execute", post(giphy))
        .route("/beta/teams/suggested", get(|| async { Json(gallery()) }))
        .route("/beta/teams/find", get(|| async { Json(gallery()) }))
        .route("/beta/teams/joinRequest/{group}", post(ok))
        .route("/beta/teams/create", post(create_team))
        .route("/beta/teams/joinByCode", post(join_by_code))
        .route("/beta/teams/{team}/channels", post(create_channel))
        .route("/beta/teams/{team}/joinCode", get(join_code).put(join_code))
        .route(
            "/beta/teams/{team}/members",
            put(|| async { Json(json!({})) }),
        )
        .route("/beta/teams/{team}/setDefaultChannel", put(ok).delete(ok))
        .route("/api/v1/teams/users/me/conversationFolders", post(ok))
        .route("/v1/agents/{bot}/invoke", post(invoke))
        .route("/beta/users/apps/entitlements", get(entitlements))
        .route("/beta/users/apps/batchedDefinitions", post(app_definitions))
        .route("/beta/users/apps/definitionsv2", get(definitions_v2))
}

#[cfg(test)]
mod tests {
    use crate::app::chat_admin::{ChatAdminAction, ChatAdminMsg};
    use crate::app::gifs::{GifsAction, GifsMsg};
    use crate::app::me::{MeAction, MeMsg, MyStatus, OutOfOffice, StatusDuration};
    use crate::app::search::{SearchAction, SearchFiltersView, SearchMsg};
    use crate::app::teams_admin::{NewTeamVisibility, TeamsAdminAction, TeamsAdminMsg};
    use crate::app::view::PresenceStatus;
    use crate::app::{App, Msg};
    use crate::task::{Job, Task};
    use std::time::Duration;

    /// Runs the network work of `task`, and of what its answers start, to the end.
    async fn run(app: &mut App, task: Task<Msg>) {
        let mut jobs = task.0;
        while let Some(job) = jobs.pop() {
            if let Job::Future(future) = job
                && let Ok(msg) = tokio::time::timeout(Duration::from_secs(15), future).await
            {
                jobs.extend(app.update(msg).0);
            }
        }
    }
    async fn send(app: &mut App, msg: Msg) {
        let task = app.update(msg);
        run(app, task).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_demo_serves_every_management_feature() {
        let (mut app, task) = App::boot(true);
        run(&mut app, task).await;

        let task = app.update(Msg::Me(MeMsg::Action(MeAction::SetStatus {
            status: MyStatus::DoNotDisturb,
            duration: StatusDuration::OneHour,
        })));
        // UPS may still report the previous status a moment after the change.
        let stale = teams_chat::Presence {
            mri: crate::mock::ME.into(),
            availability: "Available".into(),
            ..Default::default()
        };
        let generation = app.generation();
        let _ = app.update(Msg::Me(MeMsg::Loaded(generation, Ok(Some(stale)))));
        assert_eq!(
            app.me_view().presence.unwrap().status,
            PresenceStatus::DoNotDisturb,
            "a stale answer does not undo the choice"
        );
        run(&mut app, task).await;
        let me = app.me_view();
        assert_eq!(
            me.presence.as_ref().unwrap().status,
            PresenceStatus::DoNotDisturb
        );
        assert!(me.forced && me.forced_until.is_some(), "{me:?}");
        send(
            &mut app,
            Msg::Me(MeMsg::Action(MeAction::SetNote {
                text: "En congés".into(),
                duration: StatusDuration::Never,
            })),
        )
        .await;
        assert_eq!(app.me_view().note, "En congés");

        send(&mut app, Msg::Me(MeMsg::Action(MeAction::LoadOutOfOffice))).await;
        let settings = app.me_view().out_of_office_settings.settings.unwrap();
        assert!(!settings.enabled);
        send(
            &mut app,
            Msg::Me(MeMsg::Action(MeAction::SaveOutOfOffice {
                settings: OutOfOffice {
                    enabled: true,
                    internal_message: "Absent jusqu’à lundi".into(),
                    ..settings
                },
            })),
        )
        .await;
        let me = app.me_view();
        assert_eq!(me.out_of_office_settings.saved, 1, "{me:?}");
        assert!(me.out_of_office);

        send(
            &mut app,
            Msg::Gifs(GifsMsg::Action(GifsAction::Search {
                query: "bravo".into(),
            })),
        )
        .await;
        assert_eq!(app.gifs_view().results.len(), 4);

        send(
            &mut app,
            Msg::TeamsAdmin(TeamsAdminMsg::Action(TeamsAdminAction::OpenGallery)),
        )
        .await;
        let gallery = app.teams_admin_view().gallery.unwrap();
        assert_eq!(gallery.teams.len(), 3);
        send(
            &mut app,
            Msg::TeamsAdmin(TeamsAdminMsg::Action(TeamsAdminAction::CreateTeam {
                name: "Lancement".into(),
                description: String::new(),
                visibility: NewTeamVisibility::Private,
            })),
        )
        .await;
        let admin = app.teams_admin_view();
        assert_eq!((admin.created, admin.error), (1, None));

        send(
            &mut app,
            Msg::FullSearch(SearchMsg::Action(SearchAction::Run {
                query: "exemple".into(),
                filters: SearchFiltersView::default(),
            })),
        )
        .await;
        let search = app.search_view();
        assert!(search.searched && search.error.is_none(), "{search:?}");
        assert!(!search.hits.is_empty(), "{search:?}");

        let chat = app.first_chat_id();
        send(
            &mut app,
            Msg::ChatAdmin(ChatAdminMsg::Action(ChatAdminAction::SetPinned {
                chat_id: chat.clone(),
                pinned: true,
            })),
        )
        .await;
        assert_eq!(app.chat_admin_error(), None);
        assert!(app.is_favorite_id(&chat));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_demo_message_extension_searches_picks_and_sends_cards() {
        use crate::app::Selection;
        use crate::app::extensions::{ExtensionsAction, ExtensionsMsg};
        use crate::app::view::CardInputValue;
        let action = |a| Msg::Extensions(ExtensionsMsg::Action(a));
        let (mut app, task) = App::boot(true);
        run(&mut app, task).await;
        let chat = app.first_chat_id();
        send(&mut app, Msg::Select(Selection::Chat(chat))).await;

        send(&mut app, action(ExtensionsAction::Load)).await;
        let view = app.extensions_view();
        assert_eq!(view.apps.len(), 1, "{view:?}");
        let tickets = view.apps[0].clone();
        assert_eq!(tickets.commands.len(), 2);
        let choose = |command: &str| {
            action(ExtensionsAction::Choose {
                app_id: tickets.id.clone(),
                command_id: command.into(),
            })
        };
        send(&mut app, choose("searchTickets")).await;
        assert_eq!(app.extensions_view().results.len(), 3, "initial run");
        send(&mut app, action(ExtensionsAction::Search { query: "vpn".into() })).await;
        let results = app.extensions_view().results;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "#102 · Accès VPN refusé");
        send(&mut app, action(ExtensionsAction::Pick { index: 0 })).await;
        // A result with a `tap` asks the bot for its card.
        send(&mut app, action(ExtensionsAction::Search { query: String::new() })).await;
        send(&mut app, action(ExtensionsAction::Pick { index: 2 })).await;
        let cards = app.composer_cards();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[1].card.items.len(), 2, "{:?}", cards[1]);
        assert_eq!(app.extensions_view().inserted, 2);

        send(&mut app, choose("newTicket")).await;
        let form = app.extensions_view().form.expect("form");
        assert_eq!(form.title, "Nouvelle demande");
        let submit = |inputs: Vec<CardInputValue>| {
            action(ExtensionsAction::FormAction { action: 0, inputs })
        };
        send(&mut app, submit(vec![])).await;
        assert!(app.extensions_view().form.unwrap().error.is_some(), "title is required");
        send(
            &mut app,
            submit(vec![CardInputValue {
                id: "title".into(),
                value: "Badge perdu".into(),
            }]),
        )
        .await;
        assert!(app.extensions_view().form.is_none());
        assert_eq!(app.composer_cards().len(), 3);

        send(&mut app, action(ExtensionsAction::RemoveCard { index: 0 })).await;
        assert_eq!(app.composer_cards().len(), 2);
        send(&mut app, Msg::Send).await;
        assert!(app.composer_cards().is_empty(), "the cards went out");
        assert_eq!(app.error_text(), None);
    }
}
