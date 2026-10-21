//! HTTP plumbing shared by every service client: one pool, 429 backoff, retries after a renewed 401, opt-in redirects, errors without URLs.
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use http::{HeaderMap, Method, StatusCode};
use serde_json::Value;
use url::Url;

use teams_i18n::{t, tf};

use crate::{AuthError, Authenticator, Service};

/// Timeout of a request that does not set its own.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);

/// One client per tokio runtime, since pooled connections die with their runtime; never follows redirects, which would leak Teams' secret headers.
pub fn client() -> reqwest::Client {
    static CLIENTS: OnceLock<Mutex<HashMap<Option<tokio::runtime::Id>, reqwest::Client>>> =
        OnceLock::new();
    let runtime = tokio::runtime::Handle::try_current().ok().map(|h| h.id());
    CLIENTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(runtime)
        .or_insert_with(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(DEFAULT_TIMEOUT)
                .build()
                .expect("HTTP client initialization")
        })
        .clone()
}

/// Failure of [`send`]; service crates convert it into their own error type.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// Throttled; no request is sent before this many seconds.
    #[error("{}", tf!("auth.net.too_many_requests_retry_in_s", seconds = .0))]
    RateLimited(u64),
    #[error("HTTP {0}")]
    Http(u16),
    /// Network failure of a read; the message never contains the URL.
    #[error("{}", tf!("auth.net.network_error", detail = .0))]
    Transport(String),
    #[error("{}", t("auth.net.unreadable_response"))]
    Decode,
    /// A mutation the server may already have applied (transport error, 5xx, cut body): never retry blindly.
    #[error("{}", t("auth.net.unknown_outcome"))]
    Uncertain,
}

/// Seconds requested by a `Retry-After` header, as delta-seconds or HTTP date.
pub fn retry_after(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get("retry-after")?.to_str().ok()?.trim();
    value.parse::<u64>().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| (date.timestamp() - chrono::Utc::now().timestamp()).max(1) as u64)
    })
}

/// Shared "do not call before" deadlines, one per key (service, host…).
pub struct Backoff<K = ()>(Arc<Mutex<HashMap<K, Instant>>>);
impl<K> Clone for Backoff<K> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<K> Default for Backoff<K> {
    fn default() -> Self {
        Self(Arc::default())
    }
}
impl<K: Eq + Hash> Backoff<K> {
    /// Remaining whole seconds (rounded up) if `key` is still throttled.
    pub fn remaining(&self, key: &K) -> Option<u64> {
        let until = *self.0.lock().unwrap().get(key)?;
        let left = until.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        Some(
            left.as_secs()
                .saturating_add(u64::from(left.subsec_nanos() > 0))
                .max(1),
        )
    }
    /// Throttles `key` for `seconds`, never shortening a later deadline.
    pub fn note(&self, key: K, seconds: u64) {
        let now = Instant::now();
        let until = now
            .checked_add(Duration::from_secs(seconds))
            .unwrap_or_else(|| now + Duration::from_secs(86400));
        self.0
            .lock()
            .unwrap()
            .entry(key)
            .and_modify(|old| *old = (*old).max(until))
            .or_insert(until);
    }
}

