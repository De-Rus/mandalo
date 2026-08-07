use crate::{cors, graphql, Bucket, State, VERSION};
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Extension, Path, Query, Request, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, head, options, patch, post, put};
use axum::{Json, Router};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

pub const BASIC_USER: &str = "ada";
pub const BASIC_PASSWORD: &str = "lovelace";
pub const TOKEN: &str = "mock-bearer-token";
pub const API_KEY_NAME: &str = "x-api-key";
pub const API_KEY_VALUE: &str = "mock-api-key";

pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_BIG_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SLOW_MS: u64 = 10_000;
pub const MAX_REDIRECT_HOPS: u32 = 10;
pub const MAX_STREAM_MB: usize = 128;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: Vec<(String, String)>,
    pub body: String,
    #[serde(skip)]
    pub status: u16,
}

impl ReceivedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn json_body(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

fn decode(raw: &str) -> String {
    let bytes = raw.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_pairs(raw: Option<&str>) -> BTreeMap<String, String> {
    raw.unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<IpAddr> {
    let forwarded = headers
        .get("fly-client-ip")
        .or_else(|| headers.get("cf-connecting-ip"))
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse::<IpAddr>().ok());
    forwarded.or_else(|| peer.map(|p| p.ip()))
}

fn take_token(state: &State, ip: IpAddr) -> bool {
    let Some(per_minute) = state.requests_per_minute else {
        return true;
    };
    let capacity = f64::from(per_minute);
    let now = Instant::now();
    let mut buckets = state.buckets.lock().expect("mock rate limiter");
    if buckets.len() > 10_000 {
        buckets.retain(|_, b| now.duration_since(b.refilled_at).as_secs() < 60);
    }
    let bucket = buckets.entry(ip).or_insert(Bucket {
        tokens: capacity,
        refilled_at: now,
    });
    let elapsed = now.duration_since(bucket.refilled_at).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * capacity / 60.0).min(capacity);
    bucket.refilled_at = now;
    if bucket.tokens < 1.0 {
        return false;
    }
    bucket.tokens -= 1.0;
    true
}

async fn record(
    AxumState(state): AxumState<Arc<State>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let (parts, body) = request.into_parts();
    let ip = client_ip(&parts.headers, peer.map(|p| p.0));
    if let Some(ip) = ip {
        if !take_token(&state, ip) {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": "too many requests" })),
            )
                .into_response());
        }
    }
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES).await.map_err(|_| {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({ "error": "request body is too large", "limit": MAX_BODY_BYTES })),
        )
            .into_response()
    })?;
    let received = ReceivedRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_string(),
        query: query_pairs(parts.uri.query()),
        headers: parts
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or("<binary>").to_string(),
                )
            })
            .collect(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
        status: 0,
    };

    let mut request = Request::from_parts(parts, Body::from(bytes));
    request.extensions_mut().insert(received.clone());

    let started = Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    if state.log {
        println!(
            "{:<7} {:<28} {status} {}ms",
            received.method,
            received.path,
            started.elapsed().as_millis()
        );
    }
    state
        .received
        .lock()
        .expect("mock log")
        .push(ReceivedRequest { status, ..received });
    Ok(response)
}

pub(crate) fn router(state: Arc<State>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/echo", any(echo))
        .route("/get", get(echo))
        .route("/post", post(echo))
        .route("/put", put(echo))
        .route("/patch", patch(echo))
        .route("/delete", delete(echo))
        .route("/head", head(echo))
        .route("/options", options(echo))
        .route("/status/:code", any(status))
        .route("/redirect/:hops", any(redirect_chain))
        .route("/redirect-to", any(redirect_to))
        .route("/slow", any(slow))
        .route("/binary", any(binary))
        .route("/gzip", any(gzip))
        .route("/brotli", any(brotli_body))
        .route("/big", any(big))
        .route("/stream", any(stream_mb))
        .route("/auth/login", post(login))
        .route("/auth/basic", any(auth_basic))
        .route("/auth/bearer", any(auth_bearer))
        .route("/auth/apikey", any(auth_apikey))
        .route("/body/multipart", post(multipart_echo))
        .route("/body/urlencoded", post(urlencoded_echo))
        .route("/body/binary", post(binary_echo))
        .route("/headers/echo", any(headers_echo))
        .route("/cookies/set", any(cookies_set))
        .route("/json", any(json_body))
        .route("/xml", any(xml_body))
        .route("/text", any(text_body))
        .route("/empty", any(empty_body))
        .route("/graphql", post(graphql::handle))
        .merge(crate::sse::routes())
        .merge(crate::ws::routes())
        .layer(middleware::from_fn_with_state(state.clone(), record))
        .layer(middleware::from_fn(cors::apply))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    (StatusCode::OK, "mandalo mock api — GET /health, GET /get")
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "version": VERSION }))
}

