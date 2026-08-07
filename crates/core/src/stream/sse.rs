use super::{
    authority, idle_timeout, too_big, wait_before_reconnect, Command, ConnectionInfo, Direction,
    Emitter, MessageMeta, Payload, Resolved, SseOptions, StreamLimits, Waited, E_STREAM_AUTH,
    E_STREAM_CONNECT, E_STREAM_IDLE, E_STREAM_LIMIT, E_STREAM_PROTOCOL, E_STREAM_TLS,
};
use crate::error::{CoreError, CoreResult};
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

/// Long enough for the redirects a real endpoint uses, short enough that a
/// server cannot walk the client around forever.
const MAX_REDIRECTS: usize = 10;

const NOTHING_TO_SEND: &str = "server-sent events only travel from the server to the client — there is nothing to send on this stream";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

/// The `text/event-stream` wire format, incrementally: `event:`, multi-line
/// `data:` joined with newlines, `id:`, `retry:`, `:` comments, and a blank line
/// as the only thing that dispatches an event.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    data: String,
    event: Option<String>,
    id: Option<String>,
    last_event_id: Option<String>,
    retry_ms: Option<u64>,
    started: bool,
}

impl SseParser {
    pub fn new() -> Self {
        SseParser::default()
    }

    pub fn retry_ms(&self) -> Option<u64> {
        self.retry_ms
    }

    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    pub fn feed(&mut self, bytes: &[u8], max_bytes: usize) -> CoreResult<Vec<SseFrame>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > max_bytes {
            return Err(too_big(self.buffer.len(), max_bytes));
        }
        let mut frames = Vec::new();
        while let Some(line) = self.take_line() {
            if let Some(frame) = self.line(&line, max_bytes)? {
                frames.push(frame);
            }
        }
        Ok(frames)
    }

    /// A trailing `\r` is held back: it may still turn out to be the first half
    /// of a `\r\n` that arrives in the next chunk.
    fn take_line(&mut self) -> Option<String> {
        let position = self
            .buffer
            .iter()
            .position(|b| *b == b'\n' || *b == b'\r')?;
        if self.buffer[position] == b'\r' && position + 1 == self.buffer.len() {
            return None;
        }
        let skip =
            if self.buffer[position] == b'\r' && self.buffer.get(position + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        let line = String::from_utf8_lossy(&self.buffer[..position]).into_owned();
        self.buffer.drain(..position + skip);
        Some(line)
    }

    fn line(&mut self, line: &str, max_bytes: usize) -> CoreResult<Option<SseFrame>> {
        let line = if !self.started {
            self.started = true;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
                if self.data.len() > max_bytes {
                    return Err(too_big(self.data.len(), max_bytes));
                }
            }
            "id" if !value.contains('\0') => {
                self.id = Some(value.to_string());
                self.last_event_id = Some(value.to_string());
            }
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.retry_ms = Some(ms);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseFrame> {
        let event = self.event.take();
        let id = self.id.take();
        if self.data.is_empty() {
            return None;
        }
        let mut data = std::mem::take(&mut self.data);
        data.pop();
        Some(SseFrame {
            event,
            data,
            id: id.or_else(|| self.last_event_id.clone()),
        })
    }
}

fn explain(url: &str, error: &reqwest::Error) -> (String, &'static str) {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string());
    if error.is_timeout() {
        return (format!("{host} did not answer in time"), E_STREAM_CONNECT);
    }
    if error.is_connect() {
        let detail = error.to_string();
        let tls =
            detail.contains("certificate") || detail.contains("tls") || detail.contains("TLS");
        return if tls {
            (
                format!("the TLS handshake with {host} failed: {detail} — check the certificate, or use http:// if the server is not using TLS"),
                E_STREAM_TLS,
            )
        } else {
            (
                format!("cannot reach {host}: {detail} — check the host, the port and that the server is running"),
                E_STREAM_CONNECT,
            )
        };
    }
    (
        format!("the event stream from {host} failed: {error}"),
        E_STREAM_PROTOCOL,
    )
}

enum Attempt {
    Done,
    Consumer,
    Lost(String),
    Fatal(String, &'static str),
}

/// The server suggests the reconnect interval, it does not get to choose it:
/// `retry: 0` would turn this client into the server's flooder.
fn reconnect_delay(limits: &StreamLimits, retry_ms: Option<u64>, attempt: u32) -> u64 {
    match retry_ms {
        Some(ms) => ms.clamp(
            limits.backoff_base_ms,
            limits.backoff_max_ms.max(limits.backoff_base_ms),
        ),
        None => limits.backoff(attempt),
    }
}

pub(crate) async fn run(
    resolved: Resolved,
    options: SseOptions,
    mut commands: mpsc::Receiver<Command>,
    mut events: Emitter,
) {
    let limits = resolved.limits;
    let mut last_event_id = options.last_event_id.clone();
    let mut retry_ms: Option<u64> = None;
    let mut attempt: u32 = 0;

    loop {
        if !events.connecting(&resolved.url) {
            return;
        }
        let up_since = std::time::Instant::now();
        match once(
            &resolved,
            &limits,
            &mut commands,
            &mut events,
            &mut last_event_id,
            &mut retry_ms,
        )
        .await
        {
            Attempt::Done | Attempt::Consumer => return,
            Attempt::Fatal(reason, code) => {
                events.error(reason.clone(), code);
                events.disconnected(None, reason).await;
                return;
            }
            Attempt::Lost(reason) => {
                if up_since.elapsed() >= super::HEALTHY_SESSION {
                    attempt = 0;
                }
                if !options.auto_reconnect || attempt >= limits.max_reconnect_attempts {
                    let reason = if options.auto_reconnect {
                        format!(
                            "{reason} — gave up after {} attempts",
                            limits.max_reconnect_attempts
                        )
                    } else {
                        reason
                    };
                    events.disconnected(None, reason).await;
                    return;
                }
                attempt += 1;
                let delay = reconnect_delay(&limits, retry_ms, attempt);
                if !events.reconnecting(attempt, delay, &reason) {
                    return;
                }
                if let Waited::Closed =
                    wait_before_reconnect(delay, &mut commands, NOTHING_TO_SEND).await
                {
                    events.disconnected(None, "closed by the client").await;
                    return;
                }
            }
        }
    }
}

/// One client per hop: redirects are followed by hand so the host policy sees
/// every one of them, and the approved addresses are pinned so the name cannot
/// be re-resolved to somewhere else between the check and the socket.
fn client(pinned: &[SocketAddr], host: &str) -> CoreResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60 * 60 * 24));
    if !pinned.is_empty() {
        builder = builder.resolve_to_addrs(host, pinned);
    }
    builder
        .build()
        .map_err(|e| CoreError::Network(e.to_string()))
}

