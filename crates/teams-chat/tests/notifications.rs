use std::sync::Arc;

use http::{HeaderName, HeaderValue};
use serde_json::json;
use teams_auth::{Service, StaticAuthenticator};
use teams_chat::{ChannelNotificationSettings, ChatClient, FollowState, NssChoice};
use url::Url;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> ChatClient<StaticAuthenticator> {
    let auth = StaticAuthenticator::new()
        .with_service(
            Service::ChatService,
            Url::parse(&format!("{}/base/", server.uri())).unwrap(),
        )
        .with_service(
            Service::Uis,
            Url::parse(&format!("{}/nss/emea", server.uri())).unwrap(),
        )
        .with_header(
            HeaderName::from_static("x-test-auth"),
            HeaderValue::from_static("ok"),
        );
    ChatClient::new(Arc::new(auth))
}

const TEAM: &str = "19:team@thread.tacv2";
const CHANNEL: &str = "19:chan@thread.tacv2";

#[tokio::test]
async fn channel_settings_round_trip_through_nss() {
    let server = MockServer::start().await;
    let url = "/nss/emea/v1/me/notificationSettings/team/19%3Ateam%40thread.tacv2/channel/19%3Achan%40thread.tacv2";
    Mock::given(method("GET"))
        .and(path(url))
        .and(query_param("threadTenantId", "tid"))
        .and(header("x-test-auth", "ok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "allNewPosts": "Feed", "includeReplies": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(url))
        .and(body_json(json!({
            "allNewPosts": "Off", "includeReplies": false, "channelMentions": "Off"
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let read = client
        .channel_notification_settings(TEAM, CHANNEL, Some("tid"))
        .await
        .unwrap();
    assert_eq!(read.all_new_posts, Some(NssChoice::Feed));
    assert_eq!(read.channel_mentions, None);
    client
        .set_channel_notification_settings(
            TEAM,
            CHANNEL,
            Some("tid"),
            &ChannelNotificationSettings::OFF,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn channel_settings_get_retries_server_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"allNewPosts":"Off"})))
        .mount(&server)
        .await;
    let read = client(&server)
        .channel_notification_settings(TEAM, CHANNEL, None)
        .await
        .unwrap();
    assert_eq!(read.all_new_posts, Some(NssChoice::Off));
}

#[tokio::test]
async fn batch_posts_channel_ids() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/nss/emea/v1/me/notificationSettings/channels/batch"))
        .and(body_json(json!({"channelIds":[CHANNEL],"threadTenantId":"tid"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[
            {"channelId": CHANNEL, "success": true, "settings": {"channelMentions":"Feed"}}
        ]})))
        .mount(&server)
        .await;
    let results = client(&server)
        .batch_channel_notification_settings(&[CHANNEL.to_owned()], Some("tid"))
        .await
        .unwrap();
    assert_eq!(
        results[0].settings.unwrap().channel_mentions,
        Some(NssChoice::Feed)
    );
}

#[tokio::test]
async fn reply_chain_follow_uses_rcm() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(
            "/base/v1/users/8%3Aorgid%3Ame/conversations/19%3Achan%40thread.tacv2/rcmetadata/123",
        ))
        .and(body_json(json!({"f": 3})))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/base/v1/users/8%3Aorgid%3Ame/conversations/19%3Achan%40thread.tacv2/getrcmetadata",
        ))
        .and(body_json(json!({"rootMessageIds": [123]})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([{"rootMessageId":"123","f":1}])),
        )
        .mount(&server)
        .await;
    let client = client(&server);
    client
        .set_reply_chain_follow("8:orgid:me", CHANNEL, "123", false)
        .await
        .unwrap();
    let states = client
        .reply_chain_follow_states("8:orgid:me", CHANNEL, &["123".into()])
        .await
        .unwrap();
    assert_eq!(states, vec![("123".into(), FollowState::ExplicitFollow)]);
}
