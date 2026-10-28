use std::sync::{Arc, RwLock};
use std::time::Duration;

use http::{HeaderMap, HeaderValue, Method, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use teams_auth::net::{self, Payload, Signer};
use teams_auth::{Authenticator, Service};
use teams_i18n::t;
use url::Url;

use crate::error::Error;
use crate::path::{enc, query};
use crate::types::{
    Conversation, ConversationsPage, EventBatch, Member, Message, MessagesPage, OutgoingMessage,
};

const CONVERSATIONS: &str = "v1/users/ME/conversations";
const THREADS: &str = "v1/threads";
const ENDPOINTS: &str = "v2/users/ME/endpoints";
const MY_PROPERTIES: &str = "v1/users/ME/properties";
const VIEW: &str = "msnp24Equivalent";

/// Client-side timeout of a long-poll request, as in the Teams client.
const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(90);

/// Optional parameters of [`ChatClient::messages`].
#[derive(Debug, Clone, Default)]
pub struct MessagesQuery {
    /// Fetch replies of a channel post instead of the whole conversation.
    pub reply_chain_id: Option<String>,
    pub page_size: Option<u32>,
    /// Epoch milliseconds.
    pub start_time: Option<u64>,
    /// Cursor of the older page, from [`crate::PageMetadata::older_page_cursor`].
    pub sync_state: Option<String>,
}

/// Typed client for the chat service (`Service::ChatService`).
pub struct ChatClient<A> {
    pub(crate) http: reqwest::Client,
    pub(crate) auth: Arc<A>,
    pub(crate) session_id: String,
    pub(crate) regional_base: Arc<RwLock<Option<Url>>>,
    /// Presence base learnt from a Front Door redirect.
    pub(crate) presence_base: Arc<RwLock<Option<Url>>>,
    /// AMS image cookies (`aadtokenauth`), asked for at the first preview image.
    pub(crate) media_cookies: Arc<tokio::sync::Mutex<Option<Vec<crate::media::MediaCookie>>>>,
    backoffs: net::Backoff<Service>,
}

impl<A> Clone for ChatClient<A> {
    fn clone(&self) -> Self {
        Self {
            http: self.http.clone(),
            auth: self.auth.clone(),
            session_id: self.session_id.clone(),
            regional_base: self.regional_base.clone(),
            presence_base: self.presence_base.clone(),
            media_cookies: self.media_cookies.clone(),
            backoffs: self.backoffs.clone(),
        }
    }
}

impl<A: Authenticator> ChatClient<A> {
    /// Shared account authenticator for other Teams services.
    pub fn authenticator(&self) -> Arc<A> {
        self.auth.clone()
    }

    pub fn new(auth: Arc<A>) -> Self {
        Self {
            http: net::client(),
            auth,
            session_id: uuid::Uuid::new_v4().to_string(),
            regional_base: Arc::new(RwLock::new(None)),
            presence_base: Arc::new(RwLock::new(None)),
            media_cookies: Arc::default(),
            backoffs: net::Backoff::default(),
        }
    }

    /// Sends through [`net::exchange`], throttled per service.
    pub(crate) async fn exchange<'a>(
        &'a self,
        service: Service,
        request: net::Exchange<'a, Service, Error>,
    ) -> Result<reqwest::Response, Error> {
        let request = request.throttle(&self.backoffs, service, |headers| {
            net::retry_after(headers).unwrap_or(30).max(1)
        });
        net::exchange(&self.http, self.auth.as_ref(), request).await
    }
    // ---- Conversations and messages -------------------------------------

    /// Lists the user's conversations, most recent first.
    pub async fn conversations(&self, page_size: u32) -> Result<ConversationsPage, Error> {
        let path = format!("{CONVERSATIONS}?view={VIEW}&pageSize={page_size}");
        self.get("myconversations", path).await
    }

    pub async fn conversation(&self, conv: &str) -> Result<Conversation, Error> {
        let path = format!("{CONVERSATIONS}/{}?view={VIEW}", enc(conv));
        self.get("myconversation", path).await
    }

    pub async fn messages(&self, conv: &str, q: &MessagesQuery) -> Result<MessagesPage, Error> {
        let id = conversation_target(conv, q.reply_chain_id.as_deref());
        let path = format!(
            "{CONVERSATIONS}/{}/messages{}",
            enc(&id),
            query(&[
                (
                    "view",
                    Some(if conv.starts_with("48:") {
                        format!("{VIEW}|supportsMessageProperties")
                    } else {
                        VIEW.into()
                    })
                ),
                ("pageSize", q.page_size.map(|v| v.to_string())),
                ("startTime", q.start_time.map(|v| v.to_string())),
                ("syncState", q.sync_state.clone()),
            ])
        );
        self.get("messages", path).await
    }

    pub async fn message(&self, conv: &str, message_id: &str) -> Result<Message, Error> {
        let path = format!(
            "{CONVERSATIONS}/{}/messages/{}?view={VIEW}|supportsMessageProperties",
            enc(conv),
            enc(message_id)
        );
        self.get("message", path).await
    }

    /// Sends a message; pass `reply_chain_id` to reply to a channel post.
    pub async fn send_message(
        &self,
        conv: &str,
        reply_chain_id: Option<&str>,
        message: &OutgoingMessage,
    ) -> Result<Value, Error> {
        let id = conversation_target(conv, reply_chain_id);
        let path = format!("{CONVERSATIONS}/{}/messages", enc(&id));
        let mut body = serde_json::to_value(message).expect("message serialization");
        body["contenttype"] = json!("Text");
        body["conversationid"] = json!(conv);
        if let Some(reply) = reply_chain_id {
            body["replyChainId"] = json!(reply);
        }
        self.send_json("message", Method::POST, path, &body).await
    }

    pub async fn edit_message(
        &self,
        conv: &str,
        message_id: &str,
        message: &OutgoingMessage,
    ) -> Result<Value, Error> {
        let mut body = serde_json::to_value(message).expect("OutgoingMessage serializes");
        body["id"] = Value::String(message_id.to_owned());
        body["version"] = json!(chrono::Utc::now().to_rfc3339());
        body["contenttype"] = json!("Text");
        let path = format!("{CONVERSATIONS}/{}/messages/{message_id}", enc(conv));
        self.send_json("message", Method::PUT, path, &body).await
    }

    pub async fn delete_message(&self, conv: &str, message_id: &str) -> Result<(), Error> {
        let path = format!(
            "{CONVERSATIONS}/{}/messages/{message_id}?behavior=softDelete",
            enc(conv)
        );
        self.request("message", Method::DELETE, path, None)
            .await
            .map(drop)
    }

    /// Sets a message property (reactions, etc.). The body is sent as-is.
    pub async fn set_message_property(
        &self,
        conv: &str,
        message_id: &str,
        name: &str,
        body: &Value,
        replace: bool,
    ) -> Result<Value, Error> {
        let replace = if replace { "&replace=true" } else { "" };
        let path = format!(
            "{CONVERSATIONS}/{}/messages/{message_id}/properties?name={name}{replace}",
            enc(conv),
            message_id = enc(message_id),
            name = enc(name)
        );
        self.send_json("messageproperty", Method::PUT, path, body)
            .await
    }

    pub async fn add_reaction(
        &self,
        conv: &str,
        message_id: &str,
        key: &str,
        timestamp_ms: i64,
    ) -> Result<(), Error> {
        self.set_message_property(
            conv,
            message_id,
            "emotions",
            &json!({"emotions":{"key":key,"value":timestamp_ms}}),
            true,
        )
        .await
        .map(drop)
    }
    pub async fn remove_reaction(
        &self,
        conv: &str,
        message_id: &str,
        key: &str,
    ) -> Result<(), Error> {
        let path = format!(
            "{CONVERSATIONS}/{}/messages/{}/properties?name=emotions",
            enc(conv),
            enc(message_id)
        );
        self.request(
            "messageproperty",
            Method::DELETE,
            path,
            Some(serde_json::to_vec(&json!({"emotions":{"key":key}})).expect("reaction JSON")),
        )
        .await
        .map(drop)
    }

    /// `favorite` is Teams' shown/hidden flag, independent of notification following.
    pub async fn set_conversation_shown(&self, conv: &str, shown: bool) -> Result<(), Error> {
        let path = format!(
            "{CONVERSATIONS}/{}/properties?name=favorite{}",
            enc(conv),
            if shown {
                "&requirePermanentJoinedState=true"
            } else {
                ""
            }
        );
        let mut headers = HeaderMap::new();
        if shown {
            headers.insert("x-ms-favorite-mode", HeaderValue::from_static("manual"));
        }
        self.execute(
            "conversationproperty_favorite",
            Method::PUT,
            path,
            Some(
                serde_json::to_vec(&json!({"favorite":shown.to_string()}))
                    .expect("visibility JSON"),
            ),
            headers,
            None,
        )
        .await
        .map(drop)
    }
    /// Sets a conversation property, e.g. `consumptionhorizon` to mark as read.
    pub async fn set_conversation_property(
        &self,
        conv: &str,
        name: &str,
        value: Value,
    ) -> Result<Value, Error> {
        let path = format!(
            "{CONVERSATIONS}/{}/properties?name={}",
            enc(conv),
            enc(name)
        );
        self.send_json(
            "conversationproperty",
            Method::PUT,
            path,
            &json!({ name: value }),
        )
        .await
    }

    // ---- Threads ----------------------------------------------------------

    pub async fn thread(&self, thread_id: &str) -> Result<Conversation, Error> {
        let path = format!("{THREADS}/{}?view={VIEW}", enc(thread_id));
        self.get("thread", path).await
    }

    pub async fn threads(&self, thread_ids: &[&str]) -> Result<Value, Error> {
        let path = format!(
            "{THREADS}?view={VIEW}&threadIds={}",
            enc(&thread_ids.join(","))
        );
        self.get("threads", path).await
    }

    pub async fn all_thread_members(&self, thread_id: &str) -> Result<Vec<Member>, Error> {
        let mut path = format!(
            "{THREADS}/{}/members?view={VIEW}&pageSize=200",
            enc(thread_id)
        );
        let mut seen = std::collections::HashSet::new();
        let mut members = Vec::new();
        loop {
            if !seen.insert(path.clone()) {
                return Err(Error::Auth(teams_auth::AuthError::Failed(
                    t("chat.client.member_paging_repeated").into(),
                )));
            }
            let value: Value = self.get("thread_members", path).await?;
            let page = value
                .get("members")
                .cloned()
                .unwrap_or_else(|| value.clone());
            let page: Vec<Member> =
                serde_json::from_value(page).map_err(|source| Error::Decode {
                    endpoint: "thread_members",
                    source,
                })?;
            for member in page {
                if !members.iter().any(|m: &Member| m.id == member.id) {
                    members.push(member);
                }
            }
            let next = value
                .get("nextLink")
                .or_else(|| value.get("nextlink"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let Some(next) = next else {
                return Ok(members);
            };
            // Teams strips nextLink's host and reuses the regional service: check the roster path, rebase locally.
            let current = self.url(&format!("{THREADS}/{}/members", enc(thread_id)))?;
            let parsed = if next.starts_with("v1/threads/") {
                self.url(next)?
            } else {
                current.join(next)?
            };
            let marker = "/v1/threads/";
            let start = parsed.path().find(marker).ok_or_else(|| {
                Error::Auth(teams_auth::AuthError::Failed(
                    t("chat.client.invalid_member_paging_link").into(),
                ))
            })?;
            let expected = format!("v1/threads/{}/members", enc(thread_id));
            let tail = &parsed.path()[start + 1..];
            if percent_encoding::percent_decode_str(tail).decode_utf8_lossy()
                != percent_encoding::percent_decode_str(&expected).decode_utf8_lossy()
            {
                return Err(Error::Auth(teams_auth::AuthError::Failed(
                    t("chat.client.invalid_member_paging_link").into(),
                )));
            }
            let mut next = self.url(&expected)?;
            next.set_query(parsed.query());
            let base = self.url("")?;
            path = format!(
                "{}{}",
                next.path()
                    .strip_prefix(base.path())
                    .unwrap_or(next.path())
                    .trim_start_matches('/'),
                next.query().map(|q| format!("?{q}")).unwrap_or_default()
            );
        }
    }

    /// The web client's "Add people": `role: Admin`, `shareHistoryTime` -1 (none), 0 (all) or a start epoch ms.
    pub async fn add_chat_members(
        &self,
        thread_id: &str,
        mris: &[String],
        share_history_time: i64,
    ) -> Result<Value, Error> {
        let members: Vec<Value> = mris
            .iter()
            .map(|id| json!({"id": id, "role": "Admin", "shareHistoryTime": share_history_time}))
            .collect();
        let path = format!("{THREADS}/{}/members", enc(thread_id));
        self.send_json("thread_members", Method::PUT, path, &json!({ "members": members }))
            .await
    }

    pub async fn remove_member(&self, thread_id: &str, mri: &str) -> Result<(), Error> {
        let path = format!("{THREADS}/{}/members/{}", enc(thread_id), enc(mri));
        self.request("thread_members", Method::DELETE, path, None)
            .await
            .map(drop)
    }

    /// Sets a thread property, e.g. `topic` to rename a group chat.
    pub async fn set_thread_property(
        &self,
        thread_id: &str,
        name: &str,
        value: Value,
    ) -> Result<Value, Error> {
        let path = format!("{THREADS}/{}/properties?name={}", enc(thread_id), enc(name));
        self.send_json("threadproperty", Method::PUT, path, &json!({ name: value }))
            .await
    }

    pub async fn consumption_horizons(&self, thread_id: &str) -> Result<Value, Error> {
        let path = format!("{THREADS}/{}/consumptionhorizons", enc(thread_id));
        self.get("thread_consumptionhorizons", path).await
    }

    /// Sends a card action to the bot that posted it; answers its InvokeResponse for `adaptiveCard/action`, else null.
    pub async fn invoke_bot(&self, bot_mri: &str, body: &Value) -> Result<Value, Error> {
        let path = format!("v1/agents/{}/invoke", enc(bot_mri));
        let bytes = self
            .request(
                "botsinvoke",
                Method::POST,
                path,
                Some(serde_json::to_vec(body).expect("invoke JSON")),
            )
            .await?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    // ---- User properties --------------------------------------------------

    pub async fn my_properties(&self) -> Result<Value, Error> {
        self.get("myproperties", MY_PROPERTIES.to_owned()).await
    }

    pub async fn user_properties(&self) -> Result<Value, Error> {
        self.my_properties().await
    }

    pub async fn set_user_property(&self, name: &str, value: Value) -> Result<(), Error> {
        let path = format!("{MY_PROPERTIES}?name={}", enc(name));
        self.send_json("myproperty", Method::PUT, path, &json!({ name: value }))
            .await
            .map(drop)
    }

    // ---- Endpoint and long-poll -------------------------------------------

    /// Registers this client as an endpoint and subscribes to push events.
    pub async fn register_endpoint(
        &self,
        endpoint_id: &str,
        features: &str,
        subscriptions: Value,
    ) -> Result<Value, Error> {
        let path = format!("{ENDPOINTS}/{}", enc(endpoint_id));
        let body = json!({
            "endpointFeatures": features,
            "startingTimeSpan": 0,
            "subscriptions": subscriptions,
        });
        self.send_json("endpoint", Method::PUT, path, &body).await
    }

    /// Waits for events on a long-poll URL; like the Teams client, its query travels in `x-ms-query-params`.
    pub async fn poll(&self, long_poll_url: &str) -> Result<EventBatch, Error> {
        let (base, params) = long_poll_url.split_once('?').unwrap_or((long_poll_url, ""));
        let mut headers = HeaderMap::new();
        if !params.is_empty() {
            let value = HeaderValue::from_str(params)
                .map_err(|_| Error::InvalidLongPollUrl(long_poll_url.to_owned()))?;
            headers.insert("x-ms-query-params", value);
        }
        let bytes = self
            .execute(
                "longpoll",
                Method::GET,
                base.to_owned(),
                None,
                headers,
                Some(LONG_POLL_TIMEOUT),
            )
            .await?;
        decode("longpoll", &bytes)
    }

    // ---- Plumbing ---------------------------------------------------------

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        path: String,
    ) -> Result<T, Error> {
        let bytes = self.request(endpoint, Method::GET, path, None).await?;
        decode(endpoint, &bytes)
    }

    /// Sends a JSON body; an empty response decodes as `null`.
    pub(crate) async fn send_json<B: Serialize + ?Sized>(
        &self,
        endpoint: &'static str,
        method: Method,
        path: String,
        body: &B,
    ) -> Result<Value, Error> {
        let body = serde_json::to_vec(body).expect("request body serializes");
        let bytes = self.request(endpoint, method, path, Some(body)).await?;
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            decode(endpoint, &bytes)
        }
    }

    async fn request(
        &self,
        endpoint: &'static str,
        method: Method,
        path: String,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        self.execute(endpoint, method, path, body, HeaderMap::new(), None)
            .await
    }

    /// Sends a request, following regional redirects and retrying twice after a renewed 401.
    async fn execute(
        &self,
        endpoint: &'static str,
        method: Method,
        path: String,
        body: Option<Vec<u8>>,
        extra_headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>, Error> {
        let headers = || {
            let mut headers = extra_headers.clone();
            headers.insert(
                "behavioroverride",
                HeaderValue::from_static("redirectAs404"),
            );
            headers.insert("x-ms-request-priority", HeaderValue::from_static("10"));
            if body.is_some() {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
            }
            headers
        };
        let url = || self.url(&path);
        let follow = |status, headers: &HeaderMap| {
            if status != http::StatusCode::NOT_FOUND || path.starts_with("http") {
                return Ok(false);
            }
            let Some(base) = headers
                .get("location")
                .and_then(|v| v.to_str().ok())
                .and_then(regional_base)
            else {
                return Ok(false);
            };
            *self.regional_base.write().unwrap() = Some(base);
            Ok(true)
        };
        let request = net::Exchange::new(Signer::Service(Service::ChatService), method, &url)
            .headers(&headers)
            .payload(body.as_deref().map_or(Payload::Empty, Payload::Bytes))
            .timeout(timeout)
            .renewals(2)
            .redirects(10, &follow);
        let resp = self.exchange(Service::ChatService, request).await?;
        let status = resp.status();
        if status == http::StatusCode::BAD_GATEWAY
            || status == http::StatusCode::SERVICE_UNAVAILABLE
        {
            *self.regional_base.write().unwrap() = None;
        }
        if status.is_success() {
            self.auth
                .observe_response(Service::ChatService, resp.headers())?;
        }
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(Error::Status {
                endpoint,
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(bytes.to_vec())
    }

    /// Resolves a relative path against the chat service base; absolute URLs (e.g. long-poll) are kept.
    pub(crate) fn url(&self, path: &str) -> Result<Url, Error> {
        if path.starts_with("https://") || path.starts_with("http://") {
            return Ok(Url::parse(path)?);
        }
        let base = self
            .regional_base
            .read()
            .unwrap()
            .clone()
            .map(Ok)
            .unwrap_or_else(|| self.auth.base_url(Service::ChatService))?;
        let base = base.as_str().trim_end_matches('/');
        Ok(Url::parse(&format!("{base}/{path}"))?)
    }
}

/// `{conv};messageid={replyChainId}` addresses the replies of a channel post.
fn conversation_target(conv: &str, reply_chain_id: Option<&str>) -> String {
    match reply_chain_id {
        Some(rc) => format!("{conv};messageid={rc}"),
        None => conv.to_owned(),
    }
}

fn decode<T: DeserializeOwned>(endpoint: &'static str, bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|source| Error::Decode { endpoint, source })
}

/// Only recognized Teams/Skype HTTPS hosts can receive a region retry with credentials.
pub(crate) fn regional_base(location: &str) -> Option<Url> {
    let url = Url::parse(location).ok()?;
    let host = url.host_str()?;
    let trusted = [
        "teams.microsoft.com",
        "teams.live.com",
        "teams.cloud.microsoft",
        "gov.teams.microsoft.us",
        "dod.teams.microsoft.us",
    ]
    .contains(&host)
        || host.ends_with(".teams.microsoft.com")
        || host.ends_with(".skype.com");
    if !trusted
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let path = if url.path().starts_with("/api/chatsvc/") {
        let region = url
            .path()
            .trim_start_matches("/api/chatsvc/")
            .split('/')
            .next()?;
        format!("/api/chatsvc/{region}/")
    } else {
        "/".into()
    };
    let mut base = url;
    base.set_path(&path);
    base.set_query(None);
    base.set_fragment(None);
    Some(base)
}

#[cfg(test)]
mod regional_tests {
    use super::*;
    #[test]
    fn regional_redirects_keep_prefix_but_reject_arbitrary_hosts() {
        assert_eq!(
            regional_base("https://teams.microsoft.com/api/chatsvc/emea/v1/users/ME/messages?x=y")
                .unwrap()
                .as_str(),
            "https://teams.microsoft.com/api/chatsvc/emea/"
        );
        assert_eq!(
            regional_base("https://emea.msg.skype.com/v1/users/ME")
                .unwrap()
                .as_str(),
            "https://emea.msg.skype.com/"
        );
        assert!(regional_base("https://attacker.example/v1/users/ME").is_none());
        assert!(regional_base("http://teams.microsoft.com/api/chatsvc/emea/").is_none());
        assert!(regional_base("https://teams.microsoft.com.attacker.example/").is_none());
    }
}
