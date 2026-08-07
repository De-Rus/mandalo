mod mqtt;
mod sse;
mod ws;

use crate::capability::{Decision, HostPolicy};
use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use crate::request::Auth;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

pub use sse::SseFrame;

/// A connection that stayed up this long counts as healthy, so the reconnect
/// budget starts over. Without it a server that accepts and drops immediately
/// would be retried forever, because every attempt looks like the first.
pub(crate) const HEALTHY_SESSION: Duration = Duration::from_secs(30);

/// How long the closing events wait for a consumer that has stopped reading.
const FINAL_EVENT_WAIT: Duration = Duration::from_secs(5);

pub const E_STREAM_CONNECT: &str = "E_STREAM_CONNECT";
pub const E_STREAM_TLS: &str = "E_STREAM_TLS";
pub const E_STREAM_AUTH: &str = "E_STREAM_AUTH";
pub const E_STREAM_PROTOCOL: &str = "E_STREAM_PROTOCOL";
pub const E_STREAM_LIMIT: &str = "E_STREAM_LIMIT";
pub const E_STREAM_IDLE: &str = "E_STREAM_IDLE";

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    #[default]
    #[serde(alias = "ws")]
    WebSocket,
    Sse,
    Mqtt,
}

impl StreamKind {
    pub fn label(&self) -> &'static str {
        match self {
            StreamKind::WebSocket => "websocket",
            StreamKind::Sse => "sse",
            StreamKind::Mqtt => "mqtt",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct StreamLimits {
    pub max_message_bytes: usize,
    pub max_buffered_events: usize,
    /// How much memory the parked events may hold. A count alone is not a
    /// budget: a thousand slots of a megabyte each is a gigabyte.
    pub max_buffered_bytes: usize,
    pub max_reconnect_attempts: u32,
    pub connect_timeout_ms: u64,
    /// No inbound traffic for this long ends the stream. `0` disables it.
    pub idle_timeout_ms: u64,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
}

impl Default for StreamLimits {
    fn default() -> Self {
        StreamLimits {
            max_message_bytes: 1024 * 1024,
            max_buffered_events: 1024,
            max_buffered_bytes: 8 * 1024 * 1024,
            max_reconnect_attempts: 5,
            connect_timeout_ms: 15_000,
            idle_timeout_ms: 300_000,
            backoff_base_ms: 500,
            backoff_max_ms: 30_000,
        }
    }
}

const MESSAGE_BYTES_CEILING: usize = 64 * 1024 * 1024;
const BUFFERED_EVENTS_CEILING: usize = 65_536;
const BUFFERED_BYTES_CEILING: usize = 256 * 1024 * 1024;
const RECONNECT_ATTEMPTS_CEILING: u32 = 1_000;
const CONNECT_TIMEOUT_CEILING_MS: u64 = 300_000;
const IDLE_TIMEOUT_CEILING_MS: u64 = 24 * 60 * 60 * 1000;
const BACKOFF_CEILING_MS: u64 = 600_000;

impl StreamLimits {
    fn backoff(&self, attempt: u32) -> u64 {
        let shift = attempt.saturating_sub(1).min(16);
        self.backoff_base_ms
            .saturating_mul(1u64 << shift)
            .min(self.backoff_max_ms)
            .max(1)
    }

    /// Limits arrive straight off IPC, so every field is pulled back into a
    /// range this process can actually honour before anything is sized by it.
    pub fn sanitized(&self) -> StreamLimits {
        let max_message_bytes = self.max_message_bytes.clamp(1, MESSAGE_BYTES_CEILING);
        let backoff_base_ms = self.backoff_base_ms.clamp(1, BACKOFF_CEILING_MS);
        StreamLimits {
            max_message_bytes,
            max_buffered_events: self.max_buffered_events.clamp(1, BUFFERED_EVENTS_CEILING),
            max_buffered_bytes: self
                .max_buffered_bytes
                .clamp(max_message_bytes, BUFFERED_BYTES_CEILING),
            max_reconnect_attempts: self.max_reconnect_attempts.min(RECONNECT_ATTEMPTS_CEILING),
            connect_timeout_ms: self.connect_timeout_ms.clamp(1, CONNECT_TIMEOUT_CEILING_MS),
            idle_timeout_ms: self.idle_timeout_ms.min(IDLE_TIMEOUT_CEILING_MS),
            backoff_base_ms,
            backoff_max_ms: self
                .backoff_max_ms
                .clamp(backoff_base_ms, BACKOFF_CEILING_MS),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct WsOptions {
    pub subprotocols: Vec<String>,
    pub auto_reconnect: bool,
    /// Client-initiated pings, `0` disables them.
    pub ping_interval_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct SseOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default = "yes")]
    pub auto_reconnect: bool,
}

fn yes() -> bool {
    true
}

impl Default for SseOptions {
    fn default() -> Self {
        SseOptions {
            last_event_id: None,
            auto_reconnect: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum MqttVersion {
    #[default]
    #[serde(rename = "3.1.1", alias = "v311", alias = "4")]
    V311,
    #[serde(rename = "5", alias = "v5")]
    V5,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub topic: String,
    #[serde(default)]
    pub qos: u8,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct MqttOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default = "yes")]
    pub clean_session: bool,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub protocol_version: MqttVersion,
}

fn default_keep_alive() -> u64 {
    60
}

impl Default for MqttOptions {
    fn default() -> Self {
        MqttOptions {
            client_id: None,
            username: None,
            password: None,
            clean_session: true,
            keep_alive_secs: default_keep_alive(),
            subscriptions: Vec::new(),
            protocol_version: MqttVersion::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamSpec {
    pub kind: StreamKind,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub limits: StreamLimits,
    #[serde(default)]
    pub ws: WsOptions,
    #[serde(default)]
    pub sse: SseOptions,
    #[serde(default)]
    pub mqtt: MqttOptions,
}

impl StreamSpec {
    pub fn new(kind: StreamKind, url: impl Into<String>) -> Self {
        StreamSpec {
            kind,
            url: url.into(),
            ..StreamSpec::default()
        }
    }
}

/// The connection a saved request describes, ready to open. Which options apply
/// is decided by the request's kind, so the protocol is never written twice; the
/// url, headers and auth travel from the request as they are, and the two
/// headers the SSE transport owns are lifted out so they are not sent twice.
pub fn spec_for(
    request: &crate::collection::SavedRequest,
    vars: BTreeMap<String, String>,
) -> CoreResult<StreamSpec> {
    let kind = crate::collection::stream_kind(&request.kind).ok_or_else(|| {
        CoreError::Unsupported(format!(
            "a {} request is not a stream — send it with `mandalo send` instead",
            request.kind
        ))
    })?;
    let stream = request.stream.clone().unwrap_or_default();
    let mut headers = request.headers.clone();
    let mut sse = SseOptions::default();
    if kind == StreamKind::Sse {
        headers.retain(|(k, v)| {
            let owned_by_the_transport = k.eq_ignore_ascii_case("last-event-id")
                || (k.eq_ignore_ascii_case("accept")
                    && crate::http_format::accepts_event_stream(v));
            !owned_by_the_transport
        });
        sse.last_event_id = stream.last_event_id.clone();
        sse.auto_reconnect = stream.auto_reconnect.unwrap_or(true);
    }
    let defaults = MqttOptions::default();
    Ok(StreamSpec {
        kind,
        url: request.url.clone(),
        headers,
        auth: request.auth.clone(),
        vars,
        limits: StreamLimits::default(),
        ws: WsOptions {
            subprotocols: stream.subprotocols.clone(),
            auto_reconnect: stream.auto_reconnect.unwrap_or(false),
            ping_interval_ms: stream.ping_interval_ms.unwrap_or(0),
        },
        sse,
        mqtt: MqttOptions {
            client_id: stream.client_id.clone(),
            username: stream.username.clone(),
            password: stream.password.clone(),
            clean_session: stream.clean_session.unwrap_or(defaults.clean_session),
            keep_alive_secs: stream.keep_alive_secs.unwrap_or(defaults.keep_alive_secs),
            subscriptions: stream.subscriptions.clone(),
            protocol_version: stream.protocol_version.unwrap_or_default(),
        },
    })
}

/// The named message a saved stream sends, with every `{{var}}` resolved — the
/// topic included, because a topic that still holds a template publishes nowhere.
pub fn outgoing_for(
    stream: &crate::collection::SavedStream,
    name: &str,
    vars: &BTreeMap<String, String>,
) -> CoreResult<Outgoing> {
    let saved = stream.message(name)?;
    let apply = |value: &str| interpolate::apply(value, vars);
    Ok(match &saved.message {
        Outgoing::Text { text } => Outgoing::text(apply(text)?),
        Outgoing::Publish {
            topic,
            payload,
            qos,
            retain,
        } => Outgoing::Publish {
            topic: apply(topic)?,
            payload: apply(payload)?,
            qos: *qos,
            retain: *retain,
        },
        Outgoing::Subscribe { topic, qos } => Outgoing::Subscribe {
            topic: apply(topic)?,
            qos: *qos,
        },
        Outgoing::Unsubscribe { topic } => Outgoing::Unsubscribe {
            topic: apply(topic)?,
        },
        other => other.clone(),
    })
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Payload {
    Text { text: String },
    Binary { base64: String, bytes: usize },
}

impl Payload {
    pub fn text(text: impl Into<String>) -> Self {
        Payload::Text { text: text.into() }
    }

    pub fn binary(bytes: &[u8]) -> Self {
        Payload::Binary {
            base64: BASE64.encode(bytes),
            bytes: bytes.len(),
        }
    }

    /// Bytes stay bytes; text is only produced when the payload really is UTF-8,
    /// so a binary frame can never be silently mangled into replacement chars.
    pub fn detect(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Payload::text(text),
            Err(_) => Payload::binary(bytes),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Payload::Text { text } => Some(text),
            Payload::Binary { .. } => None,
        }
    }

    pub fn size_bytes(&self) -> usize {
        match self {
            Payload::Text { text } => text.len(),
            Payload::Binary { bytes, .. } => *bytes,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

impl MessageMeta {
    fn frame(name: &str) -> Self {
        MessageMeta {
            frame: Some(name.to_string()),
            ..MessageMeta::default()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_present: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamEvent {
    Connecting {
        at: u64,
        url: String,
    },
    Connected {
        at: u64,
        info: ConnectionInfo,
    },
    Message {
        at: u64,
        direction: Direction,
        payload: Payload,
        meta: MessageMeta,
    },
    Reconnecting {
        at: u64,
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    Dropped {
        at: u64,
        count: u64,
        reason: String,
    },
    Disconnected {
        at: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<u16>,
        reason: String,
    },
    Error {
        at: u64,
        message: String,
        code: String,
    },
}

impl StreamEvent {
    pub fn at(&self) -> u64 {
        match self {
            StreamEvent::Connecting { at, .. }
            | StreamEvent::Connected { at, .. }
            | StreamEvent::Message { at, .. }
            | StreamEvent::Reconnecting { at, .. }
            | StreamEvent::Dropped { at, .. }
            | StreamEvent::Disconnected { at, .. }
            | StreamEvent::Error { at, .. } => *at,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, StreamEvent::Disconnected { .. })
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outgoing {
    Text {
        text: String,
    },
    Binary {
        base64: String,
    },
    Publish {
        topic: String,
        payload: String,
        #[serde(default)]
        qos: u8,
        #[serde(default)]
        retain: bool,
    },
    Subscribe {
        topic: String,
        #[serde(default)]
        qos: u8,
    },
    Unsubscribe {
        topic: String,
    },
    Ping,
}

impl Outgoing {
    pub fn text(text: impl Into<String>) -> Self {
        Outgoing::Text { text: text.into() }
    }

    fn label(&self) -> &'static str {
        match self {
            Outgoing::Text { .. } => "a text message",
            Outgoing::Binary { .. } => "a binary message",
            Outgoing::Publish { .. } => "a publish",
            Outgoing::Subscribe { .. } => "a subscribe",
            Outgoing::Unsubscribe { .. } => "an unsubscribe",
            Outgoing::Ping => "a ping",
        }
    }

    fn binary_bytes(base64: &str) -> CoreResult<Vec<u8>> {
        BASE64
            .decode(base64)
            .map_err(|e| CoreError::Stream(format!("binary payload is not valid base64: {e}")))
    }
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

pub(crate) enum Command {
    Send(Outgoing, oneshot::Sender<CoreResult<()>>),
    Close,
}

/// Every event a protocol task produces goes through here, so the buffer cap is
/// enforced in exactly one place and a full channel becomes a visible
/// `Dropped` event instead of unbounded memory or a stalled reader.
pub(crate) struct Emitter {
    tx: mpsc::Sender<StreamEvent>,
    parked: VecDeque<usize>,
    parked_bytes: usize,
    budget: usize,
    dropped: u64,
    alive: bool,
}

/// What one event costs the buffer: the strings it owns plus the fixed weight of
/// the enum itself, so a flood of tiny events is bounded too.
fn event_bytes(event: &StreamEvent) -> usize {
    const BASE: usize = 128;
    let owned = match event {
        StreamEvent::Connecting { url, .. } => url.len(),
        StreamEvent::Connected { info, .. } => {
            info.url.len()
                + info.protocol.as_ref().map(String::len).unwrap_or_default()
                + info
                    .headers
                    .iter()
                    .map(|(k, v)| k.len() + v.len())
                    .sum::<usize>()
        }
        StreamEvent::Message { payload, meta, .. } => {
            let body = match payload {
                Payload::Text { text } => text.len(),
                Payload::Binary { base64, .. } => base64.len(),
            };
            body + meta.event.as_ref().map(String::len).unwrap_or_default()
                + meta.id.as_ref().map(String::len).unwrap_or_default()
                + meta.topic.as_ref().map(String::len).unwrap_or_default()
                + meta.frame.as_ref().map(String::len).unwrap_or_default()
        }
        StreamEvent::Reconnecting { reason, .. } => reason.len(),
        StreamEvent::Dropped { reason, .. } => reason.len(),
        StreamEvent::Disconnected { reason, .. } => reason.len(),
        StreamEvent::Error { message, code, .. } => message.len() + code.len(),
    };
    BASE + owned
}

impl Emitter {
    fn new(tx: mpsc::Sender<StreamEvent>, budget: usize) -> Self {
        Emitter {
            tx,
            parked: VecDeque::new(),
            parked_bytes: 0,
            budget: budget.max(1),
            dropped: 0,
            alive: true,
        }
    }

    /// The channel is FIFO, so the number of events still in it says how many of
    /// the sizes recorded here have already been read and can stop counting.
    fn settle(&mut self) {
        let still_in = self
            .tx
            .max_capacity()
            .saturating_sub(self.tx.capacity())
            .min(self.parked.len());
        while self.parked.len() > still_in {
            let size = self.parked.pop_front().unwrap_or_default();
            self.parked_bytes = self.parked_bytes.saturating_sub(size);
        }
    }

    fn record(&mut self, size: usize) {
        self.parked.push_back(size);
        self.parked_bytes += size;
    }

    /// The drop notice is the only thing that makes a bounded buffer honest, so
    /// it goes out even when the budget is spent.
    fn report_drops(&mut self) -> bool {
        if self.dropped == 0 {
            return true;
        }
        let notice = StreamEvent::Dropped {
            at: now_millis(),
            count: self.dropped,
            reason: "the event buffer was full".to_string(),
        };
        let size = event_bytes(&notice);
        match self.tx.try_send(notice) {
            Ok(()) => {
                self.dropped = 0;
                self.record(size);
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.alive = false;
                false
            }
        }
    }

    /// `false` means the consumer is gone and the protocol task must stop —
    /// a closed window cannot leave a socket running forever.
    pub(crate) fn emit(&mut self, event: StreamEvent) -> bool {
        if !self.alive {
            return false;
        }
        self.settle();
        if !self.report_drops() {
            return false;
        }
        let size = event_bytes(&event);
        if self.parked_bytes + size > self.budget {
            self.dropped += 1;
            return true;
        }
        match self.tx.try_send(event) {
            Ok(()) => {
                self.record(size);
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped += 1;
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.alive = false;
                false
            }
        }
    }

    pub(crate) fn connecting(&mut self, url: &str) -> bool {
        self.emit(StreamEvent::Connecting {
            at: now_millis(),
            url: crate::redact::scrub(url),
        })
    }

    pub(crate) fn connected(&mut self, info: ConnectionInfo) -> bool {
        self.emit(StreamEvent::Connected {
            at: now_millis(),
            info: ConnectionInfo {
                url: crate::redact::scrub(&info.url),
                ..info
            },
        })
    }

    pub(crate) fn message(
        &mut self,
        direction: Direction,
        payload: Payload,
        meta: MessageMeta,
    ) -> bool {
        self.emit(StreamEvent::Message {
            at: now_millis(),
            direction,
            payload,
            meta,
        })
    }

    pub(crate) fn reconnecting(&mut self, attempt: u32, delay_ms: u64, reason: &str) -> bool {
        self.emit(StreamEvent::Reconnecting {
            at: now_millis(),
            attempt,
            delay_ms,
            reason: reason.to_string(),
        })
    }

    pub(crate) fn error(&mut self, message: impl Into<String>, code: &str) -> bool {
        self.emit(StreamEvent::Error {
            at: now_millis(),
            message: crate::redact::scrub(&message.into()),
            code: code.to_string(),
        })
    }

    /// A consumer that stops reading must not pin this task forever, so waiting
    /// for room to report the close is bounded.
    async fn send_final(&mut self, event: StreamEvent) -> bool {
        let size = event_bytes(&event);
        match tokio::time::timeout(FINAL_EVENT_WAIT, self.tx.send(event)).await {
            Ok(Ok(())) => {
                self.record(size);
                true
            }
            Ok(Err(_)) | Err(_) => {
                self.alive = false;
                false
            }
        }
    }

    /// The last event of a stream waits for room instead of being dropped, and
    /// takes the pending drop count with it — otherwise a burst that overflows
    /// the buffer and then goes quiet would never be reported at all.
    pub(crate) async fn disconnected(
        &mut self,
        code: Option<u16>,
        reason: impl Into<String>,
    ) -> bool {
        if !self.alive {
            return false;
        }
        if self.dropped > 0 {
            let notice = StreamEvent::Dropped {
                at: now_millis(),
                count: self.dropped,
                reason: "the event buffer was full".to_string(),
            };
            if !self.send_final(notice).await {
                return false;
            }
            self.dropped = 0;
        }
        let closed = StreamEvent::Disconnected {
            at: now_millis(),
            code,
            reason: crate::redact::scrub(&reason.into()),
        };
        self.send_final(closed).await
    }
}

#[derive(Debug)]
pub struct StreamHandle {
    id: String,
    kind: StreamKind,
    url: String,
    commands: mpsc::Sender<Command>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StreamHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind(&self) -> StreamKind {
        self.kind
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn is_open(&self) -> bool {
        self.task
            .lock()
            .expect("stream task lock")
            .as_ref()
            .is_some_and(|t| !t.is_finished())
    }

    pub fn status(&self) -> StreamStatus {
        StreamStatus {
            id: self.id.clone(),
            kind: self.kind,
            url: crate::redact::scrub(&self.url),
            open: self.is_open(),
        }
    }

    pub async fn send(&self, message: Outgoing) -> CoreResult<()> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(Command::Send(message, reply))
            .await
            .map_err(|_| CoreError::Stream(format!("stream {} is already closed", self.id)))?;
        answer.await.map_err(|_| {
            CoreError::Stream(format!("stream {} closed before it could send", self.id))
        })?
    }

    pub async fn close(&self) -> CoreResult<()> {
        let _ = self.commands.send(Command::Close).await;
        let task = self.task.lock().expect("stream task lock").take();
        if let Some(task) = task {
            let handle = task.abort_handle();
            if tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .is_err()
            {
                handle.abort();
            }
        }
        Ok(())
    }
}

/// A dropped handle must not leave a socket and a task behind, so the task dies
/// with it. Callers that want a graceful close call `close()` first.
impl Drop for StreamHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().expect("stream task lock").take() {
            task.abort();
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub id: String,
    pub kind: StreamKind,
    pub url: String,
    pub open: bool,
}

#[derive(Default)]
pub struct StreamRegistry {
    streams: Mutex<HashMap<String, Arc<StreamHandle>>>,
}

impl StreamRegistry {
    pub fn new() -> Self {
        StreamRegistry::default()
    }

    /// Only `close` takes a stream out by name, so a server that hangs up would
    /// otherwise leave its entry here for the life of the app.
    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<StreamHandle>>> {
        let mut streams = self.streams.lock().expect("stream registry lock");
        streams.retain(|_, handle| handle.is_open());
        streams
    }

    pub fn insert(&self, handle: StreamHandle) -> String {
        let id = handle.id.clone();
        self.live().insert(id.clone(), Arc::new(handle));
        id
    }

    pub fn len(&self) -> usize {
        self.live().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ids(&self) -> Vec<String> {
        self.live().keys().cloned().collect()
    }

    fn get(&self, id: &str) -> CoreResult<Arc<StreamHandle>> {
        self.live()
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(format!("there is no open stream {id}")))
    }

    pub fn status(&self, id: &str) -> CoreResult<StreamStatus> {
        Ok(self.get(id)?.status())
    }

    pub async fn send(&self, id: &str, message: Outgoing) -> CoreResult<()> {
        self.get(id)?.send(message).await
    }

    pub async fn close(&self, id: &str) -> CoreResult<()> {
        let handle = {
            let mut streams = self.streams.lock().expect("stream registry lock");
            streams
                .remove(id)
                .ok_or_else(|| CoreError::NotFound(format!("there is no open stream {id}")))?
        };
        handle.close().await
    }

    pub async fn close_all(&self) {
        let handles: Vec<Arc<StreamHandle>> = {
            let mut streams = self.streams.lock().expect("stream registry lock");
            streams.drain().map(|(_, h)| h).collect()
        };
        for handle in handles {
            let _ = handle.close().await;
        }
    }
}

/// The url, headers, auth and protocol options with every `{{var}}` already
/// resolved. Nothing downstream of this ever sees a template.
pub(crate) struct Resolved {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) limits: StreamLimits,
    pub(crate) policy: Arc<dyn HostPolicy>,
}

impl Resolved {
    /// Every connect attempt asks again: the answer can change between the
    /// first socket and the tenth, and a reconnect is a fresh egress.
    pub(crate) async fn guard(&self, url: &str) -> CoreResult<Vec<SocketAddr>> {
        let approved = guard_host(self.policy.as_ref(), url).await?;
        Ok(sockets(url, &approved))
    }
}

/// The addresses the policy approved, paired with the port to connect to. An
/// empty list means the policy does not enforce and nothing needs pinning.
fn sockets(url: &str, approved: &[IpAddr]) -> Vec<SocketAddr> {
    let port = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.port().or_else(|| default_port(u.scheme())))
        .unwrap_or(443);
    approved
        .iter()
        .map(|ip| SocketAddr::new(*ip, port))
        .collect()
}

/// The authority a redirect must keep for the request headers to travel with it.
pub(crate) fn authority(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port().or_else(|| default_port(parsed.scheme()));
    Some(format!(
        "{}://{host}:{}",
        parsed.scheme(),
        port.unwrap_or_default()
    ))
}

pub(crate) fn scheme_of(url: &str) -> CoreResult<String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Stream(format!("invalid url {url:?}: {e}")))?;
    Ok(parsed.scheme().to_ascii_lowercase())
}

fn resolve_pairs(
    pairs: &[(String, String)],
    vars: &BTreeMap<String, String>,
) -> CoreResult<Vec<(String, String)>> {
    pairs
        .iter()
        .map(|(k, v)| Ok((interpolate::apply(k, vars)?, interpolate::apply(v, vars)?)))
        .collect()
}

#[derive(Debug)]
pub(crate) struct AuthApplied {
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) query: Vec<(String, String)>,
}

/// Auth lands exactly where it lands for HTTP: bearer, basic and a header api key
/// become headers, a query api key becomes a query parameter.
pub(crate) fn apply_auth(auth: &Auth, vars: &BTreeMap<String, String>) -> CoreResult<AuthApplied> {
    let mut applied = AuthApplied {
        headers: Vec::new(),
        query: Vec::new(),
    };
    match auth.effective() {
        Auth::None => {}
        Auth::Bearer { token } => applied.headers.push((
            "Authorization".to_string(),
            format!("Bearer {}", interpolate::apply(token, vars)?),
        )),
        Auth::Basic { username, password } => {
            let user = interpolate::apply(username, vars)?;
            let pass = interpolate::apply(password, vars)?;
            applied.headers.push((
                "Authorization".to_string(),
                format!("Basic {}", BASE64.encode(format!("{user}:{pass}"))),
            ));
        }
        Auth::Apikey {
            key,
            value,
            placement,
        } => {
            let k = interpolate::apply(key, vars)?;
            let v = interpolate::apply(value, vars)?;
            match placement.as_str() {
                "header" => applied.headers.push((k, v)),
                "query" => applied.query.push((k, v)),
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "unsupported apikey placement: {other}"
                    )))
                }
            }
        }
        Auth::Inherited { .. } => unreachable!("effective() peels the wrapper off"),
    }
    Ok(applied)
}

fn resolve(spec: &StreamSpec, policy: Arc<dyn HostPolicy>) -> CoreResult<Resolved> {
    let vars = &spec.vars;
    let mut url = interpolate::apply(&spec.url, vars)?;
    let mut headers = resolve_pairs(&spec.headers, vars)?;
    let auth = apply_auth(&spec.auth, vars)?;

    if !auth.headers.is_empty() {
        for (name, _) in &auth.headers {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        }
        headers.extend(auth.headers);
    }
    if !auth.query.is_empty() {
        let mut parsed = reqwest::Url::parse(&url)
            .map_err(|e| CoreError::Stream(format!("invalid url {url:?}: {e}")))?;
        {
            let mut query = parsed.query_pairs_mut();
            for (k, v) in &auth.query {
                query.append_pair(k, v);
            }
        }
        url = parsed.to_string();
    }

    Ok(Resolved {
        url,
        headers,
        limits: spec.limits.sanitized(),
        policy,
    })
}

/// The same rule an HTTP request obeys: a realtime connection is egress too, so
/// the policy decides before a socket is opened, not after.
pub async fn guard_host(policy: &dyn HostPolicy, url: &str) -> CoreResult<Vec<IpAddr>> {
    if !policy.enforces() {
        return Ok(Vec::new());
    }
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Stream(format!("invalid url {url:?}: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| CoreError::Stream(format!("url has no host: {url}")))?
        .to_string();
    let port = parsed
        .port()
        .or_else(|| default_port(parsed.scheme()))
        .unwrap_or(443);

    let addresses: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| CoreError::Network(format!("cannot resolve {host}: {e}")))?
            .map(|addr| addr.ip())
            .collect(),
    };
    if addresses.is_empty() {
        return Err(CoreError::Network(format!("{host} resolved to no address")));
    }
    for ip in &addresses {
        match policy.allow(&host, ip) {
            Decision::Allow => {}
            Decision::Deny(reason) => {
                return Err(CoreError::HostDenied(format!(
                    "{host} is blocked: {reason}"
                )))
            }
            Decision::Ask => {
                return Err(CoreError::HostConfirmRequired(format!(
                    "{host} needs confirmation before it can be called"
                )))
            }
        }
    }
    Ok(addresses)
}

pub(crate) enum Waited {
    Elapsed,
    Closed,
}

/// A reconnect delay that ignores the command channel is a stream that keeps
/// running after the user closed it, so the wait listens while it waits.
pub(crate) async fn wait_before_reconnect(
    delay_ms: u64,
    commands: &mut mpsc::Receiver<Command>,
    cannot_send: &str,
) -> Waited {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(delay_ms);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return Waited::Elapsed,
            command = commands.recv() => match command {
                None | Some(Command::Close) => return Waited::Closed,
                Some(Command::Send(_, reply)) => {
                    let _ = reply.send(Err(CoreError::Stream(cannot_send.to_string())));
                }
            },
        }
    }
}

/// `0` means "no limit", which still has to be a real duration: a sentinel near
/// `u64::MAX` overflows the timer wheel, so a century stands in for never.
pub(crate) fn idle_timeout(millis: u64) -> Duration {
    if millis == 0 {
        Duration::from_secs(60 * 60 * 24 * 365 * 100)
    } else {
        Duration::from_millis(millis)
    }
}

pub(crate) fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ws" | "http" => Some(80),
        "wss" | "https" => Some(443),
        "mqtt" | "tcp" => Some(1883),
        "mqtts" | "ssl" => Some(8883),
        _ => None,
    }
}

/// A bounded event channel sized by the limits the caller chose.
pub fn event_channel(
    limits: &StreamLimits,
) -> (mpsc::Sender<StreamEvent>, mpsc::Receiver<StreamEvent>) {
    mpsc::channel(limits.sanitized().max_buffered_events)
}

pub async fn open(
    spec: StreamSpec,
    policy: Arc<dyn HostPolicy>,
    events: mpsc::Sender<StreamEvent>,
) -> CoreResult<StreamHandle> {
    crate::install_crypto_provider();
    let resolved = resolve(&spec, policy.clone())?;
    let scheme = scheme_of(&resolved.url)?;
    match spec.kind {
        StreamKind::WebSocket if !matches!(scheme.as_str(), "ws" | "wss") => {
            return Err(CoreError::Stream(format!(
                "a websocket url must start with ws:// or wss:// — {scheme}:// is not one"
            )))
        }
        StreamKind::Sse if !matches!(scheme.as_str(), "http" | "https") => {
            return Err(CoreError::Stream(format!(
                "server-sent events are plain HTTP — the url must start with http:// or https://, not {scheme}://"
            )))
        }
        StreamKind::Mqtt if !matches!(scheme.as_str(), "mqtt" | "mqtts" | "ws" | "wss") => {
            return Err(CoreError::Stream(format!(
                "an mqtt url must start with mqtt://, mqtts://, ws:// or wss:// — {scheme}:// is not one"
            )))
        }
        _ => {}
    }

    guard_host(policy.as_ref(), &resolved.url).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let (commands, command_rx) = mpsc::channel(32);
    let emitter = Emitter::new(events, resolved.limits.max_buffered_bytes);
    let url = crate::redact::scrub(&resolved.url);

    let task = match spec.kind {
        StreamKind::WebSocket => {
            let options = spec.ws.clone();
            tokio::spawn(ws::run(resolved, options, command_rx, emitter))
        }
        StreamKind::Sse => {
            let options = spec.sse.clone();
            tokio::spawn(sse::run(resolved, options, command_rx, emitter))
        }
        StreamKind::Mqtt => {
            let options = mqtt::prepare(&spec, &resolved)?;
            tokio::spawn(mqtt::run(resolved, options, command_rx, emitter))
        }
    };

    Ok(StreamHandle {
        id,
        kind: spec.kind,
        url,
        commands,
        task: Mutex::new(Some(task)),
    })
}

/// Ask a protocol task to give up because the payload the caller handed it is
/// not something this protocol can carry.
pub(crate) fn wrong_message(kind: &str, message: &Outgoing, hint: &str) -> CoreError {
    CoreError::Stream(format!("{kind} cannot send {} — {hint}", message.label()))
}

pub(crate) fn too_big(bytes: usize, limit: usize) -> CoreError {
    CoreError::StreamLimit(format!(
        "the message is {bytes} bytes, over the {limit} byte limit for this stream"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_on_the_wire() {
        for (kind, wire) in [
            (StreamKind::WebSocket, "\"websocket\""),
            (StreamKind::Sse, "\"sse\""),
            (StreamKind::Mqtt, "\"mqtt\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), wire);
            assert_eq!(serde_json::from_str::<StreamKind>(wire).unwrap(), kind);
        }
        assert_eq!(
            serde_json::from_str::<StreamKind>("\"ws\"").unwrap(),
            StreamKind::WebSocket
        );
    }

    #[test]
    fn a_partial_options_object_keeps_every_default_it_did_not_mention() {
        let spec: StreamSpec = serde_json::from_value(serde_json::json!({
            "kind": "websocket",
            "url": "wss://x.dev",
            "limits": {"maxBufferedEvents": 8},
            "ws": {"subprotocols": ["chat"]},
            "mqtt": {"subscriptions": [{"topic": "a/#"}]}
        }))
        .unwrap();
        assert_eq!(spec.limits.max_buffered_events, 8);
        assert_eq!(
            spec.limits.max_message_bytes,
            StreamLimits::default().max_message_bytes
        );
        assert_eq!(spec.ws.subprotocols, vec!["chat".to_string()]);
        assert_eq!(spec.ws.ping_interval_ms, 0);
        assert_eq!(spec.mqtt.keep_alive_secs, 60);
        assert_eq!(spec.mqtt.subscriptions[0].qos, 0);
    }

    #[test]
    fn a_spec_needs_only_a_kind_and_a_url() {
        let spec: StreamSpec =
            serde_json::from_value(serde_json::json!({"kind": "sse", "url": "https://x.dev/e"}))
                .unwrap();
        assert_eq!(spec.kind, StreamKind::Sse);
        assert!(spec.sse.auto_reconnect);
        assert!(!spec.ws.auto_reconnect);
        assert_eq!(spec.limits, StreamLimits::default());
        assert_eq!(spec.mqtt.keep_alive_secs, 60);
        assert!(spec.mqtt.clean_session);
    }

    #[test]
    fn events_are_tagged_camel_case() {
        let event = StreamEvent::Message {
            at: 7,
            direction: Direction::Incoming,
            payload: Payload::text("hi"),
            meta: MessageMeta {
                topic: Some("a/b".to_string()),
                qos: Some(1),
                ..MessageMeta::default()
            },
        };
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["type"], "message");
        assert_eq!(wire["direction"], "incoming");
        assert_eq!(wire["payload"]["kind"], "text");
        assert_eq!(wire["payload"]["text"], "hi");
        assert_eq!(wire["meta"]["topic"], "a/b");
        assert_eq!(wire["meta"]["qos"], 1);
        assert!(wire["meta"].get("retain").is_none());
        assert_eq!(serde_json::from_value::<StreamEvent>(wire).unwrap(), event);
    }

    #[test]
    fn binary_payloads_travel_as_base64() {
        let payload = Payload::detect(&[0xff, 0x00, 0xfe]);
        match &payload {
            Payload::Binary { base64, bytes } => {
                assert_eq!(bytes, &3);
                assert_eq!(base64, "/wD+");
            }
            other => panic!("expected binary, got {other:?}"),
        }
        assert!(matches!(Payload::detect(b"plain"), Payload::Text { .. }));
    }

    #[test]
    fn backoff_doubles_and_stops_at_the_ceiling() {
        let limits = StreamLimits {
            backoff_base_ms: 100,
            backoff_max_ms: 800,
            ..StreamLimits::default()
        };
        assert_eq!(limits.backoff(1), 100);
        assert_eq!(limits.backoff(2), 200);
        assert_eq!(limits.backoff(3), 400);
        assert_eq!(limits.backoff(4), 800);
        assert_eq!(limits.backoff(9), 800);
    }

    #[test]
    fn auth_becomes_headers_and_query_exactly_as_it_does_for_http() {
        let vars: BTreeMap<String, String> = [("t".to_string(), "s3cr3t".to_string())]
            .into_iter()
            .collect();
        let bearer = apply_auth(
            &Auth::Bearer {
                token: "{{t}}".to_string(),
            },
            &vars,
        )
        .unwrap();
        assert_eq!(
            bearer.headers,
            vec![("Authorization".to_string(), "Bearer s3cr3t".to_string())]
        );
        let basic = apply_auth(
            &Auth::Basic {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
            &vars,
        )
        .unwrap();
        assert_eq!(basic.headers[0].1, "Basic dXNlcjpwYXNz");
        let query = apply_auth(
            &Auth::Apikey {
                key: "api_key".to_string(),
                value: "abc".to_string(),
                placement: "query".to_string(),
            },
            &vars,
        )
        .unwrap();
        assert!(query.headers.is_empty());
        assert_eq!(
            query.query,
            vec![("api_key".to_string(), "abc".to_string())]
        );
        let err = apply_auth(
            &Auth::Apikey {
                key: "k".to_string(),
                value: "v".to_string(),
                placement: "cookie".to_string(),
            },
            &vars,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unsupported apikey placement"));
    }

    #[test]
    fn resolving_interpolates_url_and_headers_and_appends_a_query_key() {
        let mut spec = StreamSpec::new(StreamKind::WebSocket, "wss://{{host}}/socket");
        spec.headers = vec![("X-{{tag}}".to_string(), "{{tag}}-1".to_string())];
        spec.auth = Auth::Apikey {
            key: "api_key".to_string(),
            value: "{{tag}}".to_string(),
            placement: "query".to_string(),
        };
        spec.vars = [("host", "x.dev"), ("tag", "trace")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let resolved = resolve(&spec, Arc::new(crate::AllowAll)).unwrap();
        assert_eq!(resolved.url, "wss://x.dev/socket?api_key=trace");
        assert_eq!(
            resolved.headers,
            vec![("X-trace".to_string(), "trace-1".to_string())]
        );
    }

    #[test]
    fn an_unresolved_variable_fails_before_anything_connects() {
        let spec = StreamSpec::new(StreamKind::WebSocket, "wss://{{host}}/socket");
        let err = resolve(&spec, Arc::new(crate::AllowAll))
            .err()
            .expect("an unresolved variable must fail");
        assert_eq!(err.code(), "E_UNRESOLVED_VAR");
    }

    #[test]
    fn auth_replaces_a_hand_written_authorization_header() {
        let mut spec = StreamSpec::new(StreamKind::WebSocket, "wss://x.dev");
        spec.headers = vec![("authorization".to_string(), "stale".to_string())];
        spec.auth = Auth::Bearer {
            token: "fresh".to_string(),
        };
        let resolved = resolve(&spec, Arc::new(crate::AllowAll)).unwrap();
        assert_eq!(
            resolved.headers,
            vec![("Authorization".to_string(), "Bearer fresh".to_string())]
        );
    }

    #[tokio::test]
    async fn a_wrong_scheme_fails_loud_per_kind() {
        for (kind, url, needle) in [
            (StreamKind::WebSocket, "https://x.dev", "ws:// or wss://"),
            (StreamKind::Sse, "ws://x.dev", "http:// or https://"),
            (StreamKind::Mqtt, "https://x.dev", "mqtt://"),
        ] {
            let (tx, _rx) = mpsc::channel(4);
            let err = open(StreamSpec::new(kind, url), Arc::new(crate::AllowAll), tx)
                .await
                .unwrap_err();
            assert!(err.to_string().contains(needle), "{kind:?}: {err}");
        }
    }

    #[tokio::test]
    async fn the_emitter_reports_what_it_had_to_drop() {
        let (tx, mut rx) = mpsc::channel(2);
        let mut emitter = Emitter::new(tx, usize::MAX);
        for i in 0..6 {
            emitter.message(
                Direction::Incoming,
                Payload::text(i.to_string()),
                MessageMeta::default(),
            );
        }
        let mut seen = Vec::new();
        while let Ok(event) = rx.try_recv() {
            seen.push(event);
        }
        emitter.message(
            Direction::Incoming,
            Payload::text("after"),
            MessageMeta::default(),
        );
        while let Ok(event) = rx.try_recv() {
            seen.push(event);
        }
        let dropped: Vec<u64> = seen
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Dropped { count, .. } => Some(*count),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, vec![4]);
    }

    #[tokio::test]
    async fn the_emitter_stops_when_the_consumer_is_gone() {
        let (tx, rx) = mpsc::channel(4);
        let mut emitter = Emitter::new(tx, usize::MAX);
        drop(rx);
        assert!(!emitter.connecting("wss://x.dev"));
    }

    #[tokio::test]
    async fn a_slot_count_is_not_a_memory_budget_so_bytes_are_capped_too() {
        let budget = 4 * 1024 * 1024;
        let (tx, mut rx) = mpsc::channel(1024);
        let mut emitter = Emitter::new(tx, budget);
        let payload = "x".repeat(1024 * 1024);
        for _ in 0..32 {
            emitter.message(
                Direction::Incoming,
                Payload::text(payload.clone()),
                MessageMeta::default(),
            );
        }
        emitter.disconnected(None, "done").await;

        let mut parked = 0usize;
        let mut dropped = 0u64;
        let mut delivered = 0u64;
        while let Ok(event) = rx.try_recv() {
            parked += event_bytes(&event);
            match event {
                StreamEvent::Dropped { count, .. } => dropped += count,
                StreamEvent::Message { .. } => delivered += 1,
                _ => {}
            }
        }
        assert_eq!(delivered, 3, "a 4 MiB budget holds three 1 MiB messages");
        assert_eq!(
            dropped, 29,
            "every message it could not hold must be counted"
        );
        assert!(
            parked <= budget,
            "{parked} bytes were parked, over the {budget} byte budget"
        );
    }

    #[test]
    fn hostile_limits_are_pulled_back_into_range_instead_of_panicking() {
        let limits: StreamLimits = serde_json::from_value(serde_json::json!({
            "maxMessageBytes": usize::MAX,
            "maxBufferedEvents": usize::MAX,
            "maxBufferedBytes": usize::MAX,
            "maxReconnectAttempts": u32::MAX,
            "connectTimeoutMs": u64::MAX,
            "idleTimeoutMs": u64::MAX,
            "backoffBaseMs": u64::MAX,
            "backoffMaxMs": 0,
        }))
        .unwrap();
        let safe = limits.sanitized();
        assert_eq!(safe.max_message_bytes, MESSAGE_BYTES_CEILING);
        assert_eq!(safe.max_buffered_events, BUFFERED_EVENTS_CEILING);
        assert_eq!(safe.max_buffered_bytes, BUFFERED_BYTES_CEILING);
        assert_eq!(safe.max_reconnect_attempts, RECONNECT_ATTEMPTS_CEILING);
        assert_eq!(safe.connect_timeout_ms, CONNECT_TIMEOUT_CEILING_MS);
        assert_eq!(safe.idle_timeout_ms, IDLE_TIMEOUT_CEILING_MS);
        assert_eq!(safe.backoff_base_ms, BACKOFF_CEILING_MS);
        assert!(safe.backoff_max_ms >= safe.backoff_base_ms);

        let (tx, _rx) = event_channel(&limits);
        assert_eq!(tx.max_capacity(), BUFFERED_EVENTS_CEILING);

        let zeroed = StreamLimits {
            max_message_bytes: 0,
            max_buffered_events: 0,
            max_buffered_bytes: 0,
            connect_timeout_ms: 0,
            backoff_base_ms: 0,
            ..StreamLimits::default()
        }
        .sanitized();
        assert_eq!(zeroed.max_buffered_events, 1);
        assert_eq!(zeroed.max_message_bytes, 1);
        assert!(zeroed.max_buffered_bytes >= zeroed.max_message_bytes);
        assert_eq!(zeroed.connect_timeout_ms, 1);
        assert_eq!(zeroed.backoff_base_ms, 1);
    }

    #[tokio::test]
    async fn a_url_with_a_secret_in_it_is_redacted_everywhere_it_is_shown() {
        crate::redact::register("test", "stream_key", "sup3rs3cr3tvalue");
        let url = "wss://x.dev/socket?api_key=sup3rs3cr3tvalue";
        let (tx, mut rx) = mpsc::channel(8);
        let mut emitter = Emitter::new(tx, usize::MAX);
        emitter.connecting(url);
        emitter.connected(ConnectionInfo {
            url: url.to_string(),
            ..ConnectionInfo::default()
        });

        let shown = match rx.try_recv().unwrap() {
            StreamEvent::Connecting { url, .. } => url,
            other => panic!("expected connecting, got {other:?}"),
        };
        assert!(!shown.contains("sup3rs3cr3tvalue"), "{shown}");
        let shown = match rx.try_recv().unwrap() {
            StreamEvent::Connected { info, .. } => info.url,
            other => panic!("expected connected, got {other:?}"),
        };
        assert!(!shown.contains("sup3rs3cr3tvalue"), "{shown}");

        let handle = StreamHandle {
            id: "s1".to_string(),
            kind: StreamKind::WebSocket,
            url: url.to_string(),
            commands: mpsc::channel(1).0,
            task: Mutex::new(None),
        };
        assert!(
            !handle.status().url.contains("sup3rs3cr3tvalue"),
            "{}",
            handle.status().url
        );
    }

    #[tokio::test]
    async fn the_registry_forgets_a_stream_the_server_ended() {
        let registry = StreamRegistry::new();
        let task = tokio::spawn(async {});
        let id = registry.insert(StreamHandle {
            id: "gone".to_string(),
            kind: StreamKind::Sse,
            url: "https://x.dev/events".to_string(),
            commands: mpsc::channel(1).0,
            task: Mutex::new(Some(task)),
        });
        while registry.get(&id).map(|h| h.is_open()).unwrap_or_default() {
            tokio::task::yield_now().await;
        }
        assert_eq!(registry.len(), 0);
        assert!(registry.ids().is_empty());
        assert_eq!(registry.status(&id).unwrap_err().code(), "E_NOT_FOUND");
    }
}