/// One authorized request for [`send`].
pub struct Request<'a> {
    pub service: Service,
    pub method: Method,
    pub url: Url,
    /// Sent with every attempt, before the credentials are added.
    pub headers: HeaderMap,
    pub json: Option<&'a Value>,
    pub timeout: Option<Duration>,
}
impl<'a> Request<'a> {
    pub fn new(service: Service, method: Method, url: Url) -> Self {
        Self {
            service,
            method,
            url,
            headers: HeaderMap::new(),
            json: None,
            timeout: None,
        }
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
    pub fn json(mut self, body: Option<&'a Value>) -> Self {
        self.json = body;
        self
    }
}

/// Successful (2xx) response.
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// Sends `request` authorized for its service, honouring `backoff`; `prepare` adjusts each attempt's headers after authorization.
pub async fn send<A: Authenticator>(
    client: &reqwest::Client,
    auth: &A,
    backoff: &Backoff,
    request: Request<'_>,
    prepare: impl Fn(&mut HeaderMap) + Send + Sync,
) -> Result<Reply, HttpError> {
    let Request {
        service,
        method,
        url,
        headers: extra,
        json,
        timeout,
    } = request;
    let mutation = method != Method::GET;
    let target = || Ok(url.clone());
    let headers = || extra.clone();
    let prepare = |headers: &mut HeaderMap| {
        prepare(headers);
        Ok(())
    };
    let request = Exchange::new(Signer::Service(service), method, &target)
        .headers(&headers)
        .prepare(&prepare)
        .payload(json.map_or(Payload::Empty, Payload::Json))
        .timeout(timeout)
        .throttle(backoff, (), |headers| {
            retry_after(headers).unwrap_or(60).min(86400)
        });
    let response = exchange(client, auth, request)
        .await
        .map_err(|failure| match failure {
            Failure::Auth(e) => HttpError::Auth(e),
            Failure::RateLimited(seconds) => HttpError::RateLimited(seconds),
            Failure::Transport(_) if mutation => HttpError::Uncertain,
            Failure::Transport(e) => HttpError::Transport(e.without_url().to_string()),
        })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(if mutation && status >= 500 {
            HttpError::Uncertain
        } else {
            HttpError::Http(status)
        });
    }
    let headers = response.headers().clone();
    let body = response.bytes().await.map_err(|_| {
        if mutation {
            HttpError::Uncertain
        } else {
            HttpError::Decode
        }
    })?;
    Ok(Reply {
        status,
        headers,
        body: body.to_vec(),
    })
}

/// Expands to `impl From<HttpError> for $error`, for error enums with the same variants as [`HttpError`].
#[macro_export]
macro_rules! from_http_error {
    ($error:ty) => {
        impl From<$crate::net::HttpError> for $error {
            fn from(e: $crate::net::HttpError) -> Self {
                match e {
                    $crate::net::HttpError::Auth(e) => Self::Auth(e),
                    $crate::net::HttpError::RateLimited(s) => Self::RateLimited(s),
                    $crate::net::HttpError::Http(s) => Self::Http(s),
                    $crate::net::HttpError::Transport(m) => Self::Transport(m),
                    $crate::net::HttpError::Decode => Self::Decode,
                    $crate::net::HttpError::Uncertain => Self::Uncertain,
                }
            }
        }
    };
}

/// Failure of [`exchange`] before any final response; callers convert it into their own error type.
#[derive(Debug)]
pub enum Failure {
    Auth(AuthError),
    /// Throttled; no request is sent before this many seconds.
    RateLimited(u64),
    /// The request could not be sent; the error may still carry the URL.
    Transport(reqwest::Error),
}

/// Credentials of an [`Exchange`] and how a 401 renews them.
#[derive(Debug, Clone, Copy)]
pub enum Signer {
    /// `authorize`; a 401 asks `on_challenge`.
    Service(Service),
    /// `authorize`; a 401 asks `on_unauthorized`, ignoring the challenge.
    Refresh(Service),
    /// `authorize_resource` for the attempt's URL; a 401 asks `on_resource_unauthorized`.
    Resource,
    Anonymous,
}

/// Body of an [`Exchange`], sent again with each attempt.
#[derive(Debug, Clone, Copy)]
pub enum Payload<'a> {
    Empty,
    /// Serialized by reqwest, which adds `content-type: application/json` unless already set.
    Json(&'a Value),
    Bytes(&'a [u8]),
}

/// Throttling of an [`Exchange`]: checked before each attempt, noted on a 429.
struct Throttle<'a, K> {
    backoff: &'a Backoff<K>,
    key: K,
    /// Seconds to wait after a 429, from its headers.
    seconds: fn(&HeaderMap) -> u64,
}