async fn echo(Extension(received): Extension<ReceivedRequest>) -> impl IntoResponse {
    Json(received)
}

async fn status(Path(code): Path<u16>) -> Response {
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if status.is_informational() || status == StatusCode::NO_CONTENT || status.as_u16() == 304 {
        return status.into_response();
    }
    if status.is_redirection() {
        return (status, [(header::LOCATION, "/get")]).into_response();
    }
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "status": code }).to_string(),
    )
        .into_response()
}

async fn redirect_chain(Path(hops): Path<u32>) -> Response {
    let next = match hops.min(MAX_REDIRECT_HOPS) {
        0 => "/get".to_string(),
        n => format!("/redirect/{}", n - 1),
    };
    (StatusCode::FOUND, [(header::LOCATION, next)]).into_response()
}

async fn redirect_to(Query(params): Query<BTreeMap<String, String>>) -> Response {
    match params.get("url") {
        Some(url) => (StatusCode::FOUND, [(header::LOCATION, url.clone())]).into_response(),
        None => (StatusCode::BAD_REQUEST, "redirect-to needs ?url=").into_response(),
    }
}

async fn slow(
    Query(params): Query<BTreeMap<String, String>>,
    Extension(received): Extension<ReceivedRequest>,
) -> Response {
    let ms: u64 = params
        .get("ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(MAX_SLOW_MS);
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    Json(received).into_response()
}

async fn binary() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Bytes::from_static(&[0xff, 0xfe, 0x00, 0x01, 0x02, 0x80]),
    )
        .into_response()
}

fn compressed_payload(encoding: &str) -> String {
    serde_json::json!({ "encoding": encoding, "ok": true }).to_string()
}

async fn gzip() -> Response {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(compressed_payload("gzip").as_bytes())
        .expect("gzip encode");
    let bytes = encoder.finish().expect("gzip finish");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        Bytes::from(bytes),
    )
        .into_response()
}

async fn brotli_body() -> Response {
    let payload = compressed_payload("brotli");
    let mut bytes = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut bytes, 4096, 5, 22);
        writer.write_all(payload.as_bytes()).expect("brotli encode");
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CONTENT_ENCODING, "br"),
        ],
        Bytes::from(bytes),
    )
        .into_response()
}

async fn big(Query(params): Query<BTreeMap<String, String>>) -> Response {
    let bytes: usize = params
        .get("bytes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024)
        .min(MAX_BIG_BYTES);
    let body = serde_json::json!({ "filler": "x".repeat(bytes) }).to_string();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// The same megabyte of `x` handed out N times: a body far larger than any
/// client should accept, without the mock ever holding more than one chunk.
async fn stream_mb(Query(params): Query<BTreeMap<String, String>>) -> Response {
    let mb: usize = params
        .get("mb")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .min(MAX_STREAM_MB);
    let chunk = Bytes::from(vec![b'x'; 1024 * 1024]);
    let chunks: Vec<Result<Bytes, std::io::Error>> =
        std::iter::repeat_n(chunk, mb).map(Ok).collect();
    let body = Body::from_stream(tokio_stream::iter(chunks));
    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response();
    if params.get("chunked").map(String::as_str) != Some("1") {
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from(mb as u64 * 1024 * 1024),
        );
    }
    response
}

fn unauthorized(realm: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, realm.to_string())],
        Json(serde_json::json!({ "error": "unauthorized" })),
    )
        .into_response()
}

async fn login(Extension(received): Extension<ReceivedRequest>) -> Response {
    let body = received.json_body();
    let user = body["username"].as_str().unwrap_or_default();
    let password = body["password"].as_str().unwrap_or_default();
    if user != BASIC_USER || password != BASIC_PASSWORD {
        return unauthorized("Login").into_response();
    }
    Json(serde_json::json!({
        "token": TOKEN,
        "user": { "id": "u-1", "name": "Ada Lovelace" }
    }))
    .into_response()
}

fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = raw.strip_prefix("Basic ")?;
    let decoded = base64_decode(encoded)?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, password) = text.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

