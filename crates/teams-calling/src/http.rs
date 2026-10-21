//! Request plumbing over [`teams_auth::net::send`]; mutations never retry on transport errors, as they may have applied.
use std::sync::Arc;

use http::{HeaderMap, Method};
use serde_json::Value;
use teams_auth::{Authenticator, Service, net};
use url::Url;

use crate::Error;

pub(crate) struct Http<A> {
    pub(crate) auth: Arc<A>,
    client: reqwest::Client,
    backoff: net::Backoff,
}
impl<A> Clone for Http<A> {
    fn clone(&self) -> Self {
        Self {
            auth: self.auth.clone(),
            client: self.client.clone(),
            backoff: self.backoff.clone(),
        }
    }
}

teams_auth::from_http_error!(Error);

impl<A: Authenticator> Http<A> {
    pub(crate) fn new(auth: Arc<A>) -> Self {
        Self {
            auth,
            client: net::client(),
            backoff: net::Backoff::default(),
        }
    }

    pub(crate) async fn json(
        &self,
        service: Service,
        method: Method,
        url: Url,
        body: Option<Value>,
        extra: HeaderMap,
    ) -> Result<Value, Error> {
        let request = net::Request::new(service, method, url)
            .headers(extra)
            .json(body.as_ref());
        let reply = net::send(
            &self.client,
            self.auth.as_ref(),
            &self.backoff,
            request,
            |_| {},
        )
        .await?;
        if reply.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&reply.body).map_err(|_| Error::Decode)
    }
}
