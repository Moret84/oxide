//! Message search (SERP "Messages" tab) on Substrate, with the web client's filters in KQL.
use http::{HeaderMap, HeaderValue, Method};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};

use crate::client::ChatClient;
use crate::error::Error;

const QUERY: &str = "searchservice/api/v2/query";
const PAGE: u32 = 25;
const FROM_FIELD: &str =
    "Extension_SkypeSpaces_ConversationPost_Extension_FromSkypeInternalId_String";
const CHANNEL_THREADS: &str =
    "(Extension_SkypeSpaces_ConversationPost_Extension_ThreadType_String:(topic OR space))";
const FIELDS: [&str; 5] = [
    FROM_FIELD,
    "Extension_SkypeSpaces_ConversationPost_Extension_FileData_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_ThreadType_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_SkypeGroupId_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_SenderTenantId_String",
];
const FLIGHTS: &str = "SearchV2Flight,SubstrateSearchFanoutFlight,lutmsearchmsg,TMSParseRefiningQueries,QueryMicroservice";

/// Where the messages were posted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchIn {
    #[default]
    Everywhere,
    Chats,
    Channels,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    /// Sender MRI.
    pub from: Option<String>,
    /// One chat or channel (`clientthreadid`); a chat also excludes channel threads.
    pub thread: Option<(String, bool)>,
    /// `[start, end)` in UTC ISO 8601, `end` open for "this year".
    pub sent: Option<(String, Option<String>)>,
    pub has_attachment: bool,
    /// The user's AAD object id, for "@mentions me".
    pub mentions: Option<String>,
    pub kind: SearchIn,
    pub hide_bots: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageHit {
    /// `InternetMessageId`: the Teams message id.
    pub message_id: String,
    /// `ClientThreadId`.
    pub thread_id: String,
    /// Root post of a channel reply chain (after `;messageid=`), if any.
    pub reply_chain_id: Option<String>,
    pub preview: String,
    pub sent: String,
    pub sender_name: String,
    pub sender_mri: String,
    pub team_name: String,
    pub channel_name: String,
    pub topic: String,
    pub is_channel: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPage {
    pub hits: Vec<MessageHit>,
    pub total: u64,
    pub more: bool,
    /// Words the service matched, to highlight.
    pub terms: Vec<String>,
}

/// Parts joined by `AND`, wrapped on a top-level `OR` or a leading `NOT ` (not `NOT (`), as the web client's `Z`.
fn and(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let top_or = {
                let mut depth = 0i32;
                let mut found = false;
                for (i, c) in p.char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ if depth == 0 && p[i..].starts_with(" OR ") => found = true,
                        _ => {}
                    }
                }
                found
            };
            if top_or || (p.starts_with("NOT ") && !p.starts_with("NOT (")) {
                format!("({p})")
            } else {
                p.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}