fn host_of(url: &str) -> CoreResult<String> {
    reqwest::Url::parse(url)
        .map_err(|e| CoreError::Stream(format!("invalid url {url:?}: {e}")))?
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::Stream(format!("url has no host: {url}")))
}

async fn once(
    resolved: &Resolved,
    limits: &StreamLimits,
    commands: &mut mpsc::Receiver<Command>,
    events: &mut Emitter,
    last_event_id: &mut Option<String>,
    retry_ms: &mut Option<u64>,
) -> Attempt {
    let mut url = resolved.url.clone();
    let mut headers = resolved.headers.clone();
    let mut resume = last_event_id.clone();
    let mut hops = 0usize;

    let response = loop {
        let pinned = match resolved.guard(&url).await {
            Ok(pinned) => pinned,
            Err(e) => return Attempt::Fatal(e.to_string(), E_STREAM_CONNECT),
        };
        let host = match host_of(&url) {
            Ok(host) => host,
            Err(e) => return Attempt::Fatal(e.to_string(), E_STREAM_CONNECT),
        };
        let client = match client(&pinned, &host) {
            Ok(client) => client,
            Err(e) => return Attempt::Fatal(e.to_string(), E_STREAM_CONNECT),
        };
        let mut request = client
            .get(&url)
            .timeout(Duration::from_secs(60 * 60 * 24))
            .header("Accept", "text/event-stream")
            .header("Cache-Control", "no-store");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        // The server decides where to resume from, so the id it last sent goes back
        // on every reconnect — without it a reconnect silently loses events.
        if let Some(id) = resume.as_deref() {
            request = request.header("Last-Event-ID", id);
        }

        let response = match tokio::time::timeout(
            Duration::from_millis(limits.connect_timeout_ms.max(1)),
            request.send(),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let (message, code) = explain(&url, &error);
                events.error(message.clone(), code);
                return Attempt::Lost(message);
            }
            Err(_) => {
                let message = format!(
                    "{url} did not answer within {} ms",
                    limits.connect_timeout_ms
                );
                events.error(message.clone(), E_STREAM_CONNECT);
                return Attempt::Lost(message);
            }
        };

        if !response.status().is_redirection() {
            break response;
        }
        let Some(location) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
        else {
            return Attempt::Fatal(
                format!("{url} answered a redirect with no location to go to"),
                E_STREAM_PROTOCOL,
            );
        };
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Attempt::Fatal(
                format!(
                    "{url} redirected more than {MAX_REDIRECTS} times — the redirects never end"
                ),
                E_STREAM_PROTOCOL,
            );
        }
        let next = match reqwest::Url::parse(&url).and_then(|base| base.join(&location)) {
            Ok(next) => next.to_string(),
            Err(e) => {
                return Attempt::Fatal(
                    format!("{url} redirected to {location:?}, which is not a url: {e}"),
                    E_STREAM_PROTOCOL,
                )
            }
        };
        match super::scheme_of(&next) {
            Ok(scheme) if matches!(scheme.as_str(), "http" | "https") => {}
            _ => {
                return Attempt::Fatal(
                    format!("{url} redirected to {next}, which is not an http url"),
                    E_STREAM_PROTOCOL,
                )
            }
        }
        if authority(&next) != authority(&url) {
            headers.clear();
            resume = None;
        }
        url = next;
    };

    let status = response.status();
    if !status.is_success() {
        let code = if status == 401 || status == 403 {
            E_STREAM_AUTH
        } else {
            E_STREAM_CONNECT
        };
        let message = if code == E_STREAM_AUTH {
            format!("{} rejected the credentials with HTTP {status}", url)
        } else {
            format!("{} answered HTTP {status}, not an event stream", url)
        };
        return if status.is_server_error() {
            events.error(message.clone(), code);
            Attempt::Lost(message)
        } else {
            Attempt::Fatal(message, code)
        };
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("text/event-stream")
    {
        return Attempt::Fatal(
            format!(
                "{} answered with {:?}, not text/event-stream — this endpoint is not an event stream",
                url,
                if content_type.is_empty() {
                    "no content type"
                } else {
                    content_type.as_str()
                }
            ),
            E_STREAM_PROTOCOL,
        );
    }

    if !events.connected(ConnectionInfo {
        url: url.clone(),
        status: Some(status.as_u16()),
        protocol: None,
        headers: response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
            .collect(),
        session_present: None,
    }) {
        return Attempt::Consumer;
    }

    let mut parser = SseParser::new();
    let mut body = response.bytes_stream();
    let idle = idle_timeout(limits.idle_timeout_ms);

    loop {
        tokio::select! {
            chunk = tokio::time::timeout(idle, body.next()) => match chunk {
                Err(_) => {
                    events.error(
                        format!("nothing arrived for {} ms", limits.idle_timeout_ms),
                        E_STREAM_IDLE,
                    );
                    events.disconnected(None, "the stream went idle").await;
                    return Attempt::Done;
                }
                Ok(None) => {
                    return Attempt::Lost("the server ended the event stream".to_string());
                }
                Ok(Some(Err(error))) => {
                    let (message, code) = explain(&url, &error);
                    events.error(message.clone(), code);
                    return Attempt::Lost(message);
                }
                Ok(Some(Ok(bytes))) => {
                    let frames = match parser.feed(&bytes, limits.max_message_bytes) {
                        Ok(frames) => frames,
                        Err(e) => {
                            events.error(e.to_string(), E_STREAM_LIMIT);
                            events.disconnected(None, "an event was too big for this stream").await;
                            return Attempt::Done;
                        }
                    };
                    if let Some(ms) = parser.retry_ms() {
                        *retry_ms = Some(ms);
                    }
                    if let Some(id) = parser.last_event_id() {
                        *last_event_id = Some(id.to_string());
                    }
                    for frame in frames {
                        let meta = MessageMeta {
                            event: frame.event.clone(),
                            id: frame.id.clone(),
                            frame: Some("event".to_string()),
                            ..MessageMeta::default()
                        };
                        if !events.message(Direction::Incoming, Payload::text(frame.data), meta) {
                            return Attempt::Consumer;
                        }
                    }
                }
            },
            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    events.disconnected(None, "closed by the client").await;
                    return Attempt::Done;
                }
                Some(Command::Send(_, reply)) => {
                    let _ = reply.send(Err(CoreError::Stream(NOTHING_TO_SEND.to_string())));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(chunks: &[&str]) -> Vec<SseFrame> {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(parser.feed(chunk.as_bytes(), 1024).unwrap());
        }
        out
    }

    #[test]
    fn a_blank_line_dispatches_and_nothing_else_does() {
        assert!(frames(&["data: one\n"]).is_empty());
        let out = frames(&["data: one\n\n"]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, "one");
        assert_eq!(out[0].event, None);
    }

    #[test]
    fn multi_line_data_is_joined_with_newlines() {
        let out = frames(&["data: one\ndata: two\ndata:three\n\n"]);
        assert_eq!(out[0].data, "one\ntwo\nthree");
    }

    #[test]
    fn event_id_and_comments_are_handled() {
        let out = frames(&[": keep alive\nevent: tick\nid: 7\ndata: now\n\n"]);
        assert_eq!(out[0].event.as_deref(), Some("tick"));
        assert_eq!(out[0].id.as_deref(), Some("7"));
        assert_eq!(out[0].data, "now");
    }

    #[test]
    fn retry_is_read_and_never_dispatches_on_its_own() {
        let mut parser = SseParser::new();
        let out = parser.feed(b"retry: 2500\n\n", 1024).unwrap();
        assert!(out.is_empty());
        assert_eq!(parser.retry_ms(), Some(2500));
    }

    #[test]
    fn a_field_split_across_chunks_still_parses() {
        let out = frames(&["da", "ta: hel", "lo\n", "\n"]);
        assert_eq!(out[0].data, "hello");
    }

    #[test]
    fn crlf_and_lone_cr_both_terminate_lines() {
        assert_eq!(frames(&["data: a\r\n\r\n"])[0].data, "a");
        assert_eq!(frames(&["data: b\r\rdata: c"])[0].data, "b");
        let out = frames(&["data: c\r", "\n\n"]);
        assert_eq!(out[0].data, "c");
    }

    #[test]
    fn the_last_event_id_survives_events_that_do_not_carry_one() {
        let mut parser = SseParser::new();
        parser.feed(b"id: 1\ndata: a\n\n", 1024).unwrap();
        let out = parser.feed(b"data: b\n\n", 1024).unwrap();
        assert_eq!(parser.last_event_id(), Some("1"));
        assert_eq!(out[0].id.as_deref(), Some("1"));
    }

    #[test]
    fn a_field_with_no_value_and_an_unknown_field_are_tolerated() {
        let out = frames(&["data\ndata:\nfoo: bar\ndata: x\n\n"]);
        assert_eq!(out[0].data, "\n\nx");
    }

    #[test]
    fn a_leading_byte_order_mark_is_stripped() {
        let out = frames(&["\u{feff}data: a\n\n"]);
        assert_eq!(out[0].data, "a");
    }

    #[test]
    fn the_servers_retry_interval_is_held_between_the_configured_bounds() {
        let limits = StreamLimits {
            backoff_base_ms: 500,
            backoff_max_ms: 30_000,
            ..StreamLimits::default()
        };
        assert_eq!(reconnect_delay(&limits, Some(0), 1), 500);
        assert_eq!(reconnect_delay(&limits, Some(1), 1), 500);
        assert_eq!(reconnect_delay(&limits, Some(2_000), 1), 2_000);
        assert_eq!(reconnect_delay(&limits, Some(u64::MAX), 1), 30_000);
        assert_eq!(reconnect_delay(&limits, None, 2), limits.backoff(2));
    }

    #[test]
    fn an_event_over_the_limit_fails_loud() {
        let mut parser = SseParser::new();
        let err = parser
            .feed(b"data: aaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\n", 8)
            .unwrap_err();
        assert_eq!(err.code(), "E_STREAM_LIMIT");
    }
}
