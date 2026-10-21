use serde_json::json;
use std::sync::Arc;
use teams_auth::{Service, StaticAuthenticator};
use teams_chat::{ChatClient, Person};
use url::Url;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
fn client(server: &MockServer) -> ChatClient<StaticAuthenticator> {
    let base = Url::parse(&format!("{}/base", server.uri())).unwrap();
    let auth = StaticAuthenticator::new()
        .with_service(Service::Csa, base.clone())
        .with_service(Service::ChatService, base.clone())
        .with_service(Service::MiddleTier, base);
    ChatClient::new(Arc::new(auth))
}
#[tokio::test]
async fn startup_reads_real_csa_shapes_and_separate_channel_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/base/api/v3/teams/users/me"))
        .and(query_param("supportsSliceItems","true")).and(query_param("enableMembershipSummary","true"))
        .and(header("x-ms-user-type","real-user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "chats":[{"id":"private","members":[{"mri":"8:orgid:alice","friendlyName":"Alice"}],"isRead":false,"isSticky":true,"lastMessage":{"id":"1","messageType":"RichText/Html","originalArrivalTime":"2026-10-14T12:00:00Z","content":"Hello"}}],
            "teams":[{"id":"team","displayName":"Engineering","channels":[{"id":"channel","displayName":"Général","isGeneral":true}]}],"channels":[],"metadata":{"syncToken":"cursor","isPartialData":true}
        }))).expect(1).mount(&server).await;
    let data = client(&server).startup().await.unwrap();
    assert!(data.metadata.is_partial_data);
    assert_eq!(data.metadata.sync_token.as_deref(), Some("cursor"));
    let c = data.chats[0].conversation();
    assert_eq!(c.members[0].id, "8:orgid:alice");
    assert_eq!(c.is_read, Some(false));
    assert_eq!(
        c.last_message.unwrap().originalarrivaltime.as_deref(),
        Some("2026-10-14T12:00:00Z")
    );
    assert_eq!(data.teams[0].channels[0].display_name, "Général");
}
#[tokio::test]
async fn channel_posts_and_replies_encode_ids_and_use_header_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/base/api/v2/teams/team%3A1/channels/channel%3A1"))
        .and(query_param("filterSystemMessage","true")).and(header("x-ms-continuation","opaque=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replyChains":[{"id":"root","messages":[{"id":"root","content":"Post"}]}],"continuationToken":"older"}))).expect(1).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(
            "/base/api/v1/teams/team%3A1/channels/channel%3A1/posts/root/replies",
        ))
        .and(header("x-ms-continuation", "next"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages":[{"id":"reply","content":"Reply"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let posts = c
        .channel_posts("team:1", "channel:1", Some("opaque=="))
        .await
        .unwrap();
    assert_eq!(posts.reply_chains[0].root().unwrap().content, "Post");
    assert_eq!(posts.continuation_token.as_deref(), Some("older"));
    assert_eq!(
        c.channel_replies("team:1", "channel:1", "root", Some("next"))
            .await
            .unwrap()
            .messages[0]
            .id,
        "reply"
    );
}
#[tokio::test]
async fn creating_private_chat_uses_unique_roster_and_location_not_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/base/v1/threads"))
        .and(body_json(json!({"members":[{"id":"8:orgid:me","role":"Admin"},{"id":"8:orgid:alice","role":"Admin"}],"properties":{"threadType":"chat","chatFilesIndexId":2,"fixedRoster":true,"uniquerosterthread":true}})))
        .respond_with(ResponseTemplate::new(201).insert_header("location","https://chat.example/v1/threads/19%3Anew%40thread.v2")).expect(1).mount(&server).await;
    let p = Person {
        mri: "8:orgid:alice".into(),
        ..Default::default()
    };
    assert_eq!(
        client(&server)
            .create_chat("8:orgid:me", &[p.clone(), p], "")
            .await
            .unwrap(),
        "19:new@thread.v2"
    );
}
#[tokio::test]
async fn people_picker_posts_json_string_and_chats_use_query_continuation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/base/beta/users/searchV2"))
        .and(query_param("source", "newChat"))
        .and(body_json(json!("Alice")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"value":[{"mri":"8:orgid:alice","displayName":"Alice"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/base/api/v3/teams/users/me/chats"))
        .and(query_param("continuationToken", "a+b=="))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"chats":[],"hasMoreData":false})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    assert_eq!(c.search_people("Alice").await.unwrap()[0].name(), "Alice");
    assert!(!c.chats_page(Some("a+b==")).await.unwrap().has_more_data);
}

#[tokio::test]
async fn teams_and_channel_types_choose_the_correct_posts_endpoint_and_read_header_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/base/api/v1/teams/users/me/teams"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!([{"id":"team","displayName":"Engineering","isArchived":true}]),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    for (version, id) in [
        (1, "standard"),
        (2, "general"),
        (2, "private"),
        (2, "shared"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/base/api/v{version}/teams/team/channels/{id}"
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ms-continuation", "next-from-header")
                    .set_body_json(json!({"replyChains":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let client = client(&server);
    assert!(client.teams().await.unwrap()[0].is_archived);
    for (id, kind, general) in [
        ("standard", "Standard", false),
        ("general", "Standard", true),
        ("private", "Private", false),
        ("shared", "Shared", false),
    ] {
        let channel = teams_chat::Channel {
            id: id.into(),
            channel_type: kind.into(),
            is_general: general,
            ..Default::default()
        };
        let page = client
            .channel_posts_for("team", &channel, None)
            .await
            .unwrap();
        assert_eq!(page.continuation_token.as_deref(), Some("next-from-header"));
    }
}

#[tokio::test]
async fn nullable_startup_fields_do_not_discard_teams_and_teams_accepts_value_envelope() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v3/teams/users/me")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "chats":[{"id":"chat","members":null,"isSticky":null,"hidden":null}],"teams":[{"id":"team","displayName":"Equipe","channels":null,"isDeleted":null,"isArchived":null}],"channels":null,"users":null,"metadata":null
    }))).expect(1).mount(&server).await;
    Mock::given(path("/base/api/v1/teams/users/me/teams"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"value":[{"id":"team","channels":null}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let startup = client.startup().await.unwrap();
    assert_eq!(startup.teams.len(), 1);
    assert_eq!(startup.chats.len(), 1);
    assert!(!startup.teams[0].is_deleted);
    assert_eq!(client.teams().await.unwrap()[0].id, "team");
}

#[tokio::test]
async fn numeric_csa_channel_types_decode_in_startup_teams_and_channels() {
    let server = MockServer::start().await;
    let channels = json!([
        {"id":"general","channelType":0,"isGeneral":true},
        {"id":"private","channelType":1},
        {"id":"shared","channelType":4},
        {"id":"standalone","channelType":9},
        {"id":"legacy","channelType":"Standard"},
        {"id":"future","channelType":2}
    ]);
    let teams = json!([{"id":"team","displayName":"Team","channels":channels}]);
    Mock::given(path("/base/api/v3/teams/users/me")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"teams":teams,"chats":[{"id":"chat","members":[{"mri":"8:orgid:alice","role":2}]}]}))).expect(1).mount(&server).await;
    Mock::given(path("/base/api/v1/teams/users/me/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(teams))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .and(query_param("channelJoinState", "unjoined"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channels))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let startup = client.startup().await.unwrap();
    assert_eq!(startup.chats[0].members[0].role.as_deref(), Some("Admin"));
    for channels in [
        startup.teams[0].channels.clone(),
        client.teams().await.unwrap()[0].channels.clone(),
        client.channels("team").await.unwrap(),
    ] {
        assert_eq!(
            channels
                .iter()
                .map(|c| c.channel_type.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Standard",
                "Private",
                "Shared",
                "Standalone",
                "Standard",
                "Unknown(2)"
            ]
        );
    }
}

#[tokio::test]
async fn csa_decode_errors_report_the_exact_field_path() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v3/teams/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"teams":[{"id":"team","channels":[{"id":"channel","channelType":{}}]}]}),
        ))
        .mount(&server)
        .await;
    let error = client(&server).startup().await.unwrap_err();
    assert!(
        matches!(&error,teams_chat::Error::DecodeField{path,..}if path=="teams[0].channels[0].channelType"),
        "{error}"
    );
}

