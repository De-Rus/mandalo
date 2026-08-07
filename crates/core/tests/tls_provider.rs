//! rustls panics — it does not return an error — when more than one crypto
//! provider is linked and none has been installed. This build legitimately
//! carries both: `reqwest` and `tokio-tungstenite` ask for `ring`, while
//! `rumqttc`'s `use-rustls` pulls `tokio-rustls` with its defaults, which turn
//! on `aws-lc-rs`. These tests drive a real TLS client config through every
//! stack that ships, so a dependency bump cannot bring the panic back quietly.

use mandalo_core::stream::{self, StreamEvent, StreamKind, StreamLimits, StreamSpec};
use mandalo_core::AllowAll;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

/// A socket that accepts and says nothing. Enough to make the client build its
/// TLS config and start a handshake, which is where the panic used to happen.
async fn silent_listener() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    port
}

fn spec(kind: StreamKind, url: &str) -> StreamSpec {
    let mut spec = StreamSpec::new(kind, url);
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 5_000,
        backoff_base_ms: 10,
        backoff_max_ms: 20,
        max_reconnect_attempts: 0,
        ..StreamLimits::default()
    };
    spec
}

/// Opening the stream is the assertion: a failed handshake is fine, a panicking
/// one is not. `open` returning at all means the provider was unambiguous.
async fn open_and_settle(spec: StreamSpec) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
    let opened = stream::open(spec, Arc::new(AllowAll), tx).await;
    if let Ok(handle) = opened {
        let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        let _ = handle.close().await;
    }
}

#[tokio::test]
async fn the_mqtt_stack_resolves_a_crypto_provider() {
    let port = silent_listener().await;
    open_and_settle(spec(StreamKind::Mqtt, &format!("mqtts://127.0.0.1:{port}"))).await;
}

#[tokio::test]
async fn the_websocket_stack_resolves_a_crypto_provider() {
    let port = silent_listener().await;
    open_and_settle(spec(
        StreamKind::WebSocket,
        &format!("wss://127.0.0.1:{port}/socket"),
    ))
    .await;
}

#[tokio::test]
async fn the_http_stack_resolves_a_crypto_provider() {
    let port = silent_listener().await;
    let spec = mandalo_core::request::RequestSpec {
        method: "GET".to_string(),
        url: format!("https://127.0.0.1:{port}/"),
        ..Default::default()
    };
    // A transport error is the expected outcome; a panic is the regression.
    let _ = mandalo_core::request::send_request(spec, &AllowAll).await;
}

#[test]
fn installing_the_provider_twice_is_harmless() {
    mandalo_core::install_crypto_provider();
    mandalo_core::install_crypto_provider();
}
