use http::{HeaderName, HeaderValue};
use std::sync::Arc;
use teams_auth::{Service, StaticAuthenticator};
use teams_chat::ChatClient;
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> ChatClient<StaticAuthenticator> {
    ChatClient::new(Arc::new(
        StaticAuthenticator::new()
            .with_service(
                Service::Ams,
                Url::parse(&format!("{}/media/", server.uri())).unwrap(),
            )
            .with_header(
                HeaderName::from_static("authorization"),
                HeaderValue::from_static("Bearer private"),
            ),
    ))
}
const SOURCE: &str = "https://eu-api.asm.skype.com/v1/objects/image/views/imgo?foo=bar";
#[tokio::test]
async fn ams_is_rewritten_and_headers_are_removed_on_non_ams_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/v1/objects/image/views/imgo"))
        .and(header("authorization", "Bearer private"))
        .and(query_param("v", "1"))
        .and(query_param("foo", "bar"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/public/image.png"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/public/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image bytes"))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        client(&server).download_image(SOURCE).await.unwrap(),
        b"image bytes"
    );
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[1].headers.contains_key("authorization"));
    assert!(!requests[1].headers.contains_key("x-skypetoken"));
}
#[tokio::test]
async fn card_images_through_the_url_preview_proxy_are_authenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/urlp/v1/url/content"))
        .and(header("authorization", "Bearer private"))
        .and(query_param("url", "https://teams.github.com/pr-icon.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"icon"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/media/v1/objects/image/views/imgo"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            "/urlp/v1/url/content?url=https%3a%2f%2fteams.github.com%2fpr-icon.png",
        ))
        .mount(&server)
        .await;
    assert_eq!(client(&server).download_image(SOURCE).await.unwrap(), b"icon");
}
#[tokio::test]
async fn insecure_external_redirects_and_oversized_images_are_rejected() {
    let server = MockServer::start().await;
    let external = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&external)
        .await;
    Mock::given(path("/media/v1/objects/image/views/imgo"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/steal", external.uri())),
        )
        .mount(&server)
        .await;
    assert!(client(&server).download_image(SOURCE).await.is_err());
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 8 * 1024 * 1024 + 1]))
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .download_image(SOURCE)
            .await
            .unwrap_err()
            .to_string()
            .contains("volumineuse")
    );
}

#[tokio::test]
async fn profile_pictures_use_authenticated_regional_service_and_bound_downloads() {
    let server = MockServer::start().await;
    let auth = StaticAuthenticator::new()
        .with_service(
            Service::ProfileImages,
            Url::parse(&format!("{}/images/", server.uri())).unwrap(),
        )
        .with_header(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer private"),
        );
    let client = ChatClient::new(Arc::new(auth));
    Mock::given(method("GET"))
        .and(path(
            "/images/beta/users/8%3Aorgid%3Aalice/profilepicturev2",
        ))
        .and(header("authorization", "Bearer private"))
        .and(query_param("size", "HR96x96"))
        .and(query_param("displayname", "Alice Martin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        client
            .profile_picture("8:orgid:alice", "Alice Martin")
            .await
            .unwrap(),
        vec![1, 2, 3]
    );
    Mock::given(method("GET"))
        .and(path("/images/beta/users/large/profilepicturev2"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 2 * 1024 * 1024 + 1]))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client.profile_picture("large", "").await.is_err());
    Mock::given(method("GET"))
        .and(path("/images/beta/users/missing/profilepicturev2"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client.profile_picture("missing", "").await.is_err());
}