#[tokio::test]
async fn posts_and_replies_decode_csa_numeric_versions_dates_and_ids() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v1/teams/team/channels/channel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replyChains":[{"id":42,"containerId":"channel","messages":[{"id":42,"version":18446744073709551615u64,"clientMessageId":100,"from":"8:orgid:alice","messageType":"RichText/Html","originalArrivalTime":0,"content":"<p>Publication réelle</p>","properties":null}]}],"continuationToken":"older"}))).expect(1).mount(&server).await;
    Mock::given(path("/base/api/v1/teams/team/channels/channel/posts/42/replies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messages":[{"id":"43","version":0,"messageType":"RichText/Html","content":"Réponse","originalArrivalTime":"2026-10-15T07:00:00Z"}]}))).expect(1).mount(&server).await;
    let client = client(&server);
    let channel = teams_chat::Channel {
        id: "channel".into(),
        channel_type: "Standard".into(),
        ..Default::default()
    };
    let page = client
        .channel_posts_for("team", &channel, None)
        .await
        .unwrap();
    let root = page.reply_chains[0].root().unwrap();
    assert_eq!(root.id, "42");
    assert_eq!(root.version.as_deref(), Some("18446744073709551615"));
    assert_eq!(root.clientmessageid.as_deref(), Some("100"));
    assert_eq!(
        root.originalarrivaltime.as_deref(),
        Some("1970-01-01T00:00:00.000Z")
    );
    assert_eq!(root.content, "<p>Publication réelle</p>");
    let reply = client
        .channel_replies("team", "channel", "42", None)
        .await
        .unwrap()
        .messages
        .remove(0);
    assert_eq!(reply.version.as_deref(), Some("0"));
    assert_eq!(
        reply.originalarrivaltime.as_deref(),
        Some("2026-10-15T07:00:00Z")
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        !requests[0]
            .url
            .query_pairs()
            .any(|(k, _)| k == "filterSystemMessage")
    );
}

