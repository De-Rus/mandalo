use mandalo_core::request::Auth;
use mandalo_core::stream::{
    self, Direction, Outgoing, Payload, StreamEvent, StreamKind, StreamLimits, StreamRegistry,
    StreamSpec,
};
use mandalo_core::{AllowAll, StrictPolicy};
use mandalo_testkit::{MockApi, CLOSE_CODE, CLOSE_REASON, TOKEN};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

fn ws_url(mock: &MockApi, path: &str) -> String {
    format!("ws://{}{}", mock.addr(), path)
}

fn spec(url: &str) -> StreamSpec {
    let mut spec = StreamSpec::new(StreamKind::WebSocket, url);
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 5_000,
        backoff_base_ms: 20,
        backoff_max_ms: 40,
        max_reconnect_attempts: 2,
        ..StreamLimits::default()
    };
    spec
}

async fn next_event(events: &mut Receiver<StreamEvent>) -> StreamEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("the stream produced no event in time")
        .expect("the stream ended without an event")
}

async fn wait_for<F>(events: &mut Receiver<StreamEvent>, mut matches: F) -> StreamEvent
where
    F: FnMut(&StreamEvent) -> bool,
{
    loop {
        let event = next_event(events).await;
        if matches(&event) {
            return event;
        }
        if event.is_terminal() {
            panic!("the stream ended before the expected event: {event:?}");
        }
    }
}

async fn drain(events: &mut Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut all = Vec::new();
    loop {
        let event = next_event(events).await;
        let done = event.is_terminal();
        all.push(event);
        if done {
            return all;
        }
    }
}

fn incoming_text(event: &StreamEvent) -> Option<&str> {
    match event {
        StreamEvent::Message {
            direction: Direction::Incoming,
            payload,
            ..
        } => payload.as_text(),
        _ => None,
    }
}

#[tokio::test]
async fn connects_sends_receives_and_closes() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/echo"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    assert!(matches!(
        next_event(&mut events).await,
        StreamEvent::Connecting { .. }
    ));
    assert!(matches!(
        next_event(&mut events).await,
        StreamEvent::Connected { .. }
    ));

    handle.send(Outgoing::text("hola")).await.unwrap();
    let sent = next_event(&mut events).await;
    assert!(matches!(
        sent,
        StreamEvent::Message {
            direction: Direction::Outgoing,
            ..
        }
    ));
    let echoed = wait_for(&mut events, |e| incoming_text(e) == Some("hola")).await;
    assert_eq!(incoming_text(&echoed), Some("hola"));

    handle.close().await.unwrap();
    let closed = wait_for(&mut events, StreamEvent::is_terminal).await;
    match closed {
        StreamEvent::Disconnected { code, reason, .. } => {
            assert_eq!(code, Some(1000));
            assert_eq!(reason, "closed by the client");
        }
        other => panic!("expected a disconnect, got {other:?}"),
    }
    assert!(!handle.is_open());
}

