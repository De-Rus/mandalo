use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::stream::{self, Direction, StreamEvent, StreamKind, StreamLimits, StreamSpec};
use mandalo_core::workspace::{self, Manifest, SCHEMA_VERSION};
use mandalo_core::AllowAll;
use mandalo_testkit::{MockApi, MqttBroker};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

struct Workspace {
    dir: tempfile::TempDir,
}

impl Workspace {
    fn new() -> Workspace {
        let dir = tempfile::tempdir().expect("a temporary workspace");
        std::fs::create_dir_all(collection::collections_dir(dir.path())).expect("collections/");
        workspace::write_manifest(
            dir.path(),
            &Manifest {
                schema_version: SCHEMA_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                name: "Streams".to_string(),
                remote: None,
                share: None,
            },
        )
        .expect("a manifest");
        collection::create_collection(dir.path(), "Live").expect("a collection");
        Workspace { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, file: &str, contents: &str) {
        collection::write_file(self.path(), "live", file, contents).expect("the file is written");
    }

    fn load(&self, path: &str) -> SavedRequest {
        collection::load_request(self.path(), "live", path).expect("the request loads")
    }
}

fn testing_limits(mut spec: StreamSpec) -> StreamSpec {
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 10_000,
        backoff_base_ms: 20,
        backoff_max_ms: 40,
        max_reconnect_attempts: 2,
        ..StreamLimits::default()
    };
    spec
}

async fn next_event(events: &mut Receiver<StreamEvent>) -> StreamEvent {
    tokio::time::timeout(Duration::from_secs(15), events.recv())
        .await
        .expect("the stream produced no event in time")
        .expect("the stream ended without an event")
}

async fn wait_for_incoming(events: &mut Receiver<StreamEvent>) -> StreamEvent {
    loop {
        let event = next_event(events).await;
        if matches!(
            event,
            StreamEvent::Message {
                direction: Direction::Incoming,
                ..
            }
        ) {
            return event;
        }
        if event.is_terminal() {
            panic!("the stream ended before a message arrived: {event:?}");
        }
    }
}

async fn wait_for_connected(events: &mut Receiver<StreamEvent>) {
    loop {
        let event = next_event(events).await;
        if matches!(event, StreamEvent::Connected { .. }) {
            return;
        }
        if event.is_terminal() {
            panic!("the stream ended before it connected: {event:?}");
        }
    }
}

fn text_of(event: &StreamEvent) -> String {
    match event {
        StreamEvent::Message { payload, .. } => {
            payload.as_text().unwrap_or_default().trim().to_string()
        }
        other => panic!("expected a message, got {other:?}"),
    }
}

fn topic_of(event: &StreamEvent) -> String {
    match event {
        StreamEvent::Message { meta, .. } => meta.topic.clone().unwrap_or_default(),
        other => panic!("expected a message, got {other:?}"),
    }
}

#[tokio::test]
async fn a_saved_websocket_connects_and_sends_a_named_message() {
    let api = MockApi::start().await;
    let ws = Workspace::new();
    ws.write(
        "chat/socket.ws",
        &format!(
            "@host = {}\n\n### Echo\nws://{{{{host}}}}/ws/echo\nx-trace: mandalo\n\n>> hola\nhola {{{{who}}}}\n",
            api.addr()
        ),
    );

    let saved = ws.load("chat/socket.ws#0");
    assert_eq!(saved.kind, "websocket");
    let stream = saved.stream.clone().expect("a stream");
    let vars: BTreeMap<String, String> = [("who".to_string(), "mundo".to_string())]
        .into_iter()
        .collect();
    let message = stream::outgoing_for(&stream, "hola", &vars).expect("the named message");

    let spec = testing_limits(stream::spec_for(&saved, vars).expect("a spec"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx)
        .await
        .expect("the socket opens");

    handle.send(message).await.expect("the message goes out");
    assert_eq!(text_of(&wait_for_incoming(&mut events).await), "hola mundo");
    handle.close().await.expect("the socket closes");
}

#[tokio::test]
async fn a_saved_sse_request_is_an_http_get_that_accepts_the_event_stream() {
    let api = MockApi::start().await;
    let ws = Workspace::new();
    ws.write(
        "feeds/events.http",
        &format!(
            "### Events\n{}/sse/basic\nAccept: text/event-stream\n",
            api.base_url()
        ),
    );

    let saved = ws.load("feeds/events.http#0");
    assert_eq!(saved.kind, "sse");
    assert_eq!(saved.method, "GET");
    assert_eq!(
        mandalo_core::collection::stream_kind(&saved.kind),
        Some(StreamKind::Sse)
    );

    let spec = testing_limits(stream::spec_for(&saved, BTreeMap::new()).expect("a spec"));
    // The transport owns the Accept header, so the saved one does not travel twice.
    assert!(spec.headers.is_empty(), "{:?}", spec.headers);
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx)
        .await
        .expect("the stream opens");

    assert!(!text_of(&wait_for_incoming(&mut events).await).is_empty());
    handle.close().await.expect("the stream closes");
}

