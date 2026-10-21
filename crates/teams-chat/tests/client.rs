use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;
use teams_auth::{AuthError, Authenticator, Service, StaticAuthenticator};
use teams_chat::{ChatClient, Error, MessagesQuery, OutgoingMessage, ResourceType};
use url::Url;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CONV: &str = "19:abc@thread.v2";
const CONV_ENC: &str = "19%3Aabc%40thread.v2";

fn client(server: &MockServer) -> ChatClient<StaticAuthenticator> {
    let auth = StaticAuthenticator::new()
        .with_service(
            Service::ChatService,
            Url::parse(&format!("{}/base/", server.uri())).unwrap(),
        )
        .with_header(
            HeaderName::from_static("x-test-auth"),
            HeaderValue::from_static("ok"),
        );
    ChatClient::new(Arc::new(auth))
}

#[tokio::test]
async fn messages_encodes_ids_and_paginates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/base/v1/users/ME/conversations/{CONV_ENC}%3Bmessageid%3D42/messages")))
        .and(query_param("view", "msnp24Equivalent"))
        .and(query_param("pageSize", "20"))
        .and(query_param("syncState", "abc=="))
        .and(header("x-test-auth", "ok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "messages": [{
                "id": "1",
                "messagetype": "RichText/Html",
                "content": "<p>hi</p>",
                "from": "https://x/v1/users/ME/contacts/8:orgid:u1",
                "conversationLink": "https://x/v1/users/ME/conversations/19:abc@thread.v2;messageid=42",
                "unknownField": true
            }],
            "_metadata": { "syncState": "next" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server)
        .messages(
            CONV,
            &MessagesQuery {
                reply_chain_id: Some("42".into()),
                page_size: Some(20),
                sync_state: Some("abc==".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(page.metadata.sync_state.as_deref(), Some("next"));
    let msg = &page.messages[0];
    assert_eq!(msg.sender_mri(), "8:orgid:u1");
    assert_eq!(msg.conversation_id(), Some(CONV));
    assert!(msg.extra.contains_key("unknownField"));
}

#[tokio::test]
async fn send_message_posts_teams_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/base/v1/users/ME/conversations/{CONV_ENC}/messages"
        )))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "clientmessageid": "123",
            "messagetype": "RichText/Html",
            "contenttype": "Text",
            "conversationid": CONV,
            "content": "<p>Bonjour</p>",
            "imdisplayname": "Me",
            "properties": {
                "mentions": "[]", "cards": "[]", "links": "[]", "files": "[]",
                "importance": "", "subject": "", "title": "", "formatVariant": "TEAMS"
            }
        })))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&server)
        .await;

    let msg = OutgoingMessage::html("123", "<p>Bonjour</p>", "Me");
    let resp = client(&server)
        .send_message(CONV, None, &msg)
        .await
        .unwrap();
    assert!(resp.is_null());
}

