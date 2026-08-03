use axum::extract::ws::{CloseFrame, Message, WebSocketUpgrade};
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::borrow::Cow;
use std::time::Duration;

use crate::http::{API_KEY_NAME, API_KEY_VALUE, TOKEN};
use crate::State;
use std::sync::Arc;

pub const CLOSE_CODE: u16 = 4001;
pub const CLOSE_REASON: &str = "that is enough";
pub const BINARY_FRAME: &[u8] = &[0xff, 0x00, 0xfe, 0x10];

#[derive(Deserialize, Default)]
pub(crate) struct Count {
    n: Option<usize>,
    bytes: Option<usize>,
    ms: Option<u64>,
}

pub(crate) fn routes() -> Router<Arc<State>> {
    Router::new()
        .route("/ws/echo", get(echo))
        .route("/ws/close", get(closes))
        .route("/ws/binary", get(binary))
        .route("/ws/ping", get(ping))
        .route("/ws/firehose", get(firehose))
        .route("/ws/big", get(big))
        .route("/ws/auth", get(auth))
        .route("/ws/protocol", get(protocol))
        .route("/ws/flaky", get(flaky))
}

async fn echo(upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            let reply = match message {
                Message::Text(text) => Message::Text(text),
                Message::Binary(bytes) => Message::Binary(bytes),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => return,
            };
            if socket.send(reply).await.is_err() {
                return;
            }
        }
    })
}

async fn closes(upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(|mut socket| async move {
        let _ = socket.send(Message::Text("bye".to_string())).await;
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_CODE,
                reason: Cow::from(CLOSE_REASON),
            })))
            .await;
    })
}

async fn binary(upgrade: WebSocketUpgrade, Query(count): Query<Count>) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        for _ in 0..count.n.unwrap_or(1) {
            if socket
                .send(Message::Binary(BINARY_FRAME.to_vec()))
                .await
                .is_err()
            {
                return;
            }
        }
        while socket.recv().await.is_some() {}
    })
}

async fn ping(upgrade: WebSocketUpgrade, Query(count): Query<Count>) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        for _ in 0..count.n.unwrap_or(3) {
            if socket.send(Message::Ping(b"beat".to_vec())).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(count.ms.unwrap_or(20))).await;
        }
        while socket.recv().await.is_some() {}
    })
}

async fn firehose(upgrade: WebSocketUpgrade, Query(count): Query<Count>) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        for i in 0..count.n.unwrap_or(500) {
            if socket.send(Message::Text(format!("m{i}"))).await.is_err() {
                return;
            }
        }
        while socket.recv().await.is_some() {}
    })
}

async fn big(upgrade: WebSocketUpgrade, Query(count): Query<Count>) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        let payload = "x".repeat(count.bytes.unwrap_or(64 * 1024));
        let _ = socket.send(Message::Text(payload)).await;
        while socket.recv().await.is_some() {}
    })
}

fn authorized(headers: &HeaderMap, query: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {TOKEN}"));
    let api_key = headers
        .get(API_KEY_NAME)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == API_KEY_VALUE);
    let in_query = query
        .split('&')
        .any(|pair| pair == format!("{API_KEY_NAME}={API_KEY_VALUE}"));
    bearer || api_key || in_query
}

async fn auth(upgrade: WebSocketUpgrade, headers: HeaderMap, uri: axum::http::Uri) -> Response {
    if !authorized(&headers, uri.query().unwrap_or_default()) {
        return (StatusCode::UNAUTHORIZED, "this socket needs a token").into_response();
    }
    upgrade.on_upgrade(|mut socket| async move {
        let _ = socket.send(Message::Text("welcome".to_string())).await;
        while let Some(Ok(Message::Text(text))) = socket.recv().await {
            if socket.send(Message::Text(text)).await.is_err() {
                return;
            }
        }
    })
}

async fn protocol(upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .protocols(["v2", "chat"])
        .on_upgrade(|mut socket| async move {
            let _ = socket.send(Message::Text("negotiated".to_string())).await;
            while socket.recv().await.is_some() {}
        })
}

/// Answers once and then drops the connection without a close frame — the shape
/// a client must survive with a reconnect rather than a silent stall.
async fn flaky(upgrade: WebSocketUpgrade, Query(count): Query<Count>) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        for i in 0..count.n.unwrap_or(1) {
            if socket.send(Message::Text(format!("n{i}"))).await.is_err() {
                return;
            }
        }
    })
}