#[tokio::test]
async fn a_saved_mqtt_connection_subscribes_and_publishes_to_an_interpolated_topic() {
    let broker = MqttBroker::start().await;
    let ws = Workspace::new();
    ws.write(
        "sensors/room.mqtt",
        &format!(
            "### Sensors\n{}\nclient-id: mandalo-test\nsubscribe: sensors/#; qos=1\n\n>> report\ntopic: sensors/{{{{room}}}}/temp\nqos: 1\n\n{{\"c\": 21.5}}\n",
            broker.url()
        ),
    );

    let saved = ws.load("sensors/room.mqtt#0");
    assert_eq!(saved.kind, "mqtt");
    let stream = saved.stream.clone().expect("a stream");
    let vars: BTreeMap<String, String> = [("room".to_string(), "cocina".to_string())]
        .into_iter()
        .collect();
    let message = stream::outgoing_for(&stream, "report", &vars).expect("the named message");

    let spec = testing_limits(stream::spec_for(&saved, vars).expect("a spec"));
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, Arc::new(AllowAll), tx)
        .await
        .expect("the broker accepts the connection");

    wait_for_connected(&mut events).await;
    handle.send(message).await.expect("the publish goes out");
    let received = loop {
        let event = wait_for_incoming(&mut events).await;
        if !topic_of(&event).is_empty() {
            break event;
        }
    };
    assert_eq!(topic_of(&received), "sensors/cocina/temp");
    assert_eq!(text_of(&received), "{\"c\": 21.5}");
    handle.close().await.expect("the connection closes");
}

#[tokio::test]
async fn the_tree_lists_streams_beside_http_and_grpc_and_addresses_them_by_index() {
    let ws = Workspace::new();
    ws.write("a.http", "### Plain\nhttps://x.dev/a\n");
    ws.write(
        "b.grpc",
        "### Call\nhttps://x.dev/p.S/M\nproto: protos/x.proto\n",
    );
    ws.write(
        "c.ws",
        "### First socket\nwss://x.dev/one\n\n>> go\n1\n\n### Second socket\nwss://x.dev/two\n",
    );
    ws.write("d.mqtt", "### Broker\nmqtt://x.dev\nsubscribe: a/#\n");
    ws.write(
        "e.http",
        "### Feed\nhttps://x.dev/e\nAccept: text/event-stream\n",
    );

    let tree = collection::list_tree(ws.path()).expect("the tree");
    assert!(tree.skipped.is_empty(), "{:?}", tree.skipped);
    let listed: Vec<(String, String, String)> = tree.collections[0]
        .requests
        .iter()
        .map(|r| (r.kind.clone(), r.method.clone(), r.path.clone()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("http".into(), "GET".into(), "a.http#0".to_string()),
            ("grpc".into(), "POST".into(), "b.grpc#0".to_string()),
            ("websocket".into(), "WS".into(), "c.ws#0".to_string()),
            ("websocket".into(), "WS".into(), "c.ws#1".to_string()),
            ("mqtt".into(), "MQTT".into(), "d.mqtt#0".to_string()),
            ("sse".into(), "GET".into(), "e.http#0".to_string()),
        ]
    );

    assert_eq!(ws.load("c.ws#1").url, "wss://x.dev/two");
    assert_eq!(ws.load("c.ws#Second socket").url, "wss://x.dev/two");
}

#[tokio::test]
async fn a_stream_moves_renames_and_is_deleted_like_every_other_request() {
    let ws = Workspace::new();
    ws.write(
        "c.ws",
        "### First\nwss://x.dev/one\n\n>> go\n1\n\n### Second\nwss://x.dev/two\n",
    );
    collection::create_folder(ws.path(), "live", "sockets").expect("a folder");

    let moved = collection::move_request(ws.path(), "live", "c.ws#1", "sockets")
        .expect("the second socket moves")
        .path;
    assert_eq!(moved, "sockets/second.ws#0");
    assert_eq!(ws.load(&moved).url, "wss://x.dev/two");

    let mut renamed = ws.load("c.ws#0");
    renamed.name = "Renamed".to_string();
    collection::save_request(ws.path(), "live", Some("c.ws#0"), None, &renamed)
        .expect("the rename lands");
    assert_eq!(ws.load("c.ws#0").name, "Renamed");

    collection::delete_request(ws.path(), "live", &moved).expect("the socket is deleted");
    assert!(collection::load_request(ws.path(), "live", &moved).is_err());
}

#[tokio::test]
async fn a_stream_survives_a_bundle_export_and_import() {
    let source = Workspace::new();
    source.write(
        "chat/socket.ws",
        "### Echo\nwss://{{host}}/ws/echo\nsubprotocol: chat.v2\n\n>> hola\nhola\n",
    );
    source.write(
        "sensors/room.mqtt",
        "### Sensors\nmqtt://x.dev\nsubscribe: a/#; qos=2\n",
    );
    source.write(
        "feeds/events.http",
        "### Feed\nhttps://x.dev/e\nAccept: text/event-stream\n",
    );

    let exported = mandalo_core::bundle::export(source.path()).expect("the bundle exports");

    let target = Workspace::new();
    mandalo_core::bundle::import(target.path(), &exported.json).expect("the bundle imports");
    for path in [
        "chat/socket.ws#0",
        "sensors/room.mqtt#0",
        "feeds/events.http#0",
    ] {
        assert_eq!(
            collection::load_request(target.path(), "live", path).expect("the request came back"),
            source.load(path),
            "{path}"
        );
    }
}