/// Per-attempt hook of an [`Exchange`].
pub type Hook<'a, T> = &'a (dyn Fn() -> T + Send + Sync);
/// Adjusts an attempt's headers after credentials.
pub type Prepare<'a, E> = &'a (dyn Fn(&mut HeaderMap) -> Result<(), E> + Send + Sync);
/// Decides from a response's status and headers whether to retarget and send again.
pub type Follow<'a, E> = &'a (dyn Fn(StatusCode, &HeaderMap) -> Result<bool, E> + Send + Sync);

/// One request for [`exchange`]: every hook runs once per attempt, so retries rebuild headers and URL.
pub struct Exchange<'a, K, E> {
    signer: Signer,
    method: Method,
    url: Hook<'a, Result<Url, E>>,
    headers: Option<Hook<'a, HeaderMap>>,
    prepare: Option<Prepare<'a, E>>,
    payload: Payload<'a>,
    timeout: Option<Duration>,
    throttle: Option<Throttle<'a, K>>,
    renewals: usize,
    redirects: usize,
    follow: Option<Follow<'a, E>>,
}
impl<'a, K, E> Exchange<'a, K, E> {
    /// `url` is resolved after authorization (a refresh may move the account), except for [`Signer::Resource`] which signs it.
    pub fn new(signer: Signer, method: Method, url: Hook<'a, Result<Url, E>>) -> Self {
        Self {
            signer,
            method,
            url,
            headers: None,
            prepare: None,
            payload: Payload::Empty,
            timeout: None,
            throttle: None,
            renewals: 1,
            redirects: 0,
            follow: None,
        }
    }
    /// Headers of each attempt, before credentials.
    pub fn headers(mut self, headers: Hook<'a, HeaderMap>) -> Self {
        self.headers = Some(headers);
        self
    }
    /// Adjusts each attempt's headers after credentials.
    pub fn prepare(mut self, prepare: Prepare<'a, E>) -> Self {
        self.prepare = Some(prepare);
        self
    }
    pub fn payload(mut self, payload: Payload<'a>) -> Self {
        self.payload = payload;
        self
    }
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }
    /// Fails fast while `key` is throttled; a 429 throttles it for `seconds(headers)`.
    pub fn throttle(
        mut self,
        backoff: &'a Backoff<K>,
        key: K,
        seconds: fn(&HeaderMap) -> u64,
    ) -> Self {
        self.throttle = Some(Throttle {
            backoff,
            key,
            seconds,
        });
        self
    }
    /// Retries after a 401 for which the authenticator renewed credentials (1 by default).
    pub fn renewals(mut self, renewals: usize) -> Self {
        self.renewals = renewals;
        self
    }
    /// Up to `max` times, `follow` may retarget the request (`Ok(true)`: send again) from a response's status and headers.
    pub fn redirects(mut self, max: usize, follow: Follow<'a, E>) -> Self {
        self.redirects = max;
        self.follow = Some(follow);
        self
    }
}

