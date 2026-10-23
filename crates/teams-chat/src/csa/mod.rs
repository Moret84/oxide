//! CSA startup, chat list, teams, channels and their posts.
use crate::path::enc;
use crate::service::decode_csa;
use crate::{ChatClient, Error};
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};
use teams_i18n::t;

mod model;
pub use model::*;

impl<A: Authenticator> ChatClient<A> {
    /// Personal call history, newest first; `before` is the oldest shown log's `originalArrivalTime`.
    pub async fn call_history(
        &self,
        before: Option<&str>,
        page_size: u32,
    ) -> Result<CallHistoryPage, Error> {
        let mut path = format!("api/v1/calling/users/me/callhistory?pageSize={page_size}");
        if let Some(before) = before {
            path.push_str(&format!("&before={}", enc(before)));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ms-client-caller",
            HeaderValue::from_static("CallHistory"),
        );
        self.service_json(
            Service::Csa,
            "call history",
            Method::GET,
            &path,
            None,
            headers,
        )
        .await
    }

    pub async fn startup(&self) -> Result<Startup, Error> {
        self.service_json(Service::Csa, "startup", Method::GET,
            "api/v3/teams/users/me?isPrefetch=false&enableMembershipSummary=true&supportsAdditionalSystemGeneratedFolders=true&supportsSliceItems=true", None, HeaderMap::new()).await
    }
    /// Adds a chat to or removes it from a conversation folder, such as Favorites.
    /// Returns the new hierarchy version the reply carries, which the next change must send.
    pub async fn update_folder_item(
        &self,
        hierarchy_version: i64,
        folder_id: &str,
        conversation_id: &str,
        add: bool,
    ) -> Result<Option<i64>, Error> {
        let action = if add { "AddItem" } else { "RemoveItem" };
        let body = json!({
            "folderHierarchyVersion": hierarchy_version,
            "supportsAdditionalSystemGeneratedFolders": true,
            "supportsSliceItems": true,
            "actions": [{"action": action, "folderId": folder_id, "itemId": conversation_id}],
        });
        self.service_request(
            Service::Csa,
            "conversationFolders",
            Method::POST,
            "api/v1/teams/users/me/conversationFolders?supportsAdditionalSystemGeneratedFolders=true&supportsSliceItems=true",
            Some(body),
            HeaderMap::new(),
        )
        .await?
        .json::<ConversationFolders>()
        .await
        .map_or(Ok(None), |folders| Ok(folders.folder_hierarchy_version))
    }
    pub async fn chat(&self, id: &str) -> Result<CsaChat, Error> {
        self.service_json(
            Service::Csa,
            "chat",
            Method::GET,
            &format!("api/v1/teams/users/me/chats/{}", enc(id)),
            None,
            HeaderMap::new(),
        )
        .await
    }
    pub async fn chats_page(&self, continuation: Option<&str>) -> Result<ChatsPage, Error> {
        let path = format!(
            "api/v3/teams/users/me/chats?pageSize=100{}",
            continuation
                .map(|c| format!("&continuationToken={}", enc(c)))
                .unwrap_or_default()
        );
        self.service_json(
            Service::Csa,
            "chats",
            Method::GET,
            &path,
            None,
            HeaderMap::new(),
        )
        .await
    }
    pub async fn teams(&self) -> Result<Vec<Team>, Error> {
        let value: Value = self
            .service_json(
                Service::Csa,
                "teams",
                Method::GET,
                "api/v1/teams/users/me/teams?enableMembershipSummary=true",
                None,
                HeaderMap::new(),
            )
            .await?;
        decode_csa(
            "teams",
            value
                .get("teams")
                .or_else(|| value.get("value"))
                .cloned()
                .unwrap_or(value),
        )
    }
    /// Joined channels, then the unjoined ones the web client lists as hidden (best effort).
    pub async fn channels(&self, team: &str) -> Result<Vec<Channel>, Error> {
        self.channels_with_report(team).await.map(|(channels, _)| channels)
    }
    /// [`Self::channels`], and why some may be missing: the unjoined list could not be read,
    /// or channels of either list could not be decoded and were left out.
    pub async fn channels_with_report(
        &self,
        team: &str,
    ) -> Result<(Vec<Channel>, Option<String>), Error> {
        let (mut channels, mut problems) = self.channels_in(team, None).await?;
        match self.channels_in(team, Some("unjoined")).await {
            Ok((unjoined, skipped)) => {
                problems.extend(skipped);
                for mut channel in unjoined {
                    if channels.iter().any(|c| c.id == channel.id) {
                        continue;
                    }
                    channel
                        .extra
                        .entry("isUnjoinedChannel")
                        .or_insert(Value::Bool(true));
                    channels.push(channel);
                }
            }
            Err(e) => problems.push(format!("channelJoinState=unjoined: {e}")),
        }
        let report = (!problems.is_empty()).then(|| problems.join(" · "));
        Ok((channels, report))
    }
    /// One list of channels; a channel that does not decode is left out and reported,
    /// instead of losing the whole list.
    async fn channels_in(
        &self,
        team: &str,
        join_state: Option<&str>,
    ) -> Result<(Vec<Channel>, Vec<String>), Error> {
        let value: Value = self
            .service_json(
                Service::Csa,
                "channels",
                Method::GET,
                &format!(
                    "api/v1/teams/users/me/teams/{}/channels?{}enableMembershipSummary=true",
                    enc(team),
                    join_state
                        .map(|s| format!("channelJoinState={s}&"))
                        .unwrap_or_default()
                ),
                None,
                HeaderMap::new(),
            )
            .await?;
        let list = value
            .get("channels")
            .or_else(|| value.get("value"))
            .cloned()
            .unwrap_or(value);
        let items: Vec<Value> = decode_csa("channels", list)?;
        let mut channels = Vec::new();
        let mut skipped = Vec::new();
        for item in items {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            match decode_csa::<Channel>("channels", item) {
                Ok(channel) => channels.push(channel),
                Err(e) => skipped.push(format!("{id}: {e}")),
            }
        }
        Ok((channels, skipped))
    }
    pub async fn channel_posts(
        &self,
        team: &str,
        channel: &str,
        continuation: Option<&str>,
    ) -> Result<PostsPage, Error> {
        self.posts_version(team, channel, continuation, 2).await
    }
    /// CSA uses V1 for standard channels and V2 for general/private/shared channels.
    pub async fn channel_posts_for(
        &self,
        team: &str,
        channel: &Channel,
        continuation: Option<&str>,
    ) -> Result<PostsPage, Error> {
        let v2 = channel.is_general
            || team == channel.id
            || !channel.channel_type.eq_ignore_ascii_case("standard");
        self.posts_version(team, &channel.id, continuation, if v2 { 2 } else { 1 })
            .await
    }
    async fn posts_version(
        &self,
        team: &str,
        channel: &str,
        continuation: Option<&str>,
        version: u8,
    ) -> Result<PostsPage, Error> {
        self.service_json(
            Service::Csa,
            "posts",
            Method::GET,
            &format!(
                "api/v{version}/teams/{}/channels/{}?pageSize=30{}",
                enc(team),
                enc(channel),
                if version == 2 {
                    "&filterSystemMessage=true"
                } else {
                    ""
                }
            ),
            None,
            continuation_headers(continuation)?,
        )
        .await
    }
    pub async fn channel_replies(
        &self,
        team: &str,
        channel: &str,
        post: &str,
        continuation: Option<&str>,
    ) -> Result<RepliesPage, Error> {
        self.service_json(
            Service::Csa,
            "replies",
            Method::GET,
            &format!(
                "api/v1/teams/{}/channels/{}/posts/{}/replies?pageSize=200",
                enc(team),
                enc(channel),
                enc(post)
            ),
            None,
            continuation_headers(continuation)?,
        )
        .await
    }
}

fn continuation_headers(token: Option<&str>) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        headers.insert(
            "x-ms-continuation",
            HeaderValue::from_str(token).map_err(|_| {
                Error::Auth(teams_auth::AuthError::Failed(
                    t("chat.csa.invalid_paging_cursor").into(),
                ))
            })?,
        );
    }
    Ok(headers)
}
