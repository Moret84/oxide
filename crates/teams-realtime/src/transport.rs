//! socket.io 0.9 transports: websocket, or `xhr-polling` (long-poll GET receives, POST sends); both carry text frames.
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http::HeaderMap;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

/// Long polls are held by the server for about the heartbeat interval (70 s).
const POLL_TIMEOUT: Duration = Duration::from_secs(90);

pub(crate) enum Link {
    WebSocket(Box<WebSocketStream<MaybeTlsStream<TcpStream>>>),
    Polling(Polling),
}

pub(crate) struct Polling {
    http: reqwest::Client,
    /// Of the packets sent; the long poll itself waits up to `POLL_TIMEOUT`.
    timeout: Duration,
    url: Url,
    headers: HeaderMap,
    frames: mpsc::UnboundedReceiver<Result<String, String>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Polling {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn stamped(url: &Url) -> Url {
    let mut url = url.clone();
    url.query_pairs_mut()
        .append_pair("t", &crate::trouter::now_ms().to_string());
    url
}

impl Polling {
    /// Starts long-polling `url` (`…/socket.io/1/xhr-polling/<sid>?<query>`).
    pub(crate) fn start(
        http: reqwest::Client,
        timeout: Duration,
        url: Url,
        headers: HeaderMap,
    ) -> Self {
        let (tx, frames) = mpsc::unbounded_channel();
        let task = tokio::spawn({
            let (http, url, headers) = (http.clone(), url.clone(), headers.clone());
            async move {
                loop {
                    let result = http
                        .get(stamped(&url))
                        .headers(headers.clone())
                        .timeout(POLL_TIMEOUT)
                        .send()
                        .await;
                    let text = match result {
                        Ok(response) if response.status().is_success() => response
                            .text()
                            .await
                            .map_err(|e| e.without_url().to_string()),
                        Ok(response) => Err(format!("HTTP {}", response.status().as_u16())),
                        Err(e) => Err(e.without_url().to_string()),
                    };
                    let failed = text.is_err();
                    if tx.send(text).is_err() || failed {
                        return;
                    }
                }
            }
        });
        Self {
            http,
            timeout,
            url,
            headers,
            frames,
            task,
        }
    }

    async fn send(&self, text: String) -> Result<(), String> {
        let response = self
            .http
            .post(stamped(&self.url))
            .headers(self.headers.clone())
            .header("content-type", "text/plain;charset=UTF-8")
            .body(text)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP {}", response.status().as_u16()))
        }
    }
}

impl Link {
    pub(crate) fn is_websocket(&self) -> bool {
        matches!(self, Link::WebSocket(_))
    }

    /// Next text received (cancel-safe): `None` once the server closed, `Err` on a transport failure.
    pub(crate) async fn recv(&mut self) -> Option<Result<String, String>> {
        match self {
            Link::WebSocket(socket) => loop {
                match socket.next().await {
                    None | Some(Ok(Frame::Close(_))) => return None,
                    Some(Err(e)) => return Some(Err(e.to_string())),
                    Some(Ok(Frame::Text(text))) => return Some(Ok(text.to_string())),
                    Some(Ok(_)) => continue,
                }
            },
            Link::Polling(polling) => loop {
                match polling.frames.recv().await {
                    None => return None,
                    // An empty long-poll answer carries nothing.
                    Some(Ok(text)) if text.is_empty() => continue,
                    other => return other,
                }
            },
        }
    }

    pub(crate) async fn send(&mut self, text: String) -> Result<(), String> {
        match self {
            Link::WebSocket(socket) => socket
                .send(Frame::text(text))
                .await
                .map_err(|e| e.to_string()),
            Link::Polling(polling) => polling.send(text).await,
        }
    }

    pub(crate) async fn close(&mut self) {
        match self {
            Link::WebSocket(socket) => {
                let _ = WebSocketStream::close(&mut **socket, None).await;
            }
            Link::Polling(polling) => {
                // socket.io 0.9 disconnect packet, then stop polling.
                let _ = polling.send("0::".into()).await;
                polling.task.abort();
            }
        }
    }
}
