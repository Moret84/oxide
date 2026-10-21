//! Creating and joining teams through the middle tier's `teamsAndChannelsService`, as the web client does.
use http::{HeaderMap, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};

use crate::client::ChatClient;
use crate::error::Error;
use crate::path::enc;

/// Who can find and join a new team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamVisibility {
    Private,
    Public,
    /// Everyone in the organization is a member.
    OrgWide,
}
/// A team made from scratch: `POST beta/teams/create`.
#[derive(Debug, Clone)]
pub struct NewTeam {
    pub name: String,
    pub description: String,
    pub visibility: TeamVisibility,
}
/// A team of the "Join or create a team" gallery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryTeam {
    pub group_id: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    /// People the user knows in the team: the web client's "member count".
    pub known_members: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GalleryPage {
    pub teams: Vec<GalleryTeam>,
    pub skip_token: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GalleryEntry {
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    access_type: Value,
    #[serde(default)]
    is_dynamic: bool,
    #[serde(default)]
    relevant_members: Vec<Value>,
    #[serde(default)]
    join_state: Option<String>,
}

/// Body of `beta/teams/create`: `accessType` 1 Private, 3 Public (also org-wide).
fn create_body(team: &NewTeam) -> Value {
    json!({
        "displayName": team.name.trim(),
        "description": team.description.trim(),
        "accessType": if team.visibility == TeamVisibility::Private { 1 } else { 3 },
        "isTenantWide": team.visibility == TeamVisibility::OrgWide,
        "sensitivityLabelId": null,
        "validationRequired": false,
    })
}
/// `{"value": …}` of a middle tier answer, or the answer itself.
fn value(v: Value) -> Value {
    match v {
        Value::Object(mut o) if o.contains_key("value") => o.remove("value").unwrap_or(Value::Null),
        other => other,
    }
}
fn gallery_page(v: Value) -> GalleryPage {
    let skip_token = v
        .get("skipToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let entries: Vec<GalleryEntry> = v
        .get("value")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let public = |a: &Value| a.as_str() == Some("Public") || a.as_i64() == Some(3);
    let private = |a: &Value| a.as_str() == Some("Private") || a.as_i64() == Some(1);
    GalleryPage {
        teams: entries
            .into_iter()
            .filter(|e| {
                !e.group_id.is_empty()
                    && !e.is_dynamic
                    && e.join_state.as_deref() != Some("Joined")
                    && (public(&e.access_type) || private(&e.access_type))
            })
            .map(|e| GalleryTeam {
                private: private(&e.access_type),
                group_id: e.group_id,
                name: e.display_name.unwrap_or_default(),
                description: e.description.unwrap_or_default(),
                known_members: e.relevant_members.len() as u32,
            })
            .collect(),
        skip_token,
    }
}
/// A middle tier error code from an answer that may carry one (`errorCode`, `errors[0]`).
fn error_code(v: &Value) -> Option<String> {
    v.get("errorCode")
        .or_else(|| v.get("errors")?.get(0)?.get("errorCode"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl<A: Authenticator> ChatClient<A> {
    pub(crate) async fn mt(
        &self,
        endpoint: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Error> {
        let response = self
            .service_request(
                Service::MiddleTier,
                endpoint,
                method,
                path,
                body,
                HeaderMap::new(),
            )
            .await?;
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        // Some answers are a bare string, such as a join code.
        Ok(serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())))
    }
    /// Creates a team; returns its thread id (the team and General channel id).
    pub async fn create_team(&self, team: &NewTeam) -> Result<String, Error> {
        let v = value(
            self.mt(
                "createTeam",
                Method::POST,
                "beta/teams/create",
                Some(create_body(team)),
            )
            .await?,
        );
        v.get("skypeThreadId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| missing("createTeam"))
    }
    /// Creates a standard or private channel; returns its id.
    pub async fn create_channel(
        &self,
        team_thread: &str,
        group_id: &str,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<String, Error> {
        let mut body = json!({
            "displayName": name.trim(),
            "description": description.trim(),
            "groupId": group_id,
            "channelType": if private { "Private" } else { "Standard" },
        });
        if private {
            body["useOneDriveStorage"] = json!(false);
        }
        let v = value(
            self.mt(
                "createChannel",
                Method::POST,
                &format!("beta/teams/{}/channels", enc(team_thread)),
                Some(body),
            )
            .await?,
        );
        v.get("objectId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| missing("createChannel"))
    }
    /// "Show for everyone": the channel appears in every member's list.
    pub async fn set_default_channel(&self, channel: &str, on: bool) -> Result<(), Error> {
        self.mt(
            "setChannelShowForMember",
            if on { Method::PUT } else { Method::DELETE },
            &format!("beta/teams/{}/setDefaultChannel", enc(channel)),
            None,
        )
        .await
        .map(drop)
    }
    /// Joins with a team code; returns the team's thread id, `None` when the owner must approve first.
    pub async fn join_team_with_code(&self, code: &str) -> Result<Option<String>, Error> {
        let v = value(
            self.mt(
                "joinTeamWithCode",
                Method::POST,
                "beta/teams/joinByCode?restrictJoinToPublicTeam=false",
                Some(json!(code.trim())),
            )
            .await?,
        );
        if v.get("isTeamOwnerApprovalRequired")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Ok(None);
        }
        v.get("skypeThreadId")
            .and_then(Value::as_str)
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| missing("joinTeamWithCode"))
    }
    /// The team's join code, generated when `reset`.
    pub async fn team_join_code(&self, team_thread: &str, reset: bool) -> Result<String, Error> {
        let v = self
            .mt(
                "teamJoinCode",
                if reset { Method::PUT } else { Method::GET },
                &format!("beta/teams/{}/joinCode", enc(team_thread)),
                None,
            )
            .await?;
        let v = value(v);
        v.as_str()
            .or_else(|| v.get("joinCode").and_then(Value::as_str))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| missing("teamJoinCode"))
    }
    /// Teams suggested in the gallery.
    pub async fn suggested_teams(&self) -> Result<GalleryPage, Error> {
        self.mt(
            "getRecommendedTeams",
            Method::GET,
            "beta/teams/suggested?accessType=All&enableFallback=true",
            None,
        )
        .await
        .map(gallery_page)
    }
    /// Gallery search; the web client only searches past two characters.
    pub async fn find_teams(
        &self,
        keywords: &str,
        skip_token: Option<&str>,
    ) -> Result<GalleryPage, Error> {
        let path = format!(
            "beta/teams/find?keywords={}&accessType=All&enableKeywordSearch=false&top=100&skipToken={}",
            enc(keywords.trim()),
            skip_token.map(enc).unwrap_or_default()
        );
        self.mt("findTeams", Method::GET, &path, None)
            .await
            .map(gallery_page)
    }
    /// Joins a public team, or asks the owners of a private one.
    pub async fn join_team(&self, group_id: &str) -> Result<(), Error> {
        self.mt(
            "joinTeamById",
            Method::POST,
            &format!("beta/teams/joinRequest/{}", enc(group_id)),
            None,
        )
        .await
        .map(drop)
    }
    /// Leaves a team; a refusal is a middle tier error code such as `OperationDeniedAsMemberIsLastAdminInTeam`.
    pub async fn leave_team(
        &self,
        team_thread: &str,
        me: &str,
        group_id: &str,
    ) -> Result<Option<String>, Error> {
        let body = json!({"userMri": me, "updateType": "DemotedAndLeft", "groupId": group_id});
        match self
            .mt(
                "leaveTeamRequest",
                Method::PUT,
                &format!("beta/teams/{}/members", enc(team_thread)),
                Some(body),
            )
            .await
        {
            Ok(v) => Ok(error_code(&value(v.clone())).or_else(|| error_code(&v))),
            // 4xx answers are results for this call: their body names the reason.
            Err(Error::Status { status, body, .. }) if status.is_client_error() => {
                let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                Ok(Some(error_code(&v).unwrap_or_else(|| status.to_string())))
            }
            Err(e) => Err(e),
        }
    }
}
fn missing(endpoint: &'static str) -> Error {
    Error::Decode {
        endpoint,
        source: serde::de::Error::custom("missing id"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_bodies_use_the_middle_tier_access_types() {
        let team = NewTeam {
            name: " Design ".into(),
            description: String::new(),
            visibility: TeamVisibility::OrgWide,
        };
        let body = create_body(&team);
        assert_eq!(body["displayName"], "Design");
        assert_eq!(body["accessType"], 3);
        assert_eq!(body["isTenantWide"], true);
    }
    #[test]
    fn gallery_keeps_joinable_teams_only() {
        let page = gallery_page(json!({
            "value": [
                {"groupId":"a","displayName":"A","accessType":"Public","relevantMembers":[{},{}]},
                {"groupId":"b","displayName":"B","accessType":"Private"},
                {"groupId":"c","displayName":"C","accessType":"Public","isDynamic":true},
                {"groupId":"d","displayName":"D","accessType":"Public","joinState":"Joined"}
            ],
            "skipToken": "next"
        }));
        assert_eq!(page.skip_token.as_deref(), Some("next"));
        let ids: Vec<_> = page
            .teams
            .iter()
            .map(|t| (t.group_id.as_str(), t.private, t.known_members))
            .collect();
        assert_eq!(ids, [("a", false, 2), ("b", true, 0)]);
    }
}
