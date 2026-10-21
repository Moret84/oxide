//! Requests to the services around the chat service (CSA, middle tier, NSS…) and their decoding.
use crate::{ChatClient, Error};
use http::{HeaderMap, HeaderValue, Method};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::time::Duration;
use teams_auth::net::{self, Payload, Signer};
use teams_auth::{Authenticator, Service};
use url::Url;

impl<A: Authenticator> ChatClient<A> {
    /// A service's JSON answer; a paging cursor sent as `x-ms-continuation` lands in `continuationToken`.
    pub(crate) async fn service_json<T: DeserializeOwned>(
        &self,
        service: Service,
        endpoint: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
        headers: HeaderMap,
    ) -> Result<T, Error> {
        let response = self
            .service_request(service, endpoint, method, path, body, headers)
            .await?;
        let continuation = response
            .headers()
            .get("x-ms-continuation")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let Some(continuation) = continuation else {
            // Decode straight into `T`: a `Value` tree doubles the memory of large payloads (startup).
            return decode_csa_slice(endpoint, &bytes);
        };
        let mut value: Value =
            serde_json::from_slice(&bytes).map_err(|source| Error::Decode { endpoint, source })?;
        if value.is_object() && value.get("continuationToken").is_none_or(|v| v.is_null()) {
            value["continuationToken"] = json!(continuation);
        }
        decode_csa(endpoint, value)
    }
    /// Sends with the headers of the web client's service calls; 2xx only, credentials observed.
    pub(crate) async fn service_request(
        &self,
        service: Service,
        endpoint: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
        extra: HeaderMap,
    ) -> Result<reqwest::Response, Error> {
        let headers = || {
            let mut headers = extra.clone();
            headers.insert(
                "cache-control",
                HeaderValue::from_static("no-store, no-cache"),
            );
            headers.insert(
                "x-ms-request-id",
                HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()).unwrap(),
            );
            headers.insert(
                "x-ms-session-id",
                HeaderValue::from_str(&self.session_id).unwrap(),
            );
            headers.insert("x-ms-request-priority", HeaderValue::from_static("10"));
            if service == Service::ChatService {
                headers.insert(
                    "behavioroverride",
                    HeaderValue::from_static("redirectAs404"),
                );
            }
            if service == Service::Csa {
                headers.insert("x-ms-client-type", HeaderValue::from_static("web"));
                headers.insert(
                    "x-ms-client-version",
                    HeaderValue::from_static(teams_auth::CLIENT_BUILD),
                );
                headers.insert("x-ms-user-type", HeaderValue::from_static("real-user"));
                headers.insert("x-ringoverride", HeaderValue::from_static(""));
            }
            headers
        };
        let url = || {
            let base = self.auth.base_url(service)?;
            Ok(Url::parse(&format!(
                "{}/{}",
                base.as_str().trim_end_matches('/'),
                path.trim_start_matches('/')
            ))?)
        };
        let request = net::Exchange::new(Signer::Service(service), method, &url)
            .headers(&headers)
            .payload(body.as_ref().map_or(Payload::Empty, Payload::Json))
            .timeout(Some(Duration::from_secs(120)))
            .renewals(2);
        let response = self.exchange(service, request).await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(Error::Status {
                endpoint,
                status,
                body: response.text().await?,
            });
        }
        self.auth.observe_response(service, response.headers())?;
        Ok(response)
    }
}

fn decode_csa_slice<T: DeserializeOwned>(endpoint: &'static str, bytes: &[u8]) -> Result<T, Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        // Malformed JSON reads as before; only well-formed but unexpected data has a path.
        if error.inner().is_syntax() || error.inner().is_eof() {
            Error::Decode {
                endpoint,
                source: error.into_inner(),
            }
        } else {
            Error::DecodeField {
                endpoint,
                path: error.path().to_string(),
                source: error.into_inner(),
            }
        }
    })?;
    deserializer
        .end()
        .map_err(|source| Error::Decode { endpoint, source })?;
    Ok(value)
}

/// A middle tier list, bare or under `value`.
pub(crate) fn decode_value_list<T: DeserializeOwned>(endpoint: &'static str, response: Value) -> Result<T, Error> {
    serde_json::from_value(response.get("value").cloned().unwrap_or(response))
        .map_err(|source| Error::Decode { endpoint, source })
}

pub(crate) fn decode_csa<T: DeserializeOwned>(endpoint: &'static str, value: Value) -> Result<T, Error> {
    serde_path_to_error::deserialize(value).map_err(|error| Error::DecodeField {
        endpoint,
        path: error.path().to_string(),
        source: error.into_inner(),
    })
}
