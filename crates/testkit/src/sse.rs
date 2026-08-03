use axum::body::Body;
use axum::extract::Query;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::State;
use std::sync::Arc;

pub const RETRY_MS: u64 = 120;

#[derive(Deserialize, Default)]
pub(crate) struct Options {
    n: Option<usize>,
    bytes: Option<usize>,
    ms: Option<u64>,
}

pub(crate) fn routes() -> Router<Arc<State>> {
    Router::new()
        .route("/sse/basic", get(basic))
        .route("/sse/multiline", get(multiline))
        .route("/sse/ids", get(ids))
        .route("/sse/retry", get(retry))
        .route("/sse/big", get(big))
        .route("/sse/forever", get(forever))
        .route("/sse/not-a-stream", get(not_a_stream))
        .route("/sse/denied", get(denied))
}

fn stream(chunks: Vec<String>, gap: Option<Duration>) -> Response {
    let (tx, rx) = mpsc::channel::<Result<String, std::io::Error>>(8);
    tokio::spawn(async move {
        for chunk in chunks {
            if tx.send(Ok(chunk)).await.is_err() {
                return;
            }
            if let Some(gap) = gap {
                tokio::time::sleep(gap).await;
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response()
}

async fn basic(Query(options): Query<Options>) -> Response {
    let chunks = (0..options.n.unwrap_or(3))
        .map(|i| format!("data: message {i}\n\n"))
        .collect();
    stream(chunks, options.ms.map(Duration::from_millis))
}

async fn multiline() -> Response {
    stream(
        vec![
            ": this is a comment, and it must be ignored\n".to_string(),
            "event: greeting\nid: 42\ndata: hello\ndata: world\n\n".to_string(),
            "data: plain\n\n".to_string(),
        ],
        None,
    )
}

/// Resumes from `Last-Event-ID`, which is the only way to tell whether a client
/// really reconnected where it left off instead of starting over.
async fn ids(headers: HeaderMap, Query(options): Query<Options>) -> Response {
    let resumed: usize = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let chunks = (1..=options.n.unwrap_or(2))
        .map(|i| {
            let id = resumed + i;
            format!("id: {id}\ndata: event {id}\n\n")
        })
        .collect();
    stream(chunks, options.ms.map(Duration::from_millis))
}

async fn retry() -> Response {
    stream(vec![format!("retry: {RETRY_MS}\ndata: first\n\n")], None)
}

async fn big(Query(options): Query<Options>) -> Response {
    let payload = "x".repeat(options.bytes.unwrap_or(64 * 1024));
    stream(vec![format!("data: {payload}\n\n")], None)
}

async fn forever(Query(options): Query<Options>) -> Response {
    let (tx, rx) = mpsc::channel::<Result<String, std::io::Error>>(8);
    let gap = options.ms.unwrap_or(0);
    tokio::spawn(async move {
        let mut i = 0usize;
        loop {
            if tx.send(Ok(format!("data: tick {i}\n\n"))).await.is_err() {
                return;
            }
            i += 1;
            if gap > 0 {
                tokio::time::sleep(Duration::from_millis(gap)).await;
            } else {
                tokio::task::yield_now().await;
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response()
}

async fn not_a_stream() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        "{\"hello\":\"world\"}",
    )
        .into_response()
}

async fn denied() -> Response {
    (StatusCode::UNAUTHORIZED, "this stream needs a token").into_response()
}