/// KQL of the filters, as `j` builds it, then the user's text.
fn query_string(text: &str, f: &SearchFilters) -> String {
    let mut parts = Vec::new();
    if let Some((start, end)) = &f.sent {
        parts.push(match end {
            Some(end) => format!("sent >= {start} AND sent < {end}"),
            None => format!("sent >= {start}"),
        });
    }
    if f.has_attachment {
        parts.push("hasattachment:yes".into());
    }
    if let Some(oid) = &f.mentions {
        parts.push(format!("mentions:{}", oid.replace('-', "")));
    }
    if let Some(mri) = &f.from {
        parts.push(format!("({FROM_FIELD}:{mri})"));
    }
    if let Some((thread, chat)) = &f.thread {
        parts.push(format!("clientthreadid:{thread}"));
        if *chat {
            parts.push(format!("NOT {CHANNEL_THREADS}"));
        }
    }
    match f.kind {
        SearchIn::Channels => parts.push(CHANNEL_THREADS.into()),
        SearchIn::Chats => parts.push(format!("NOT {CHANNEL_THREADS}")),
        SearchIn::Everywhere => {}
    }
    let text = text.trim();
    parts.push(if text.is_empty() {
        "*".into()
    } else {
        text.to_owned()
    });
    // Soft-deleted messages stay out, as every build that adds the clause does.
    parts.push("NOT (isClientSoftDeleted:TRUE)".into());
    and(&parts)
}
fn body(text: &str, f: &SearchFilters, page: u32, cvid: &str, time_zone: &str) -> Value {
    let mut entity = json!({
        "entityType": "Message",
        "contentSources": ["Teams"],
        "fields": FIELDS,
        "propertySet": "Optimized",
        "query": {"queryString": query_string(text, f), "displayQueryString": text.trim()},
        "from": page * PAGE,
        "size": PAGE,
        "topResultsCount": 5,
    });
    if f.kind == SearchIn::Chats {
        entity["filter"] = json!({"Term": {"ConversationType": "OneToOne"}});
    }
    let mut body = json!({
        "entityRequests": [entity],
        "QueryAlterationOptions": {"EnableAlteration": true, "EnableSuggestion": true,
            "SupportedRecourseDisplayTypes": ["Suggestion"]},
        "cvid": cvid,
        "logicalId": uuid::Uuid::new_v4().to_string(),
        "scenario": {"Dimensions": [
            {"DimensionName": "QueryType", "DimensionValue": "Messages"},
            {"DimensionName": "FormFactor", "DimensionValue": "general.web.reactSearch"}],
            "Name": "powerbar"},
    });
    if *f == SearchFilters::default() {
        body["timezone"] = json!(time_zone);
    }
    body
}
fn text<'a>(v: &'a Value, path: &str) -> &'a str {
    v.pointer(path).and_then(Value::as_str).unwrap_or_default()
}
fn page(v: &Value) -> SearchPage {
    let Some(set) = v.pointer("/EntitySets/0/ResultSets/0") else {
        return SearchPage::default();
    };
    let hits = set
        .get("Results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let s = r.get("Source")?;
            let message_id = text(s, "/InternetMessageId").to_owned();
            let conversation = text(s, "/ClientConversationId");
            let (before, after) = conversation.split_once(';').unwrap_or((conversation, ""));
            let thread_id = Some(text(s, "/ClientThreadId"))
                .filter(|t| !t.is_empty())
                .unwrap_or(before)
                .to_owned();
            if message_id.is_empty() || thread_id.is_empty() {
                return None;
            }
            let ext = |name: &str| {
                text(
                    s,
                    &format!("/Extensions/SkypeSpaces_ConversationPost_Extension_{name}"),
                )
                .to_owned()
            };
            let thread_type = ext("ThreadType").to_ascii_lowercase();
            let is_channel = match thread_type.as_str() {
                "topic" | "space" => true,
                "" => {
                    !(thread_id.starts_with("19:meeting")
                        || thread_id.contains("@unq.gbl.spaces")
                        || thread_id.ends_with("@thread.v2"))
                }
                _ => false,
            };
            let sender = |field: &str| {
                Some(text(s, &format!("/Sender/EmailAddress/{field}")))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| text(s, &format!("/From/EmailAddress/{field}")))
                    .to_owned()
            };
            Some(MessageHit {
                reply_chain_id: after
                    .strip_prefix("messageid=")
                    .filter(|id| !id.is_empty() && is_channel)
                    .map(str::to_owned),
                preview: text(s, "/Preview").trim().to_owned(),
                sent: Some(text(s, "/DateTimeSent"))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| text(s, "/LastDeliveryTime"))
                    .to_owned(),
                sender_name: sender("Name"),
                sender_mri: ext("FromSkypeInternalId"),
                team_name: text(s, "/TeamName").to_owned(),
                channel_name: text(s, "/ChannelName").to_owned(),
                topic: text(s, "/ConversationTopic").to_owned(),
                is_channel,
                message_id,
                thread_id,
            })
        })
        .collect();
    SearchPage {
        hits,
        total: set.get("Total").and_then(Value::as_u64).unwrap_or(0),
        more: set
            .get("MoreResultsAvailable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        terms: set
            .get("SearchTerms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    }
}

impl<A: Authenticator> ChatClient<A> {
    /// One page (25 hits) of messages matching `text` and `filters`.
    pub async fn search_messages(
        &self,
        text: &str,
        filters: &SearchFilters,
        page_number: u32,
        cvid: &str,
        time_zone: &str,
        language: &str,
    ) -> Result<SearchPage, Error> {
        let mut headers = HeaderMap::new();
        let id = uuid::Uuid::new_v4().to_string();
        let mut flights = FLIGHTS.to_owned();
        if filters.hide_bots {
            flights.push_str(",FilterBotMessagesOnTeamsFlight");
        }
        for (name, value) in [
            ("x-client-version", "T2.1".to_owned()),
            ("x-client-flights", flights),
            ("x-client-ui-language", language.to_owned()),
            ("x-client-localtime", chrono::Local::now().to_rfc3339()),
            ("client-request-id", id.clone()),
            ("clientrequestid", id.clone()),
            ("x-cafelatencyheaderenabled", "1".to_owned()),
        ] {
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(name, value);
            }
        }
        let body = body(text, filters, page_number, cvid, time_zone);
        let response = self
            .service_request(
                Service::SubstrateSearch,
                "searchMessages",
                Method::POST,
                QUERY,
                Some(body),
                headers,
            )
            .await?;
        let value: Value =
            serde_json::from_slice(&response.bytes().await?).map_err(|source| Error::Decode {
                endpoint: "searchMessages",
                source,
            })?;
        Ok(page(&value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_become_the_web_client_kql() {
        let f = SearchFilters {
            from: Some("8:orgid:a".into()),
            sent: Some((
                "2026-10-17T22:00:00.000Z".into(),
                Some("2026-10-24T22:00:00.000Z".into()),
            )),
            has_attachment: true,
            ..Default::default()
        };
        assert_eq!(
            query_string("budget", &f),
            format!(
                "sent >= 2026-10-17T22:00:00.000Z AND sent < 2026-10-24T22:00:00.000Z AND hasattachment:yes AND ({FROM_FIELD}:8:orgid:a) AND budget AND NOT (isClientSoftDeleted:TRUE)"
            )
        );
        let chats = SearchFilters {
            kind: SearchIn::Chats,
            mentions: Some("1a2b-3c".into()),
            ..Default::default()
        };
        assert_eq!(
            query_string("a OR b", &chats),
            format!(
                "mentions:1a2b3c AND NOT {CHANNEL_THREADS} AND (a OR b) AND NOT (isClientSoftDeleted:TRUE)"
            )
        );
        let b = body("x", &chats, 2, "cvid", "Europe/Paris");
        assert_eq!(b["entityRequests"][0]["from"], 50);
        assert_eq!(
            b["entityRequests"][0]["filter"]["Term"]["ConversationType"],
            "OneToOne"
        );
        assert!(b.get("timezone").is_none());
        assert_eq!(
            body("x", &SearchFilters::default(), 0, "c", "Europe/Paris")["timezone"],
            "Europe/Paris"
        );
    }
    #[test]
    fn hits_carry_what_opens_them() {
        let v = json!({"EntitySets": [{"ResultSets": [{"Total": 2, "MoreResultsAvailable": true,
            "SearchTerms": ["budget"], "Results": [
            {"Source": {"InternetMessageId": "1700", "ClientThreadId": "19:c@thread.tacv2",
                "ClientConversationId": "19:c@thread.tacv2;messageid=1600", "Preview": "Le budget",
                "DateTimeSent": "2026-10-20T10:00:00Z", "TeamName": "Produit", "ChannelName": "Général",
                "Sender": {"EmailAddress": {"Name": "Alice"}},
                "Extensions": {"SkypeSpaces_ConversationPost_Extension_ThreadType": "topic",
                    "SkypeSpaces_ConversationPost_Extension_FromSkypeInternalId": "8:orgid:a"}}},
            {"Source": {"InternetMessageId": "42", "ClientConversationId": "19:x@unq.gbl.spaces;messageid=42",
                "From": {"EmailAddress": {"Name": "Bob"}}}}
        ]}]}]});
        let p = page(&v);
        assert_eq!((p.total, p.more), (2, true));
        assert_eq!(p.hits[0].reply_chain_id.as_deref(), Some("1600"));
        assert!(p.hits[0].is_channel);
        assert_eq!(p.hits[1].thread_id, "19:x@unq.gbl.spaces");
        assert!(!p.hits[1].is_channel && p.hits[1].reply_chain_id.is_none());
        assert_eq!(p.hits[1].sender_name, "Bob");
    }
}