#[tokio::test]
async fn http_errors_carry_status_and_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
        .mount(&server)
        .await;

    let err = client(&server).conversation(CONV).await.unwrap_err();
    match err {
        Error::Status {
            endpoint,
            status,
            body,
        } => {
            assert_eq!(endpoint, "myconversation");
            assert_eq!(status, 404);
            assert_eq!(body, "nope");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn poll_moves_query_into_header_and_decodes_events() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/poll/abc"))
        .and(header("x-ms-query-params", "ackId=7&foo=bar"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "eventMessages": [
                {
                    "id": 1,
                    "type": "EventMessage",
                    "resourceType": "NewMessage",
                    "resourceLink": "https://x/messages/1",
                    "resource": { "id": "1", "messagetype": "Text", "content": "yo", "from": "https://x/contacts/8:orgid:u2" }
                },
                { "id": 2, "resourceType": "SomethingNew", "resource": {} }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/poll/abc?ackId=7&foo=bar", server.uri());
    let batch = client(&server).poll(&url).await.unwrap();

    assert_eq!(batch.events.len(), 2);
    assert_eq!(
        batch.events[0].message().unwrap().sender_mri(),
        "8:orgid:u2"
    );
    assert_eq!(batch.events[1].resource_type, ResourceType::Other);
}

/// Refreshes once on 401, then succeeds.
struct RefreshingAuth {
    base: Url,
    refreshes: AtomicUsize,
}

impl Authenticator for RefreshingAuth {
    fn base_url(&self, _service: Service) -> Result<Url, AuthError> {
        if self.refreshes.load(Ordering::SeqCst) > 0 {
            Ok(self.base.join("refreshed/").unwrap())
        } else {
            Ok(self.base.clone())
        }
    }

    async fn authorize(&self, _service: Service, headers: &mut HeaderMap) -> Result<(), AuthError> {
        let generation = self.refreshes.load(Ordering::SeqCst).to_string();
        headers.insert(
            "x-test-generation",
            HeaderValue::from_str(&generation).unwrap(),
        );
        Ok(())
    }

    async fn on_unauthorized(&self, _service: Service) -> bool {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        true
    }
}

#[tokio::test]
async fn retries_once_after_refresh_on_401() {
    let server = MockServer::start().await;
    Mock::given(header("x-test-generation", "0"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(header("x-test-generation", "1"))
        .and(path("/refreshed/v1/users/ME/properties"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let auth = Arc::new(RefreshingAuth {
        base: Url::parse(&server.uri()).unwrap(),
        refreshes: AtomicUsize::new(0),
    });
    ChatClient::new(auth.clone()).my_properties().await.unwrap();
    assert_eq!(auth.refreshes.load(Ordering::SeqCst), 1);
}

struct RegistrationAuth {
    base: Url,
    registration: std::sync::Mutex<Option<HeaderValue>>,
}
impl Authenticator for RegistrationAuth {
    fn base_url(&self, _: Service) -> Result<Url, AuthError> {
        Ok(self.base.clone())
    }
    async fn authorize(&self, _: Service, headers: &mut HeaderMap) -> Result<(), AuthError> {
        if let Some(value) = self.registration.lock().unwrap().clone() {
            headers.insert("registrationtoken", value);
        }
        Ok(())
    }
    fn observe_response(&self, _: Service, headers: &HeaderMap) -> Result<(), AuthError> {
        if let Some(value) = headers.get("set-registrationtoken") {
            *self.registration.lock().unwrap() = Some(value.clone());
        }
        Ok(())
    }
}
#[tokio::test]
async fn registration_response_reaches_authenticator_and_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v2/users/ME/endpoints/endpoint"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-registrationtoken", "registrationToken=test")
                .set_body_json(json!({})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("registrationtoken", "registrationToken=test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let auth = Arc::new(RegistrationAuth {
        base: Url::parse(&server.uri()).unwrap(),
        registration: std::sync::Mutex::new(None),
    });
    let client = ChatClient::new(auth);
    client
        .register_endpoint("endpoint", "", json!([]))
        .await
        .unwrap();
    client.my_properties().await.unwrap();
}

#[tokio::test]
async fn redirects_do_not_forward_teams_credentials() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", other.uri()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&other)
        .await;
    let error = client(&server).my_properties().await.unwrap_err();
    assert!(matches!(
        error,
        Error::Status {
            status: http::StatusCode::FOUND,
            ..
        }
    ));
}

#[tokio::test]
async fn roster_pagination_accepts_service_relative_links_and_refuses_foreign_origins() {
    let server = MockServer::start().await;
    let roster = format!("/base/v1/threads/{CONV_ENC}/members");
    Mock::given(path(&roster))
        .and(query_param("state", "next"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"members":["8:orgid:b",{"mri":"8:orgid:c","friendlyName":"Claire"}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(&roster)).and(query_param("pageSize","200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"members":["8:orgid:a","8:orgid:b"],"nextLink":format!("v1/threads/{CONV_ENC}/members?state=next")}))).expect(1).mount(&server).await;
    let members = client(&server).all_thread_members(CONV).await.unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[2].display_name(), Some("Claire"));
    server.reset().await;
    Mock::given(path(roster))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"members":[],"nextLink":"https://attacker.invalid/members"})),
        )
        .mount(&server)
        .await;
    assert!(client(&server).all_thread_members(CONV).await.is_err());
}

#[tokio::test]
async fn reactions_use_replace_and_delete_the_selected_key_with_teams_payload() {
    let server = MockServer::start().await;
    let route = format!("/base/v1/users/ME/conversations/{CONV_ENC}/messages/42/properties");
    Mock::given(method("PUT"))
        .and(path(&route))
        .and(query_param("name", "emotions"))
        .and(query_param("replace", "true"))
        .and(body_json(json!({"emotions":{"key":"heart","value":1234}})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(&route))
        .and(query_param("name", "emotions"))
        .and(body_json(json!({"emotions":{"key":"heart"}})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    client
        .add_reaction(CONV, "42", "heart", 1234)
        .await
        .unwrap();
    client.remove_reaction(CONV, "42", "heart").await.unwrap();
    server.reset().await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    assert!(matches!(
        client.add_reaction(CONV, "42", "heart", 1234).await,
        Err(Error::Status {
            status: http::StatusCode::FORBIDDEN,
            ..
        })
    ));
}

#[tokio::test]
async fn roster_next_links_are_rebased_onto_the_current_service_without_contacting_the_link_host() {
    let server = MockServer::start().await;
    let foreign = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&foreign)
        .await;
    let roster = format!("/base/v1/threads/{CONV_ENC}/members");
    for next in [
        format!("/v1/threads/{CONV_ENC}/members?state=next"),
        format!(
            "{}/other-prefix/v1/threads/{CONV_ENC}/members?state=next",
            foreign.uri()
        ),
    ] {
        server.reset().await;
        Mock::given(path(&roster))
            .and(query_param("pageSize", "200"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"members":["8:orgid:a"],"nextLink":next})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(&roster))
            .and(query_param("state", "next"))
            .and(header("x-test-auth", "ok"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"members":["8:orgid:b"]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            client(&server)
                .all_thread_members(CONV)
                .await
                .unwrap()
                .len(),
            2
        );
    }
    server.reset().await;
    Mock::given(path(&roster))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"members":[],"nextLink":"/v1/threads/other-chat/members?state=next"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client(&server).all_thread_members(CONV).await.is_err());
}

#[tokio::test]
async fn chat_rate_limit_is_shared_between_clones_and_does_not_block_csa() {
    let server = MockServer::start().await;
    Mock::given(path("/base/v1/users/ME/conversations"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "60"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":"team"}])))
        .expect(1)
        .mount(&server)
        .await;
    let base = Url::parse(&format!("{}/base/", server.uri())).unwrap();
    let client = ChatClient::new(Arc::new(
        StaticAuthenticator::new()
            .with_service(Service::ChatService, base.clone())
            .with_service(Service::Csa, base),
    ));
    assert!(matches!(
        client.conversations(100).await,
        Err(Error::RateLimited {
            retry_after_secs: 60
        })
    ));
    assert!(matches!(
        client.clone().conversations(100).await,
        Err(Error::RateLimited {
            retry_after_secs: 1..=60
        })
    ));
    assert_eq!(client.teams().await.unwrap()[0].id, "team");
}

#[tokio::test]
async fn csa_rate_limit_is_shared_and_keeps_chat_reads_available() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v1/teams/users/me/teams"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/v1/users/ME/conversations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"conversations":[]})))
        .expect(1)
        .mount(&server)
        .await;
    let base = Url::parse(&format!("{}/base/", server.uri())).unwrap();
    let client = ChatClient::new(Arc::new(
        StaticAuthenticator::new()
            .with_service(Service::ChatService, base.clone())
            .with_service(Service::Csa, base),
    ));
    assert!(matches!(
        client.teams().await,
        Err(Error::RateLimited {
            retry_after_secs: 120
        })
    ));
    assert!(matches!(
        client.clone().teams().await,
        Err(Error::RateLimited {
            retry_after_secs: 1..=120
        })
    ));
    assert!(
        client
            .conversations(100)
            .await
            .unwrap()
            .conversations
            .is_empty()
    );
}

#[tokio::test]
async fn activity_history_requests_read_properties_and_uses_its_own_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/base/v1/users/ME/conversations/48%3Anotifications/messages"))
        .and(query_param("view","msnp24Equivalent|supportsMessageProperties"))
        .and(query_param("syncState","older"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages":[{
            "id":"notification", "properties":{"isread":"true","activity":{"activityType":"likeInChat"}}
        }]}))).expect(1).mount(&server).await;
    let page = client(&server)
        .messages(
            "48:notifications",
            &MessagesQuery {
                sync_state: Some("older".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.messages[0].properties["isread"], "true");
}

#[tokio::test]
async fn teams_visibility_and_roaming_order_use_the_original_teams_wire_contract() {
    let server = MockServer::start().await;
    let client = client(&server);
    Mock::given(method("GET"))
        .and(path("/base/v1/users/ME/properties"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"teamsOrder":"[{\"teamId\":\"team\",\"order\":0}]"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(client.user_properties().await.unwrap()["teamsOrder"].is_string());
    Mock::given(method("PUT"))
        .and(path(format!(
            "/base/v1/users/ME/conversations/{CONV_ENC}/properties"
        )))
        .and(query_param("name", "favorite"))
        .and(query_param("requirePermanentJoinedState", "true"))
        .and(header("x-ms-favorite-mode", "manual"))
        .and(body_json(json!({"favorite":"true"})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    client.set_conversation_shown(CONV, true).await.unwrap();
    Mock::given(method("PUT"))
        .and(path(format!(
            "/base/v1/users/ME/conversations/{CONV_ENC}/properties"
        )))
        .and(query_param("name", "favorite"))
        .and(body_json(json!({"favorite":"false"})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    client.set_conversation_shown(CONV, false).await.unwrap();
    let order = json!([{"teamId":"team","order":0}]).to_string();
    Mock::given(method("PUT"))
        .and(path("/base/v1/users/ME/properties"))
        .and(query_param("name", "teamsOrder"))
        .and(body_json(json!({"teamsOrder":order})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    client
        .set_user_property("teamsOrder", json!(order))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let hide = requests
        .iter()
        .find(|r| r.body == serde_json::to_vec(&json!({"favorite":"false"})).unwrap())
        .unwrap();
    assert!(
        !hide
            .url
            .query_pairs()
            .any(|(k, _)| k == "requirePermanentJoinedState")
    );
    assert!(!hide.headers.contains_key("x-ms-favorite-mode"));
}

#[tokio::test]
async fn channel_listing_preserves_hidden_flags_from_wrapped_csa_response() {
    let server = MockServer::start().await;
    let auth = StaticAuthenticator::new().with_service(
        Service::Csa,
        Url::parse(&format!("{}/", server.uri())).unwrap(),
    );
    let client = ChatClient::new(Arc::new(auth));
    Mock::given(method("GET")).and(path("/api/v1/teams/users/me/teams/team/channels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"channels":[{"id":"hidden","displayName":"Masqué","isFavorite":false,"isFollowed":true,"channelType":"Private"}]})))
        // Joined, then unjoined (`channelJoinState=unjoined`) listing of the same path.
        .expect(2).mount(&server).await;
    let channels = client.channels("team").await.unwrap();
    assert_eq!(channels[0].extra["isFavorite"], false);
    assert_eq!(channels[0].channel_type, "Private");
}
