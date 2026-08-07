use mandalo_core::request::Auth;
use mandalo_core::stream::{
    self, Direction, Outgoing, Payload, StreamEvent, StreamKind, StreamLimits, StreamSpec,
    Subscription,
};
use mandalo_core::{AllowAll, StrictPolicy};
use mandalo_testkit::{MqttBroker, MQTT_PASSWORD, MQTT_USER};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

fn spec(url: &str, topics: &[(&str, u8)]) -> StreamSpec {
    let mut spec = StreamSpec::new(StreamKind::Mqtt, url);
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 10_000,
        backoff_base_ms: 20,
        backoff_max_ms: 40,
        max_reconnect_attempts: 1,
        ..StreamLimits::default()
    };
    spec.mqtt.subscriptions = topics
        .iter()
        .map(|(topic, qos)| Subscription {
            topic: topic.to_string(),
            qos: *qos,
        })
        .collect();
    spec
}

async fn next_event(events: &mut Receiver<StreamEvent>) -> StreamEvent {
    tokio::time::timeout(Duration::from_secs(15), events.recv())
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

fn is_incoming(event: &StreamEvent, topic: &str) -> bool {
    matches!(
        event,
        StreamEvent::Message {
            direction: Direction::Incoming,
            meta,
            ..
        } if meta.topic.as_deref() == Some(topic)
    )
}

#[tokio::test]
async fn connects_subscribes_publishes_and_receives_its_own_message() {
    let broker = MqttBroker::start().await;
    let spec = spec(&broker.url(), &[("mandalo/test/#", 1)]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let connected = wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;
    match connected {
        StreamEvent::Connected { info, .. } => {
            assert_eq!(info.session_present, Some(false));
            assert_eq!(info.protocol.as_deref(), Some("mqtt/3.1.1"));
        }
        other => panic!("expected a connect, got {other:?}"),
    }

    handle
        .send(Outgoing::Publish {
            topic: "mandalo/test/room".to_string(),
            payload: "21.5".to_string(),
            qos: 1,
            retain: false,
        })
        .await
        .unwrap();

    let delivered = wait_for(&mut events, |e| is_incoming(e, "mandalo/test/room")).await;
    match delivered {
        StreamEvent::Message { payload, meta, .. } => {
            assert_eq!(payload, Payload::text("21.5"));
            assert_eq!(meta.qos, Some(1));
            assert_eq!(meta.retain, Some(false));
            assert_eq!(meta.frame.as_deref(), Some("publish"));
        }
        other => panic!("expected a message, got {other:?}"),
    }

    handle.close().await.unwrap();
    let closed = wait_for(&mut events, StreamEvent::is_terminal).await;
    assert!(matches!(closed, StreamEvent::Disconnected { .. }));
    assert!(!handle.is_open());
}

#[tokio::test]
async fn qos_2_and_the_retain_flag_survive_the_round_trip() {
    let broker = MqttBroker::start().await;
    let spec = spec(&broker.url(), &[("mandalo/retained", 2)]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    handle
        .send(Outgoing::Publish {
            topic: "mandalo/retained".to_string(),
            payload: "keep me".to_string(),
            qos: 2,
            retain: true,
        })
        .await
        .unwrap();

    let delivered = wait_for(&mut events, |e| is_incoming(e, "mandalo/retained")).await;
    match delivered {
        StreamEvent::Message { payload, meta, .. } => {
            assert_eq!(payload.as_text(), Some("keep me"));
            assert_eq!(meta.qos, Some(2));
        }
        other => panic!("expected a message, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn subscribing_after_connecting_works_and_a_bad_qos_fails_loud() {
    let broker = MqttBroker::start().await;
    let spec = spec(&broker.url(), &[]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    let err = handle
        .send(Outgoing::Subscribe {
            topic: "late".to_string(),
            qos: 9,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("qos 9 does not exist"), "{err}");

    handle
        .send(Outgoing::Subscribe {
            topic: "late/#".to_string(),
            qos: 0,
        })
        .await
        .unwrap();
    handle
        .send(Outgoing::Publish {
            topic: "late/one".to_string(),
            payload: "here".to_string(),
            qos: 0,
            retain: false,
        })
        .await
        .unwrap();

    let delivered = wait_for(&mut events, |e| is_incoming(e, "late/one")).await;
    match delivered {
        StreamEvent::Message { payload, .. } => assert_eq!(payload.as_text(), Some("here")),
        other => panic!("expected a message, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn a_text_frame_is_refused_because_mqtt_needs_a_topic() {
    let broker = MqttBroker::start().await;
    let spec = spec(&broker.url(), &[]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    let err = handle.send(Outgoing::text("naked")).await.unwrap_err();
    assert!(err.to_string().contains("needs a topic"), "{err}");
    handle.close().await.unwrap();
}

#[tokio::test]
async fn credentials_are_accepted_and_a_rejected_handshake_says_why() {
    let broker = MqttBroker::start_with_credentials().await;

    let mut good = spec(&broker.url(), &[]);
    good.auth = Auth::Basic {
        username: MQTT_USER.to_string(),
        password: "{{password}}".to_string(),
    };
    good.vars = [("password".to_string(), MQTT_PASSWORD.to_string())]
        .into_iter()
        .collect();
    let (tx, mut events) = stream::event_channel(&good.limits);
    let handle = stream::open(good, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;
    handle.close().await.unwrap();

    let mut bad = spec(&broker.url(), &[]);
    bad.mqtt.username = Some(MQTT_USER.to_string());
    bad.mqtt.password = Some("wrong".to_string());
    let (tx, mut events) = stream::event_channel(&bad.limits);
    let _handle = stream::open(bad, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let (message, code) = all
        .iter()
        .find_map(|e| match e {
            StreamEvent::Error { message, code, .. } => Some((message.clone(), code.clone())),
            _ => None,
        })
        .expect("a rejected handshake must produce an error event");
    assert_eq!(code, stream::E_STREAM_AUTH);
    assert!(
        message.contains("rejected username or password"),
        "{message}"
    );
    assert!(
        !all.iter().any(|e| matches!(e, StreamEvent::Message { .. })),
        "a rejected client must never look connected"
    );
}

#[tokio::test]
async fn mqtt_over_websocket_is_the_same_stream() {
    let broker = MqttBroker::start().await;
    let spec = spec(&broker.ws_url(), &[("mandalo/ws/#", 0)]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    handle
        .send(Outgoing::Publish {
            topic: "mandalo/ws/hello".to_string(),
            payload: "over websocket".to_string(),
            qos: 0,
            retain: false,
        })
        .await
        .unwrap();
    let delivered = wait_for(&mut events, |e| is_incoming(e, "mandalo/ws/hello")).await;
    match delivered {
        StreamEvent::Message { payload, .. } => {
            assert_eq!(payload.as_text(), Some("over websocket"))
        }
        other => panic!("expected a message, got {other:?}"),
    }
    handle.close().await.unwrap();
}

#[tokio::test]
async fn topics_and_the_client_id_are_interpolated() {
    let broker = MqttBroker::start().await;
    let mut spec = spec("mqtt://{{host}}", &[("sensors/{{room}}/#", 0)]);
    spec.mqtt.client_id = Some("mandalo-{{room}}".to_string());
    spec.vars = [
        ("host".to_string(), broker.addr().to_string()),
        ("room".to_string(), "kitchen".to_string()),
    ]
    .into_iter()
    .collect();
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    handle
        .send(Outgoing::Publish {
            topic: "sensors/kitchen/temp".to_string(),
            payload: "19".to_string(),
            qos: 0,
            retain: false,
        })
        .await
        .unwrap();
    wait_for(&mut events, |e| is_incoming(e, "sensors/kitchen/temp")).await;
    handle.close().await.unwrap();
}

#[tokio::test]
async fn no_broker_listening_says_so_by_name() {
    let spec = spec("mqtt://127.0.0.1:1", &[]);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();

    let all = drain(&mut events).await;
    let message = all
        .iter()
        .find_map(|e| match e {
            StreamEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .expect("a refused broker must produce an error event");
    assert!(message.contains("no mqtt broker is listening"), "{message}");
    assert!(
        all.iter()
            .any(|e| matches!(e, StreamEvent::Reconnecting { .. })),
        "a refused broker is worth retrying, and the retry must be visible"
    );
}

const PUBLISHED: u64 = 40;

/// Every publish comes back before the next one goes out, so by the time the
/// last one returns the stream has offered the buffer one event per publish —
/// while nothing here has read an event since the connect. What the buffer could
/// not hold it must report, on any machine at any speed.
async fn fill_by_publishing(spec: StreamSpec) -> Vec<StreamEvent> {
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx).await.unwrap();
    wait_for(&mut events, |e| matches!(e, StreamEvent::Connected { .. })).await;

    for i in 0..PUBLISHED {
        handle
            .send(Outgoing::Publish {
                topic: "mandalo/full".to_string(),
                payload: format!("m{i}"),
                qos: 0,
                retain: false,
            })
            .await
            .unwrap();
    }
    let closing = tokio::spawn(async move { handle.close().await });
    let all = drain(&mut events).await;
    closing.await.unwrap().unwrap();
    all
}

fn dropped_total(all: &[StreamEvent]) -> u64 {
    all.iter()
        .filter_map(|e| match e {
            StreamEvent::Dropped { count, .. } => Some(*count),
            _ => None,
        })
        .sum()
}

#[tokio::test]
async fn a_full_buffer_drops_and_reports_instead_of_growing() {
    let broker = MqttBroker::start().await;
    let mut spec = spec(&broker.url(), &[]);
    spec.limits.max_buffered_events = 4;
    spec.limits.max_buffered_bytes = 8 * 1024 * 1024;

    let all = fill_by_publishing(spec).await;
    let dropped = dropped_total(&all);
    assert!(
        dropped >= PUBLISHED - 1 - 4,
        "{PUBLISHED} publishes into a 4 event buffer that nobody read must report the rest as dropped, got {dropped}: {all:?}"
    );
}

#[tokio::test]
async fn a_buffer_full_of_bytes_drops_and_reports_too() {
    let broker = MqttBroker::start().await;
    let mut spec = spec(&broker.url(), &[]);
    spec.limits.max_buffered_events = 1024;
    spec.limits.max_message_bytes = 4096;
    spec.limits.max_buffered_bytes = 4096;

    let all = fill_by_publishing(spec).await;
    let dropped = dropped_total(&all);
    assert!(
        dropped >= PUBLISHED - 1 - 4096 / 128,
        "a 4096 byte budget cannot park {PUBLISHED} events, got {dropped}: {all:?}"
    );
}

#[tokio::test]
async fn strict_mode_refuses_a_loopback_broker() {
    let broker = MqttBroker::start().await;
    let (tx, _events) = stream::event_channel(&StreamLimits::default());
    let denied = stream::open(spec(&broker.url(), &[]), Arc::new(StrictPolicy::new()), tx)
        .await
        .unwrap_err();
    assert_eq!(denied.code(), "E_HOST_DENIED");
}