#[tokio::test]
async fn binary_frames_arrive_as_base64_with_their_size() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/binary"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let message = wait_for(&mut events, |e| {
        matches!(
            e,
            StreamEvent::Message {
                payload: Payload::Binary { .. },
                ..
            }
        )
    })
    .await;
    match message {
        StreamEvent::Message { payload, meta, .. } => {
            assert_eq!(payload, Payload::binary(mandalo_testkit::BINARY_FRAME));
            assert_eq!(meta.frame.as_deref(), Some("binary"));
        }
        other => panic!("expected a binary message, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn a_binary_message_can_be_sent_and_comes_back_unchanged() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/echo"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let expected = Payload::binary(&[0x00, 0xff, 0x7f]);
    let Payload::Binary { base64, .. } = expected.clone() else {
        unreachable!()
    };
    handle.send(Outgoing::Binary { base64 }).await.unwrap();

    let message = wait_for(&mut events, |e| {
        matches!(
            e,
            StreamEvent::Message {
                direction: Direction::Incoming,
                payload: Payload::Binary { .. },
                ..
            }
        )
    })
    .await;
    match message {
        StreamEvent::Message { payload, .. } => assert_eq!(payload, expected),
        other => panic!("expected the echo back, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn pings_from_the_server_are_surfaced() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/ping?n=2&ms=5"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let ping = wait_for(&mut events, |e| match e {
        StreamEvent::Message { meta, .. } => meta.frame.as_deref() == Some("ping"),
        _ => false,
    })
    .await;
    match ping {
        StreamEvent::Message { payload, .. } => assert_eq!(payload.as_text(), Some("beat")),
        other => panic!("expected a ping, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn a_server_close_surfaces_its_code_and_reason() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/close"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    match all.last().unwrap() {
        StreamEvent::Disconnected { code, reason, .. } => {
            assert_eq!(*code, Some(CLOSE_CODE));
            assert_eq!(reason, CLOSE_REASON);
        }
        other => panic!("expected a disconnect, got {other:?}"),
    }
}

#[tokio::test]
async fn bearer_auth_reaches_the_handshake_and_a_missing_token_fails_loud() {
    let mock = MockApi::start().await;
    let mut authorized = spec(&ws_url(&mock, "/ws/auth"));
    authorized.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    authorized.vars = [("token".to_string(), TOKEN.to_string())]
        .into_iter()
        .collect();
    let (tx, mut events) = stream::event_channel(&authorized.limits);
    let handle = stream::open(authorized, Arc::new(AllowAll), tx)
        .await
        .unwrap();
    wait_for(&mut events, |e| incoming_text(e) == Some("welcome")).await;
    assert_eq!(
        mock.requests_to("/ws/auth")[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    handle.close().await.unwrap();

    let anonymous = spec(&ws_url(&mock, "/ws/auth"));
    let (tx, mut events) = stream::event_channel(&anonymous.limits);
    let _handle = stream::open(anonymous, Arc::new(AllowAll), tx)
        .await
        .unwrap();
    let all = drain(&mut events).await;
    let failure = all
        .iter()
        .find_map(|e| match e {
            StreamEvent::Error { message, code, .. } => Some((message.clone(), code.clone())),
            _ => None,
        })
        .expect("an unauthorized handshake must produce an error event");
    assert_eq!(failure.1, stream::E_STREAM_AUTH);
    assert!(failure.0.contains("401"), "{}", failure.0);
}

#[tokio::test]
async fn an_api_key_in_the_query_travels_on_the_url() {
    let mock = MockApi::start().await;
    let mut spec = spec(&ws_url(&mock, "/ws/auth"));
    spec.auth = Auth::Apikey {
        key: mandalo_testkit::API_KEY_NAME.to_string(),
        value: mandalo_testkit::API_KEY_VALUE.to_string(),
        placement: "query".to_string(),
    };
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| incoming_text(e) == Some("welcome")).await;
    handle.close().await.unwrap();
}

#[tokio::test]
async fn variables_are_interpolated_into_the_url_and_the_headers() {
    let mock = MockApi::start().await;
    let mut spec = spec("ws://{{host}}/ws/echo");
    spec.headers = vec![("X-Trace".to_string(), "{{trace}}".to_string())];
    spec.vars = [
        ("host".to_string(), mock.addr().to_string()),
        ("trace".to_string(), "abc123".to_string()),
    ]
    .into_iter()
    .collect();
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;
    assert_eq!(
        mock.requests_to("/ws/echo")[0].header("x-trace"),
        Some("abc123")
    );
    handle.close().await.unwrap();
}

#[tokio::test]
async fn a_message_over_the_limit_ends_the_stream_instead_of_growing_memory() {
    let mock = MockApi::start().await;
    let mut spec = spec(&ws_url(&mock, "/ws/big?bytes=200000"));
    spec.limits.max_message_bytes = 4096;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    assert!(
        all.iter().any(|e| matches!(
            e,
            StreamEvent::Error { code, .. } if code == stream::E_STREAM_LIMIT
        )),
        "{all:?}"
    );
    match all.last().unwrap() {
        StreamEvent::Disconnected { code, .. } => assert_eq!(*code, Some(1009)),
        other => panic!("expected a disconnect, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_send_is_refused_and_the_stream_stays_open() {
    let mock = MockApi::start().await;
    let mut spec = spec(&ws_url(&mock, "/ws/echo"));
    spec.limits.max_message_bytes = 16;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    let err = handle
        .send(Outgoing::text("x".repeat(64)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "E_STREAM_LIMIT");
    handle.send(Outgoing::text("small")).await.unwrap();
    wait_for(&mut events, |e| incoming_text(e) == Some("small")).await;
    handle.close().await.unwrap();
}

#[tokio::test]
async fn strict_mode_refuses_a_loopback_socket_that_allow_all_permits() {
    let mock = MockApi::start().await;
    let url = ws_url(&mock, "/ws/echo");

    let (tx, _events) = stream::event_channel(&StreamLimits::default());
    let denied = stream::open(spec(&url), Arc::new(StrictPolicy::new()), tx)
        .await
        .unwrap_err();
    assert_eq!(denied.code(), "E_HOST_DENIED");
    assert!(denied.to_string().contains("loopback"), "{denied}");

    let (tx, mut events) = stream::event_channel(&StreamLimits::default());
    let handle = stream::open(spec(&url), Arc::new(AllowAll), tx)
        .await
        .unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;
    handle.close().await.unwrap();
}

#[tokio::test]
async fn a_dropped_connection_is_retried_and_the_retry_is_visible() {
    let mock = MockApi::start().await;
    let mut spec = spec(&ws_url(&mock, "/ws/flaky"));
    spec.ws.auto_reconnect = true;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let attempts: Vec<(u32, u64)> = all
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Reconnecting {
                attempt, delay_ms, ..
            } => Some((*attempt, *delay_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![(1, 20), (2, 40)]);
    match all.last().unwrap() {
        StreamEvent::Disconnected { reason, .. } => {
            assert!(reason.contains("gave up after 2 attempts"), "{reason}");
        }
        other => panic!("expected a disconnect, got {other:?}"),
    }
}

#[tokio::test]
async fn a_full_buffer_reports_what_it_dropped_instead_of_growing() {
    let mock = MockApi::start().await;
    let mut spec = spec(&ws_url(&mock, "/ws/firehose?n=400"));
    spec.limits.max_buffered_events = 4;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;
    let closing = tokio::spawn(async move { handle.close().await });

    let mut dropped = 0u64;
    let mut seen = 0usize;
    loop {
        let event = next_event(&mut events).await;
        seen += 1;
        if let StreamEvent::Dropped { count, .. } = &event {
            dropped += count;
        }
        if event.is_terminal() {
            break;
        }
    }
    closing.await.unwrap().unwrap();

    assert!(
        dropped > 300,
        "a 400 message firehose into a 4 event buffer must drop most of it, dropped {dropped}"
    );
    assert!(
        seen < 100,
        "the buffer must stay bounded, but {seen} events came through"
    );
}

#[tokio::test]
async fn the_registry_forgets_a_stream_and_leaves_no_task_behind() {
    let mock = MockApi::start().await;
    let registry = StreamRegistry::new();
    let spec = spec(&ws_url(&mock, "/ws/echo"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    let id = registry.insert(handle);

    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;
    assert_eq!(registry.len(), 1);
    assert!(registry.status(&id).unwrap().open);

    registry.send(&id, Outgoing::text("ping")).await.unwrap();
    wait_for(&mut events, |e| incoming_text(e) == Some("ping")).await;

    registry.close(&id).await.unwrap();
    assert_eq!(registry.len(), 0);
    assert!(registry.status(&id).unwrap_err().code() == "E_NOT_FOUND");
    assert!(registry.send(&id, Outgoing::text("gone")).await.is_err());
}

#[tokio::test]
async fn dropping_the_consumer_shuts_the_stream_down() {
    let mock = MockApi::start().await;
    let spec = spec(&ws_url(&mock, "/ws/firehose?n=100000"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    drop(events);
    for _ in 0..100 {
        if !handle.is_open() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a stream with nobody listening must not keep running");
}

#[tokio::test]
async fn nothing_listening_says_so_by_name() {
    let spec = spec("ws://127.0.0.1:1/socket");
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    let all = drain(&mut events).await;
    let message = all
        .iter()
        .find_map(|e| match e {
            StreamEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .expect("a refused connection must produce an error event");
    assert!(
        message.contains("nothing is listening on 127.0.0.1:1"),
        "{message}"
    );
}
