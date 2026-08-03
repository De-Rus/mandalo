use mandalo_core::request::Auth;
use mandalo_core::stream::{
    self, Direction, Outgoing, StreamEvent, StreamKind, StreamLimits, StreamSpec,
};
use mandalo_core::{AllowAll, StrictPolicy};
use mandalo_testkit::{MockApi, RETRY_MS};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

fn spec(url: &str) -> StreamSpec {
    let mut spec = StreamSpec::new(StreamKind::Sse, url);
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 5_000,
        backoff_base_ms: 20,
        backoff_max_ms: 40,
        max_reconnect_attempts: 2,
        ..StreamLimits::default()
    };
    spec.sse.auto_reconnect = false;
    spec
}

async fn next_event(events: &mut Receiver<StreamEvent>) -> StreamEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("the stream produced no event in time")
        .expect("the stream ended without an event")
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

fn messages(all: &[StreamEvent]) -> Vec<(Option<String>, String, Option<String>)> {
    all.iter()
        .filter_map(|e| match e {
            StreamEvent::Message {
                direction: Direction::Incoming,
                payload,
                meta,
                ..
            } => Some((
                meta.event.clone(),
                payload.as_text().unwrap_or_default().to_string(),
                meta.id.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn errors(all: &[StreamEvent]) -> Vec<(String, String)> {
    all.iter()
        .filter_map(|e| match e {
            StreamEvent::Error { message, code, .. } => Some((message.clone(), code.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn reads_events_until_the_server_is_done() {
    let mock = MockApi::start().await;
    let spec = spec(&mock.url("/sse/basic?n=3"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    assert!(matches!(all[0], StreamEvent::Connecting { .. }));
    assert!(matches!(all[1], StreamEvent::Connected { .. }));
    let data: Vec<String> = messages(&all).into_iter().map(|m| m.1).collect();
    assert_eq!(data, vec!["message 0", "message 1", "message 2"]);
    assert!(matches!(all.last(), Some(StreamEvent::Disconnected { .. })));
}

#[tokio::test]
async fn multi_line_data_ids_and_comments_are_parsed_off_the_wire() {
    let mock = MockApi::start().await;
    let spec = spec(&mock.url("/sse/multiline"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    assert_eq!(
        messages(&all),
        vec![
            (
                Some("greeting".to_string()),
                "hello\nworld".to_string(),
                Some("42".to_string())
            ),
            (None, "plain".to_string(), Some("42".to_string())),
        ]
    );
}

#[tokio::test]
async fn a_reconnect_is_visible_and_resumes_from_the_last_event_id() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/ids?n=2"));
    spec.sse.auto_reconnect = true;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let ids: Vec<String> = messages(&all).into_iter().filter_map(|m| m.2).collect();
    assert_eq!(ids, vec!["1", "2", "3", "4", "5", "6"]);

    let attempts: Vec<u32> = all
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Reconnecting { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![1, 2]);

    let resumed: Vec<Option<String>> = mock
        .requests_to("/sse/ids")
        .iter()
        .map(|r| r.header("last-event-id").map(str::to_string))
        .collect();
    assert_eq!(
        resumed,
        vec![None, Some("2".to_string()), Some("4".to_string())]
    );
}

#[tokio::test]
async fn the_servers_retry_interval_sets_the_reconnect_delay() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/retry"));
    spec.sse.auto_reconnect = true;
    spec.limits.max_reconnect_attempts = 1;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let delays: Vec<u64> = all
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Reconnecting { delay_ms, .. } => Some(*delay_ms),
            _ => None,
        })
        .collect();
    assert_eq!(delays, vec![RETRY_MS]);
}

#[tokio::test]
async fn a_last_event_id_given_up_front_is_sent_on_the_first_request() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/ids?n=1"));
    spec.sse.last_event_id = Some("100".to_string());
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let ids: Vec<String> = messages(&all).into_iter().filter_map(|m| m.2).collect();
    assert_eq!(ids, vec!["101"]);
}

#[tokio::test]
async fn an_endpoint_that_is_not_an_event_stream_fails_loud() {
    let mock = MockApi::start().await;
    let spec = spec(&mock.url("/sse/not-a-stream"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let (message, code) = errors(&all).pop().expect("a loud failure");
    assert_eq!(code, stream::E_STREAM_PROTOCOL);
    assert!(message.contains("not text/event-stream"), "{message}");
}

#[tokio::test]
async fn a_rejected_stream_is_reported_as_auth_and_never_retried() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/denied"));
    spec.sse.auto_reconnect = true;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let (message, code) = errors(&all).pop().expect("a loud failure");
    assert_eq!(code, stream::E_STREAM_AUTH);
    assert!(message.contains("401"), "{message}");
    assert!(
        !all.iter()
            .any(|e| matches!(e, StreamEvent::Reconnecting { .. })),
        "a rejection must not be retried"
    );
}

#[tokio::test]
async fn auth_and_headers_reach_the_request() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/basic?n=1"));
    spec.headers = vec![("X-Trace".to_string(), "{{trace}}".to_string())];
    spec.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    spec.vars = [
        ("trace".to_string(), "abc".to_string()),
        ("token".to_string(), "s3cr3t".to_string()),
    ]
    .into_iter()
    .collect();
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    drain(&mut events).await;

    let request = &mock.requests_to("/sse/basic")[0];
    assert_eq!(request.header("x-trace"), Some("abc"));
    assert_eq!(request.header("authorization"), Some("Bearer s3cr3t"));
    assert_eq!(request.header("accept"), Some("text/event-stream"));
}

#[tokio::test]
async fn nothing_can_be_sent_on_an_event_stream() {
    let mock = MockApi::start().await;
    let spec = spec(&mock.url("/sse/forever?ms=50"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    next_event(&mut events).await;
    next_event(&mut events).await;

    let err = handle.send(Outgoing::text("nope")).await.unwrap_err();
    assert!(
        err.to_string().contains("from the server to the client"),
        "{err}"
    );
    handle.close().await.unwrap();
}

#[tokio::test]
async fn an_event_over_the_limit_ends_the_stream() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/big?bytes=100000"));
    spec.limits.max_message_bytes = 2048;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let (_, code) = errors(&all).pop().expect("a loud failure");
    assert_eq!(code, stream::E_STREAM_LIMIT);
}

#[tokio::test]
async fn strict_mode_refuses_a_loopback_event_stream() {
    let mock = MockApi::start().await;
    let (tx, _events) = stream::event_channel(&StreamLimits::default());
    let denied = stream::open(
        spec(&mock.url("/sse/basic")),
        Arc::new(StrictPolicy::new()),
        tx,
    )
    .await
    .unwrap_err();
    assert_eq!(denied.code(), "E_HOST_DENIED");
}

#[tokio::test]
async fn a_full_buffer_reports_what_it_dropped() {
    let mock = MockApi::start().await;
    let mut spec = spec(&mock.url("/sse/forever"));
    spec.limits.max_buffered_events = 4;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;
    let closing = tokio::spawn(async move { handle.close().await });

    let mut dropped = 0u64;
    loop {
        let event = next_event(&mut events).await;
        if let StreamEvent::Dropped { count, .. } = &event {
            dropped += count;
        }
        if event.is_terminal() {
            break;
        }
    }
    closing.await.unwrap().unwrap();
    assert!(dropped > 0, "a firehose into a 4 event buffer must drop");
}
