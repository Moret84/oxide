//! Demo fakes for status and automatic replies, GIFs, team creation and gallery, and bot card actions; data is fictitious.
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
/// `Action.Execute` answers with a replacement card; other invokes are acknowledged.
async fn invoke(Path(_bot): Path<String>, Json(body): Json<Value>) -> Json<Value> {
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
}
