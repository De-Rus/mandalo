use mandalo_core::stream::{self, StreamEvent, StreamKind, StreamLimits, StreamSpec};
use mandalo_core::{Decision, HostPolicy};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;

#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
}

impl HostPolicy for Counting {
    fn allow(&self, _host: &str, _ip: &IpAddr) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Decision::Allow
    }
}

struct OnlyLoopbackByNumber;

impl HostPolicy for OnlyLoopbackByNumber {
    fn allow(&self, host: &str, _ip: &IpAddr) -> Decision {
        if host == "127.0.0.1" {
            Decision::Allow
        } else {
            Decision::Deny(format!("{host} is not on the allow list"))
        }
    }
}

fn spec(kind: StreamKind, url: &str) -> StreamSpec {
    let mut spec = StreamSpec::new(kind, url);
    spec.limits = StreamLimits {
        connect_timeout_ms: 5_000,
        idle_timeout_ms: 5_000,
        backoff_base_ms: 10,
        backoff_max_ms: 20,
        max_reconnect_attempts: 2,
        ..StreamLimits::default()
    };
    spec.ws.auto_reconnect = true;
    spec.sse.auto_reconnect = true;
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

fn errors(all: &[StreamEvent]) -> Vec<String> {
    all.iter()
        .filter_map(|e| match e {
            StreamEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn texts(all: &[StreamEvent]) -> Vec<String> {
    all.iter()
        .filter_map(|e| match e {
            StreamEvent::Message { payload, .. } => Some(payload.as_text()?.to_string()),
            _ => None,
        })
        .collect()
}

struct Endpoint {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Endpoint {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

async fn serve<F>(reply: F) -> Endpoint
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            let head = {
                let mut buffer = Vec::new();
                let mut byte = [0u8; 1];
                while socket.read_exact(&mut byte).await.is_ok() {
                    buffer.push(byte[0]);
                    if buffer.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                String::from_utf8_lossy(&buffer).to_ascii_lowercase()
            };
            recorded.lock().unwrap().push(head.clone());
            let _ = socket.write_all(reply(&head).as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    Endpoint { addr, seen }
}

#[tokio::test]
async fn the_host_policy_runs_on_every_connect_attempt() {
    for (kind, url) in [
        (StreamKind::WebSocket, "ws://127.0.0.1:1/socket"),
        (StreamKind::Sse, "http://127.0.0.1:1/events"),
        (StreamKind::Mqtt, "mqtt://127.0.0.1:1"),
    ] {
        let policy = Arc::new(Counting::default());
        let spec = spec(kind, url);
        let (tx, mut events) = stream::event_channel(&spec.limits);
        let _handle = stream::open(spec, policy.clone(), tx).await.unwrap();

        let all = drain(&mut events).await;
        let attempts = all
            .iter()
            .filter(|e| matches!(e, StreamEvent::Connecting { .. }))
            .count();
        assert_eq!(attempts, 3, "{kind:?} should try three times: {all:?}");
        assert_eq!(
            policy.calls.load(Ordering::SeqCst),
            attempts + 1,
            "{kind:?} must ask the policy once before it starts and once per attempt"
        );
    }
}

#[tokio::test]
async fn a_redirect_to_another_host_loses_the_headers_it_arrived_with() {
    let target = serve(|_| {
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: arrived\n\n".to_string()
    })
    .await;
    let hop = target.addr;
    let entry = serve(move |_| {
        format!("HTTP/1.1 302 Found\r\nlocation: http://{hop}/events\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    })
    .await;

    let mut spec = spec(StreamKind::Sse, &entry.url("/events"));
    spec.sse.auto_reconnect = false;
    spec.headers = vec![("X-Api-Key".to_string(), "s3cr3t-key".to_string())];
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(mandalo_core::AllowAll), tx)
        .await
        .unwrap();

    let all = drain(&mut events).await;
    assert_eq!(texts(&all), vec!["arrived".to_string()], "{all:?}");

    let asked = entry.seen.lock().unwrap().clone();
    assert!(asked[0].contains("x-api-key"), "{asked:?}");
    let followed = target.seen.lock().unwrap().clone();
    assert_eq!(followed.len(), 1, "the redirect must be followed once");
    assert!(
        !followed[0].contains("x-api-key") && !followed[0].contains("s3cr3t-key"),
        "the headers must not travel to another host: {followed:?}"
    );
}

#[tokio::test]
async fn a_redirect_to_a_denied_host_never_opens_a_socket() {
    let target = serve(|_| {
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: arrived\n\n".to_string()
    })
    .await;
    let port = target.addr.port();
    let entry = serve(move |_| {
        format!("HTTP/1.1 302 Found\r\nlocation: http://localhost:{port}/events\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    })
    .await;

    let mut spec = spec(StreamKind::Sse, &entry.url("/events"));
    spec.sse.auto_reconnect = false;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(OnlyLoopbackByNumber), tx)
        .await
        .unwrap();

    let all = drain(&mut events).await;
    assert!(
        errors(&all).iter().any(|m| m.contains("localhost")),
        "the denial must name the host it refused: {all:?}"
    );
    assert!(
        target.seen.lock().unwrap().is_empty(),
        "a denied host must never be asked for anything"
    );
}

#[tokio::test]
async fn redirects_that_never_end_fail_loud() {
    let entry = serve(|_| {
        "HTTP/1.1 302 Found\r\nlocation: /again\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            .to_string()
    })
    .await;

    let mut spec = spec(StreamKind::Sse, &entry.url("/events"));
    spec.sse.auto_reconnect = false;
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let _handle = stream::open(spec, Arc::new(mandalo_core::AllowAll), tx)
        .await
        .unwrap();

    let all = drain(&mut events).await;
    assert!(
        errors(&all)
            .iter()
            .any(|m| m.contains("redirected more than")),
        "{all:?}"
    );
    assert!(
        entry.seen.lock().unwrap().len() <= 11,
        "the hop cap must stop the walk"
    );
}
