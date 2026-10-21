use std::sync::Arc;

use http::{HeaderName, HeaderValue};
use serde_json::json;
use teams_auth::{Service, StaticAuthenticator};
use teams_calling::{Error, VoicemailClient};
use url::Url;
use wiremock::matchers::{body_json, header, method, path, query_param_contains};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> VoicemailClient<StaticAuthenticator> {
    let auth = StaticAuthenticator::new()
        .with_service(
            Service::Outlook,
            Url::parse(&format!("{}/", server.uri())).unwrap(),
        )
        .with_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer outlook"),
        );
    VoicemailClient::new(Arc::new(auth))
}

#[tokio::test]
async fn lists_voicemails_with_duration_transcription_and_caller() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/me/messages"))
        .and(header("authorization", "Bearer outlook"))
        .and(header("accept", "application/json;odata.metadata=none"))
        .and(query_param_contains("$filter", "IPM.Note.Microsoft.Voicemail.UM.CA"))
        .and(query_param_contains("$filter", "parentFolderId ne 'deleteditems'"))
        .and(query_param_contains("$expand", "X-VoiceMessageTranscription"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": [
            {"Id": "old", "ReceivedDateTime": "2026-10-12T08:00:00Z", "IsRead": true,
             "From": {"EmailAddress": {"Name": "Bob", "Address": "bob@example.com"}}},
            {"Id": "new", "ReceivedDateTime": "2026-10-13T08:00:00Z", "IsRead": false,
             "InternetMessageHeaders": [
                {"Name": "X-CallingTelephoneNumber", "Value": "+33100000000"},
                {"Name": "X-VoiceMessageSenderName", "Value": "Alice"},
                {"Name": "CallID", "Value": "call-1"}],
             "SingleValueExtendedProperties": [
                {"PropertyId": "Integer {00020328-0000-0000-c000-000000000046} Id 0x6801", "Value": "42"},
                {"PropertyId": "String {00020386-0000-0000-C000-000000000046} Name X-VoiceMessageTranscription", "Value": "Rappelle-moi"}]}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let list = client(&server).list(30).await.unwrap();
    assert_eq!(list[0].id, "new");
    assert_eq!(list[0].sender_name.as_deref(), Some("Alice"));
    assert_eq!(list[0].sender_address.as_deref(), Some("+33100000000"));
    assert_eq!(list[0].duration_secs, Some(42));
    assert_eq!(list[0].transcription.as_deref(), Some("Rappelle-moi"));
    assert_eq!(list[0].call_id.as_deref(), Some("call-1"));
    assert_eq!(list[1].sender_name.as_deref(), Some("Bob"));
    assert!(list[1].is_read);
}

#[tokio::test]
async fn audio_mark_and_delete_use_escaped_message_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2.0/me/messages/a%2Fb=/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value": [
            {"ContentType": "text/plain", "ContentBytes": "eA=="},
            {"ContentType": "audio/mpeg", "ContentBytes": "SUQz"}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v2.0/me/messages/a%2Fb="))
        .and(body_json(json!({"IsRead": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2.0/me/messages/a%2Fb=/move"))
        .and(body_json(json!({"DestinationId": "DeletedItems"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let (kind, bytes) = client.audio("a/b=").await.unwrap();
    assert_eq!(kind, "audio/mpeg");
    assert_eq!(bytes, b"ID3");
    client.set_read("a/b=", true).await.unwrap();
    client.delete("a/b=").await.unwrap();
}

#[tokio::test]
async fn server_errors_on_mutations_are_reported_as_uncertain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .mount(&server)
        .await;
    let client = client(&server);
    assert!(matches!(client.delete("x").await, Err(Error::Uncertain)));
    assert!(matches!(client.list(5).await, Err(Error::RateLimited(7))));
    // The backoff is honoured without hitting the server again.
    assert!(matches!(client.list(5).await, Err(Error::RateLimited(_))));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn trap_credentials_use_api_version_2_and_the_calling_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trap/tokens"))
        .and(header("api-version", "2"))
        .and(header("authorization", "Bearer outlook"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"expires": 600,
            "tokens": [{"realm": "rtcmedia", "username": "u", "password": "p"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let auth = StaticAuthenticator::new()
        .with_service(
            Service::Trap,
            Url::parse(&format!("{}/trap/tokens", server.uri())).unwrap(),
        )
        .with_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer outlook"),
        );
    let credentials = teams_calling::trap::TrapClient::new(Arc::new(auth))
        .credentials()
        .await
        .unwrap();
    assert_eq!(credentials.username, "u");
    assert_eq!(credentials.expires, 600);
}
