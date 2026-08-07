mod cors;
pub mod fixtures;
mod graphql;
mod grpc;
pub mod http;
pub mod mqtt;
pub mod sse;
pub mod ws;

pub use grpc::PROTO;
pub use http::{
    MockServer, ReceivedRequest, API_KEY_NAME, API_KEY_VALUE, BASIC_PASSWORD, BASIC_USER,
    MAX_BIG_BYTES, MAX_BODY_BYTES, MAX_REDIRECT_HOPS, MAX_SLOW_MS, TOKEN,
};
pub use mqtt::{MqttBroker, MQTT_PASSWORD, MQTT_USER};
pub use sse::RETRY_MS;
pub use ws::{BINARY_FRAME, CLOSE_CODE, CLOSE_REASON};

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub const GRPC_SERVICE: &str = "mock.v1.Mock";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Options {
    pub bind: IpAddr,
    pub http_port: u16,
    pub grpc_port: u16,
    pub log: bool,
    pub proto_dir: Option<PathBuf>,
    pub requests_per_minute: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            http_port: 0,
            grpc_port: 0,
            log: false,
            proto_dir: None,
            requests_per_minute: None,
        }
    }
}

pub(crate) struct Bucket {
    pub(crate) tokens: f64,
    pub(crate) refilled_at: std::time::Instant,
}

pub(crate) struct State {
    pub(crate) received: Mutex<Vec<ReceivedRequest>>,
    pub(crate) buckets: Mutex<HashMap<IpAddr, Bucket>>,
    pub(crate) log: bool,
    pub(crate) requests_per_minute: Option<u32>,
}

enum ProtoHome {
    Temp(tempfile::TempDir),
    Fixed(PathBuf),
}

impl ProtoHome {
    fn dir(&self) -> &Path {
        match self {
            ProtoHome::Temp(dir) => dir.path(),
            ProtoHome::Fixed(dir) => dir.as_path(),
        }
    }
}

pub struct MockApi {
    addr: SocketAddr,
    grpc_addr: SocketAddr,
    state: Arc<State>,
    proto_home: ProtoHome,
    _http_stop: oneshot::Sender<()>,
    _grpc_stop: oneshot::Sender<()>,
}

impl MockApi {
    pub async fn start() -> MockApi {
        MockApi::start_with(Options::default()).await
    }

    pub async fn start_with(options: Options) -> MockApi {
        let proto_home = match &options.proto_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir).expect("mock proto directory");
                ProtoHome::Fixed(dir.clone())
            }
            None => ProtoHome::Temp(tempfile::tempdir().expect("mock proto directory")),
        };
        let proto_path = proto_home.dir().join("mock.proto");
        std::fs::write(&proto_path, PROTO).expect("write mock.proto");

        let state = Arc::new(State {
            received: Mutex::new(Vec::new()),
            buckets: Mutex::new(HashMap::new()),
            log: options.log,
            requests_per_minute: options.requests_per_minute,
        });

        let listener = tokio::net::TcpListener::bind((options.bind, options.http_port))
            .await
            .expect("bind mock http port");
        let addr = listener.local_addr().expect("mock http address");
        let (http_stop, http_stopped) = oneshot::channel();
        let app = http::router(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = http_stopped.await;
            })
            .await;
        });

        let (grpc_addr, grpc_stop) =
            grpc::spawn(options.bind, options.grpc_port, &proto_path, options.log).await;

        MockApi {
            addr,
            grpc_addr,
            state,
            proto_home,
            _http_stop: http_stop,
            _grpc_stop: grpc_stop,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn grpc_addr(&self) -> SocketAddr {
        self.grpc_addr
    }

    pub fn grpc_url(&self) -> String {
        format!("http://{}", self.grpc_addr)
    }

    pub fn proto_path(&self) -> String {
        self.proto_home
            .dir()
            .join("mock.proto")
            .to_str()
            .expect("utf-8 proto path")
            .to_string()
    }

    pub fn received_requests(&self) -> Vec<ReceivedRequest> {
        self.state.received.lock().expect("mock log").clone()
    }

    pub fn requests_to(&self, path: &str) -> Vec<ReceivedRequest> {
        self.received_requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }

    pub fn last_request(&self) -> Option<ReceivedRequest> {
        self.received_requests().pop()
    }

    pub fn clear_requests(&self) {
        self.state.received.lock().expect("mock log").clear();
    }

    pub fn index(&self) -> String {
        index(&self.base_url(), &self.grpc_url(), &self.proto_path())
    }
}