#[tokio::test]
async fn unexpected_posts_envelope_is_an_error_instead_of_an_empty_channel() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unexpected":[]})))
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .channel_posts("team", "channel", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn numeric_chat_subtypes_decode_in_startup_chat_lists_and_single_chat() {
    let server = MockServer::start().await;
    let chats = json!([
        {"id":"regular","chatSubType":0},
        {"id":"meeting","chatSubType":1},
        {"id":"channel","chatSubType":2},
        {"id":"nullable","chatSubType":null},
        {"id":"legacy","chatSubType":"Meeting"},
        {"id":"future","chatSubType":99}
    ]);
    Mock::given(path("/base/api/v3/teams/users/me")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"chats":chats,"teams":[{"id":"team","channels":[{"id":"general","channelType":0}]}]}))).expect(1).mount(&server).await;
    Mock::given(path("/base/api/v3/teams/users/me/chats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"chats":chats})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/chats/regular"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"regular","chatSubType":0})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let startup = client.startup().await.unwrap();
    assert_eq!(startup.teams.len(), 1);
    for chats in [startup.chats, client.chats_page(None).await.unwrap().chats] {
        assert_eq!(
            chats
                .iter()
                .map(|c| c.chat_sub_type.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("None"),
                Some("Meeting"),
                Some("Channel"),
                None,
                Some("Meeting"),
                Some("Unknown(99)")
            ]
        );
        for (index, chat) in chats.iter().enumerate() {
            assert_eq!(
                chat.conversation().meeting,
                Some(matches!(index, 1 | 4))
            );
        }
    }
    assert_eq!(
        client
            .chat("regular")
            .await
            .unwrap()
            .chat_sub_type
            .as_deref(),
        Some("None")
    );
}