/// Sends `request` until a final response: 429s, renewed 401s and followed redirects never reach the caller.
pub async fn exchange<A, K, E>(
    client: &reqwest::Client,
    auth: &A,
    request: Exchange<'_, K, E>,
) -> Result<reqwest::Response, E>
where
    A: Authenticator,
    K: Eq + Hash + Clone,
    E: From<Failure>,
{
    let mut renewed = 0;
    let mut redirected = 0;
    loop {
        if let Some(throttle) = &request.throttle
            && let Some(seconds) = throttle.backoff.remaining(&throttle.key)
        {
            return Err(Failure::RateLimited(seconds).into());
        }
        let mut headers = request.headers.map(|h| h()).unwrap_or_default();
        let url = match request.signer {
            Signer::Service(service) | Signer::Refresh(service) => {
                auth.authorize(service, &mut headers)
                    .await
                    .map_err(Failure::Auth)?;
                (request.url)()?
            }
            Signer::Resource => {
                let url = (request.url)()?;
                auth.authorize_resource(&url, &mut headers)
                    .await
                    .map_err(Failure::Auth)?;
                url
            }
            Signer::Anonymous => (request.url)()?,
        };
        if let Some(prepare) = request.prepare {
            prepare(&mut headers)?;
        }
        let mut builder = client
            .request(request.method.clone(), url.clone())
            .headers(headers);
        match request.payload {
            Payload::Empty => {}
            Payload::Json(body) => builder = builder.json(body),
            Payload::Bytes(body) => builder = builder.body(body.to_vec()),
        }
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        let response = builder.send().await.map_err(Failure::Transport)?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS
            && let Some(throttle) = &request.throttle
        {
            let seconds = (throttle.seconds)(response.headers());
            throttle.backoff.note(throttle.key.clone(), seconds);
            return Err(Failure::RateLimited(seconds).into());
        }
        if status == StatusCode::UNAUTHORIZED && renewed < request.renewals {
            let renew = match request.signer {
                Signer::Service(service) => auth.on_challenge(service, response.headers()).await,
                Signer::Refresh(service) => auth.on_unauthorized(service).await,
                Signer::Resource => auth.on_resource_unauthorized(&url).await,
                Signer::Anonymous => false,
            };
            if renew {
                renewed += 1;
                continue;
            }
        }
        if redirected < request.redirects
            && let Some(follow) = request.follow
            && follow(status, response.headers())?
        {
            redirected += 1;
            continue;
        }
        return Ok(response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn retry_after_accepts_seconds_and_dates() {
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        headers.insert("retry-after", HeaderValue::from_static("90"));
        assert_eq!(retry_after(&headers), Some(90));
        let date = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc2822();
        headers.insert("retry-after", HeaderValue::from_str(&date).unwrap());
        assert!((59..=60).contains(&retry_after(&headers).unwrap()));
        headers.insert("retry-after", HeaderValue::from_static("invalid"));
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn backoff_keeps_the_latest_deadline_per_key() {
        let backoff = Backoff::<&str>::default();
        assert_eq!(backoff.remaining(&"a"), None);
        backoff.note("a", 30);
        backoff.note("a", 5);
        assert!((29..=30).contains(&backoff.remaining(&"a").unwrap()));
        assert_eq!(backoff.remaining(&"b"), None);
        let shared = backoff.clone();
        shared.note("b", 1);
        assert!(backoff.remaining(&"b").is_some());
    }

    struct Renewing;
    impl Authenticator for Renewing {
        fn base_url(&self, service: Service) -> Result<Url, AuthError> {
            Err(AuthError::UnknownService(service))
        }
        async fn authorize(&self, _: Service, headers: &mut HeaderMap) -> Result<(), AuthError> {
            headers.insert("authorization", HeaderValue::from_static("Bearer t"));
            Ok(())
        }
        async fn on_unauthorized(&self, _: Service) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn exchange_renews_follows_and_throttles_within_its_limits() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/denied"))
            .and(header("authorization", "Bearer t"))
            .respond_with(ResponseTemplate::new(401))
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(path("/moved"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/moved"))
            .expect(3)
            .mount(&server)
            .await;
        Mock::given(path("/busy"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
            .expect(1)
            .mount(&server)
            .await;
        let client = client();
        let at = |p: &str| Url::parse(&format!("{}{p}", server.uri())).unwrap();
        let denied = || Ok::<_, Failure>(at("/denied"));
        let request = Exchange::<(), _>::new(Signer::Service(Service::Csa), Method::GET, &denied)
            .renewals(2);
        let response = exchange(&client, &Renewing, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let moved = || Ok::<_, Failure>(at("/moved"));
        let follow = |status: StatusCode, _: &HeaderMap| Ok(status.is_redirection());
        let request = Exchange::<(), _>::new(Signer::Anonymous, Method::GET, &moved)
            .redirects(2, &follow);
        let response = exchange(&client, &Renewing, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);

        let backoff = Backoff::default();
        let busy = || Ok::<_, Failure>(at("/busy"));
        for _ in 0..2 {
            let request = Exchange::new(Signer::Anonymous, Method::GET, &busy)
                .throttle(&backoff, (), |h| retry_after(h).unwrap_or(1));
            let result = exchange(&client, &Renewing, request).await;
            assert!(matches!(result, Err(Failure::RateLimited(s)) if (6..=7).contains(&s)));
        }
    }
}