fn base64_decode(raw: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for byte in raw.bytes().filter(|b| *b != b'=') {
        let value = ALPHABET.iter().position(|c| *c == byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

async fn auth_basic(
    headers: HeaderMap,
    Extension(received): Extension<ReceivedRequest>,
) -> Response {
    match basic_credentials(&headers) {
        Some((user, password)) if user == BASIC_USER && password == BASIC_PASSWORD => {
            Json(serde_json::json!({ "authenticated": true, "user": user, "path": received.path }))
                .into_response()
        }
        _ => unauthorized("Basic realm=\"mock\""),
    }
}

async fn auth_bearer(headers: HeaderMap) -> Response {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(token) if token == TOKEN => {
            Json(serde_json::json!({ "authenticated": true, "token": token })).into_response()
        }
        _ => unauthorized("Bearer"),
    }
}

async fn auth_apikey(Extension(received): Extension<ReceivedRequest>) -> Response {
    let from_header = received.header(API_KEY_NAME);
    let from_query = received.query.get(API_KEY_NAME).map(String::as_str);
    let placement = match (from_header, from_query) {
        (Some(v), _) if v == API_KEY_VALUE => "header",
        (_, Some(v)) if v == API_KEY_VALUE => "query",
        _ => return unauthorized("ApiKey"),
    };
    Json(serde_json::json!({ "authenticated": true, "placement": placement })).into_response()
}

/// Reports every part in the order it arrived. Parts are deliberately NOT
/// grouped by name: several files posted under one field name is a real HTTP
/// shape, and grouping would hide whether they all made it.
async fn multipart_echo(
    Extension(received): Extension<ReceivedRequest>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let mut parts = Vec::new();
    loop {
        let field =
            match multipart.next_field().await {
                Ok(Some(field)) => field,
                Ok(None) => break,
                Err(e) => return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unreadable multipart body: {e}") })),
                )
                    .into_response(),
            };
        let name = field.name().unwrap_or_default().to_string();
        let file_name = field.file_name().map(String::from);
        let content_type = field.content_type().map(String::from);
        let bytes = field.bytes().await.unwrap_or_default().to_vec();
        parts.push(serde_json::json!({
            "name": name,
            "filename": file_name,
            "contentType": content_type,
            "size": bytes.len(),
            "text": String::from_utf8(bytes.clone()).ok(),
            "bytes": bytes,
        }));
    }
    Json(serde_json::json!({
        "count": parts.len(),
        "contentType": received.header("content-type"),
        "parts": parts,
    }))
    .into_response()
}

async fn urlencoded_echo(Extension(received): Extension<ReceivedRequest>) -> Response {
    let pairs: Vec<[String; 2]> = received
        .body
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => [decode(k), decode(v)],
            None => [decode(pair), String::new()],
        })
        .collect();
    Json(serde_json::json!({
        "count": pairs.len(),
        "contentType": received.header("content-type"),
        "raw": received.body,
        "pairs": pairs,
    }))
    .into_response()
}

async fn binary_echo(Extension(received): Extension<ReceivedRequest>, bytes: Bytes) -> Response {
    Json(serde_json::json!({
        "size": bytes.len(),
        "contentType": received.header("content-type"),
        "text": String::from_utf8(bytes.to_vec()).ok(),
        "bytes": bytes.to_vec(),
    }))
    .into_response()
}

async fn headers_echo(Extension(received): Extension<ReceivedRequest>) -> Response {
    Json(serde_json::json!({
        "headers": received.headers,
        "count": received.headers.len()
    }))
    .into_response()
}

async fn cookies_set() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("mock_session=abc123; Path=/; HttpOnly"),
    );
    (
        StatusCode::OK,
        headers,
        Json(serde_json::json!({ "cookie": "mock_session=abc123" })),
    )
        .into_response()
}

async fn json_body() -> Response {
    Json(serde_json::json!({
        "id": 7,
        "name": "nova",
        "tags": ["a", "b"],
        "nested": { "ok": true }
    }))
    .into_response()
}

async fn xml_body() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        "<?xml version=\"1.0\"?><user><id>7</id><name>nova</name></user>",
    )
        .into_response()
}

async fn text_body() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "hola",
    )
        .into_response()
}

async fn empty_body() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// A narrower view of [`crate::MockApi`] for tests that only need "a server that echoes":
/// same server, same recording, one call to start it.
pub struct MockServer(crate::MockApi);

impl MockServer {
    pub async fn start() -> MockServer {
        MockServer(crate::MockApi::start().await)
    }

    pub fn base_url(&self) -> String {
        self.0.base_url()
    }

    pub fn url(&self, path: &str) -> String {
        self.0.url(path)
    }

    pub async fn last_request(&self) -> Option<ReceivedRequest> {
        self.0.last_request()
    }

    pub async fn received_requests(&self) -> Vec<ReceivedRequest> {
        self.0.received_requests()
    }
}