pub fn index(base_url: &str, grpc_url: &str, proto_path: &str) -> String {
    let mut out = format!(
        "Mándalo mock API {VERSION}\n\n  HTTP  {base_url}\n  gRPC  {grpc_url}\n  proto {proto_path}\n"
    );
    for (area, lines) in ENDPOINTS {
        out.push_str(&format!("\n{area}\n"));
        for (path, what) in *lines {
            out.push_str(&format!("  {path:<28} {what}\n"));
        }
    }
    out.push_str(&format!(
        "\ncredentials\n  {:<28} {BASIC_USER} / {BASIC_PASSWORD}\n  {:<28} {TOKEN}\n  {:<28} {API_KEY_NAME}: {API_KEY_VALUE}\n",
        "basic", "bearer", "api key (header or query)"
    ));
    out
}

type Area = (&'static str, &'static [(&'static str, &'static str)]);

const ENDPOINTS: &[Area] = &[
    (
        "methods — each echoes method, path, query, headers and body",
        &[
            ("GET     /get", "echo"),
            ("POST    /post", "echo"),
            ("PUT     /put", "echo"),
            ("PATCH   /patch", "echo"),
            ("DELETE  /delete", "echo"),
            ("HEAD    /head", "echo without a body"),
            ("OPTIONS /options", "echo"),
        ],
    ),
    (
        "status, redirects and timing",
        &[
            ("/status/:code", "responds with that status"),
            ("/redirect/:n", "n hops, then /get (n capped at 10)"),
            ("/redirect-to?url=", "one redirect response, never a fetch"),
            ("/slow?ms=", "responds after ms (capped at 10000)"),
        ],
    ),
    (
        "auth",
        &[
            ("POST /auth/login", "username + password in, token out"),
            ("/auth/basic", "401 unless the Basic credentials match"),
            ("/auth/bearer", "401 unless the Bearer token matches"),
            (
                "/auth/apikey",
                "401 unless the api key header or query matches",
            ),
        ],
    ),
    (
        "bodies and encoding",
        &[
            ("/json", "application/json"),
            ("/xml", "application/xml"),
            ("/text", "text/plain"),
            ("/empty", "204, no body"),
            ("/binary", "non-utf8 octet-stream"),
            ("/gzip", "gzip-encoded json"),
            ("/brotli", "brotli-encoded json"),
            ("/big?bytes=", "a large json body (capped at 5 MB)"),
            (
                "/stream?mb=&chunked=",
                "mb megabytes, declared or chunked (capped at 128)",
            ),
            ("/headers/echo", "every request header, duplicates kept"),
            ("/cookies/set", "sets a cookie"),
            ("/health", "status + version"),
        ],
    ),
    (
        "graphql — POST /graphql with {query, variables}",
        &[
            ("{ user(id: $id) { … } }", "one user"),
            ("{ users { id name } }", "two users"),
            ("mutation createUser", "creates and echoes a user"),
            ("{ boom }", "errors array with HTTP 200"),
            ("{ malformed }", "invalid JSON with HTTP 200"),
        ],
    ),
    (
        "websocket — ws://host/…",
        &[
            ("/ws/echo", "echoes text and binary frames"),
            ("/ws/close", "one message, then closes with 4001"),
            ("/ws/binary", "sends binary frames"),
            ("/ws/ping", "sends pings"),
            ("/ws/firehose?n=", "sends n messages as fast as it can"),
            ("/ws/big?bytes=", "one very large message"),
            ("/ws/auth", "401 unless the bearer token or api key matches"),
            ("/ws/protocol", "negotiates the v2 or chat subprotocol"),
            (
                "/ws/flaky",
                "answers once, then drops without a close frame",
            ),
        ],
    ),
    (
        "server-sent events — GET, Accept: text/event-stream",
        &[
            ("/sse/basic?n=", "n plain data events"),
            ("/sse/multiline", "comment, event name, multi-line data, id"),
            ("/sse/ids?n=", "ids that resume from Last-Event-ID"),
            ("/sse/retry", "sets the reconnect interval, then ends"),
            ("/sse/big?bytes=", "one very large event"),
            ("/sse/forever", "never stops"),
            ("/sse/not-a-stream", "json, so the client must refuse it"),
            ("/sse/denied", "401"),
        ],
    ),
    (
        "grpc — service mock.v1.Mock (native and grpc-web)",
        &[
            ("Say", "text in, text + metadata echo out"),
            ("GetUser", "nested message, repeated field, enum"),
            ("Fail", "returns a FailedPrecondition status"),
            ("Slow", "responds after ms (capped at 10000)"),
            ("Ticks", "server streaming — the client must refuse it"),
        ],
    ),
];