#[tokio::test]
async fn null_sender_in_the_195th_chat_does_not_discard_startup_or_single_chat() {
    let server = MockServer::start().await;
    let message = json!({"id":"system","from":null,"messageType":null,"content":null,"properties":null,"version":null,"clientMessageId":null,"sequenceId":null,"contenttype":null,"imdisplayname":null,"composeTime":null,"originalArrivalTime":null,"conversationLink":null,"threadtype":null});
    let mut chats: Vec<_> = (0..195)
        .map(|i| json!({"id":format!("chat-{i}"),"chatSubType":0}))
        .collect();
    chats[194]["lastMessage"] = message.clone();
    Mock::given(path("/base/api/v3/teams/users/me")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"chats":chats,"teams":[{"id":"team","channels":[{"id":"general","channelType":0}]}]}))).expect(1).mount(&server).await;
    Mock::given(path("/base/api/v1/teams/users/me/chats/chat-194"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chats[194].clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v2/teams/team/channels/general"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"replyChains":[{"id":"system","messages":[message]}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(&server);
    let startup = client.startup().await.unwrap();
    assert_eq!(startup.chats.len(), 195);
    assert_eq!(startup.teams.len(), 1);
    let last = startup.chats[194].last_message.as_ref().unwrap();
    assert!(last.from.is_empty());
    assert!(last.content.is_empty());
    assert!(last.properties.is_empty());
    assert_eq!(last.sender_mri(), "");
    assert!(
        client
            .chat("chat-194")
            .await
            .unwrap()
            .last_message
            .unwrap()
            .from
            .is_empty()
    );
    assert!(
        client
            .channel_posts("team", "general", None)
            .await
            .unwrap()
            .reply_chains[0]
            .root()
            .unwrap()
            .from
            .is_empty()
    );
}

#[tokio::test]
async fn external_names_use_federated_profiles_only_for_unresolved_people() {
    for regular_status in [200, 403] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/base/beta/users/fetchShortProfile"))
            .respond_with(
                ResponseTemplate::new(regular_status).set_body_json(json!({"value":[
                    {"mri":"8:orgid:local","displayName":"Alice"},
                    {"mri":"8:orgid:external","displayName":null}
                ]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let unresolved = if regular_status == 200 {
            vec!["8:orgid:external"]
        } else {
            vec!["8:orgid:local", "8:orgid:external"]
        };
        Mock::given(method("POST"))
            .and(path("/base/beta/users/fetchFederated"))
            .and(query_param("edEnabled", "false"))
            .and(body_json(json!(unresolved)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value":[
                {"mri":"8:orgid:external","displayName":"Bruno Externe"}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        let profiles = client(&server)
            .people_profiles(&["8:orgid:local".into(), "8:orgid:external".into()])
            .await
            .unwrap();
        assert_eq!(
            profiles
                .iter()
                .find(|p| p.mri == "8:orgid:external")
                .unwrap()
                .known_name(),
            Some("Bruno Externe")
        );
        if regular_status == 200 {
            assert_eq!(
                profiles
                    .iter()
                    .find(|p| p.mri == "8:orgid:local")
                    .unwrap()
                    .known_name(),
                Some("Alice")
            );
        }
    }
}

#[tokio::test]
async fn current_account_profile_reads_name_and_email_without_exposing_identifiers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/base/beta/users/me/"))
        .and(query_param("skypeTeamsInfo", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"value":{
            "mri":"8:orgid:me", "displayName":"John Doe",
            "email":"", "userPrincipalName":"john.doe@example.com"
        }})))
        .expect(1)
        .mount(&server)
        .await;
    let profile = client(&server).my_profile().await.unwrap();
    assert_eq!(profile.known_name(), Some("John Doe"));
    assert_eq!(profile.email, "john.doe@example.com");
    let profile: Person = serde_json::from_value(json!({
        "givenName":"John", "surname":"Doe", "mail":"john.doe@example.com"
    }))
    .unwrap();
    assert_eq!(profile.display_name, "John Doe");
}
#[tokio::test]
async fn call_history_reads_call_log_messages_and_pages_with_before() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/base/api/v1/calling/users/me/callhistory"))
        .and(query_param("pageSize", "50"))
        .and(query_param("before", "2026-10-14T12:00:00.000Z"))
        .and(header("x-ms-client-caller", "CallHistory"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-ms-continuation", "next")
                .set_body_json(json!({
                    "messages": [{"id": "1", "messageType": "Text",
                        "originalArrivalTime": "2026-10-13T09:00:00.000Z",
                        "properties": {"call-log": "{\"callId\":\"c1\",\"callDirection\":\"incoming\",\"callState\":\"missed\"}"}}],
                    "_metadata": {"syncState": "s"}
                })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let page = client(&server)
        .call_history(Some("2026-10-14T12:00:00.000Z"), 50)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(page.messages[0].properties["call-log"].is_string());
    assert_eq!(page.continuation_token.as_deref(), Some("next"));
    assert_eq!(page.metadata["syncState"], "s");
}

#[tokio::test]
async fn unjoined_channels_are_added_and_marked() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .and(query_param("channelJoinState", "unjoined"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"id":"joined","displayName":"Joined"},{"id":"other","displayName":"Other"}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([{"id":"joined","displayName":"Joined"}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let channels = client(&server).channels("team").await.unwrap();
    let ids: Vec<_> = channels.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["joined", "other"]);
    assert!(channels[0].extra.get("isUnjoinedChannel").is_none());
    assert_eq!(channels[1].extra["isUnjoinedChannel"], true);
}

#[tokio::test]
async fn an_unreadable_channel_is_reported_not_the_whole_list_lost() {
    let server = MockServer::start().await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .and(query_param("channelJoinState", "unjoined"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(path("/base/api/v1/teams/users/me/teams/team/channels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":"good","displayName":"Good"},
            {"id":"bad","displayName":{"unexpected":"object"}}
        ])))
        .mount(&server)
        .await;
    let (channels, report) = client(&server).channels_with_report("team").await.unwrap();
    assert_eq!(channels.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["good"]);
    let report = report.unwrap();
    assert!(report.contains("bad:"), "{report}");
    assert!(report.contains("unjoined"), "{report}");
}
