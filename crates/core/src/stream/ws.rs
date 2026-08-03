use super::{
    default_port, idle_timeout, too_big, wrong_message, Command, ConnectionInfo, Direction,
    Emitter, MessageMeta, Outgoing, Payload, Resolved, WsOptions, E_STREAM_AUTH, E_STREAM_CONNECT,
    E_STREAM_IDLE, E_STREAM_LIMIT, E_STREAM_PROTOCOL, E_STREAM_TLS,
};
use crate::error::{CoreError, CoreResult};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

fn request(resolved: &Resolved, options: &WsOptions) -> CoreResult<http::Request<()>> {
    let mut request = resolved
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| CoreError::Stream(format!("cannot open {}: {e}", resolved.url)))?;
    let headers = request.headers_mut();
    for (name, value) in &resolved.headers {
        let name: http::HeaderName = name
            .parse()
            .map_err(|_| CoreError::Stream(format!("invalid header name: {name}")))?;
        let value = http::HeaderValue::from_str(value)
            .map_err(|_| CoreError::Stream(format!("invalid value for header {name}")))?;
        headers.insert(name, value);
    }
    if !options.subprotocols.is_empty() {
        let value = http::HeaderValue::from_str(&options.subprotocols.join(", "))
            .map_err(|_| CoreError::Stream("invalid subprotocol list".to_string()))?;
        headers.insert("sec-websocket-protocol", value);
    }
    Ok(request)
}

/// A bare "connection error" tells nobody anything, so every failure names the
/// host, what went wrong and what to do about it.
fn explain(url: &str, error: &WsError) -> (String, &'static str) {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| {
                let port = u.port().or_else(|| default_port(u.scheme()));
                match port {
                    Some(port) => format!("{h}:{port}"),
                    None => h.to_string(),
                }
            })
        })
        .unwrap_or_else(|| url.to_string());
    match error {
        WsError::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => (
            format!("nothing is listening on {host} — check the host, the port and that the server is running"),
            E_STREAM_CONNECT,
        ),
        WsError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => (
            format!("{host} did not answer the websocket handshake in time"),
            E_STREAM_CONNECT,
        ),
        WsError::Io(e) => (format!("cannot reach {host}: {e}"), E_STREAM_CONNECT),
        WsError::Tls(e) => (
            format!("the TLS handshake with {host} failed: {e} — check the certificate, or use ws:// if the server is not using TLS"),
            E_STREAM_TLS,
        ),
        WsError::Url(e) => (format!("invalid websocket url {url}: {e}"), E_STREAM_CONNECT),
        WsError::Http(response) => {
            let status = response.status();
            let body = response
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).trim().to_string())
                .filter(|b| !b.is_empty())
                .map(|b| format!(" — {b}"))
                .unwrap_or_default();
            if status == 401 || status == 403 {
                (
                    format!("{host} rejected the credentials with HTTP {status}{body}"),
                    E_STREAM_AUTH,
                )
            } else {
                (
                    format!("{host} refused the websocket upgrade with HTTP {status}{body}"),
                    E_STREAM_CONNECT,
                )
            }
        }
        WsError::Capacity(e) => (
            format!("{host} sent a message over this stream's size limit: {e}"),
            E_STREAM_LIMIT,
        ),
        WsError::ConnectionClosed | WsError::AlreadyClosed => (
            format!("{host} closed the connection"),
            E_STREAM_PROTOCOL,
        ),
        other => (
            format!("the websocket connection to {host} failed: {other}"),
            E_STREAM_PROTOCOL,
        ),
    }
}

fn connected_info(url: &str, response: &http::Response<Option<Vec<u8>>>) -> ConnectionInfo {
    ConnectionInfo {
        url: url.to_string(),
        status: Some(response.status().as_u16()),
        protocol: response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        headers: response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
            .collect(),
        session_present: None,
    }
}

fn outgoing_message(
    message: &Outgoing,
    limit: usize,
) -> CoreResult<(Message, Payload, MessageMeta)> {
    match message {
        Outgoing::Text { text } => {
            if text.len() > limit {
                return Err(too_big(text.len(), limit));
            }
            Ok((
                Message::text(text.clone()),
                Payload::text(text),
                MessageMeta::frame("text"),
            ))
        }
        Outgoing::Binary { base64 } => {
            let bytes = Outgoing::binary_bytes(base64)?;
            if bytes.len() > limit {
                return Err(too_big(bytes.len(), limit));
            }
            Ok((
                Message::binary(bytes.clone()),
                Payload::binary(&bytes),
                MessageMeta::frame("binary"),
            ))
        }
        Outgoing::Ping => Ok((
            Message::Ping(Vec::new().into()),
            Payload::text(""),
            MessageMeta::frame("ping"),
        )),
        other => Err(wrong_message(
            "a websocket",
            other,
            "websockets carry text and binary frames — publish, subscribe and unsubscribe are mqtt",
        )),
    }
}

fn incoming_message(
    message: Message,
    limit: usize,
) -> Result<Option<(Payload, MessageMeta)>, CoreError> {
    Ok(match message {
        Message::Text(text) => {
            if text.len() > limit {
                return Err(too_big(text.len(), limit));
            }
            Some((Payload::text(text.as_str()), MessageMeta::frame("text")))
        }
        Message::Binary(bytes) => {
            if bytes.len() > limit {
                return Err(too_big(bytes.len(), limit));
            }
            Some((Payload::binary(&bytes), MessageMeta::frame("binary")))
        }
        Message::Ping(bytes) => Some((Payload::detect(&bytes), MessageMeta::frame("ping"))),
        Message::Pong(bytes) => Some((Payload::detect(&bytes), MessageMeta::frame("pong"))),
        Message::Frame(_) | Message::Close(_) => None,
    })
}

pub(crate) async fn run(
    resolved: Resolved,
    options: WsOptions,
    mut commands: mpsc::Receiver<Command>,
    mut events: Emitter,
) {
    let limits = resolved.limits;
    let mut attempt: u32 = 0;

    loop {
        if !events.connecting(&resolved.url) {
            return;
        }
        let request = match request(&resolved, &options) {
            Ok(request) => request,
            Err(e) => {
                events.error(e.to_string(), e.code());
                events
                    .disconnected(None, "the websocket was never opened")
                    .await;
                return;
            }
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(limits.max_message_bytes))
            .max_frame_size(Some(limits.max_message_bytes));

        let connect = tokio_tungstenite::connect_async_with_config(request, Some(config), false);
        let outcome = tokio::time::timeout(
            Duration::from_millis(limits.connect_timeout_ms.max(1)),
            connect,
        )
        .await;

        let (socket, response) = match outcome {
            Ok(Ok(pair)) => pair,
            Ok(Err(error)) => {
                let (message, code) = explain(&resolved.url, &error);
                if !events.error(message.clone(), code) {
                    return;
                }
                match retry(&mut events, &options, &limits, &mut attempt, &message).await {
                    Retry::Again => continue,
                    Retry::Stop => return,
                }
            }
            Err(_) => {
                let message = format!(
                    "{} did not complete the websocket handshake within {} ms",
                    resolved.url, limits.connect_timeout_ms
                );
                if !events.error(message.clone(), E_STREAM_CONNECT) {
                    return;
                }
                match retry(&mut events, &options, &limits, &mut attempt, &message).await {
                    Retry::Again => continue,
                    Retry::Stop => return,
                }
            }
        };

        if !events.connected(connected_info(&resolved.url, &response)) {
            return;
        }
        let up_since = std::time::Instant::now();

        match pump(socket, &mut commands, &mut events, &options, &limits).await {
            Ended::Closed => return,
            Ended::Consumer => return,
            Ended::Lost(reason) => {
                if up_since.elapsed() >= super::HEALTHY_SESSION {
                    attempt = 0;
                }
                match retry(&mut events, &options, &limits, &mut attempt, &reason).await {
                    Retry::Again => continue,
                    Retry::Stop => return,
                }
            }
        }
    }
}

enum Ended {
    Closed,
    Consumer,
    Lost(String),
}

enum Retry {
    Again,
    Stop,
}

async fn retry(
    events: &mut Emitter,
    options: &WsOptions,
    limits: &super::StreamLimits,
    attempt: &mut u32,
    reason: &str,
) -> Retry {
    if !options.auto_reconnect || *attempt >= limits.max_reconnect_attempts {
        let reason = if options.auto_reconnect {
            format!(
                "{reason} — gave up after {} attempts",
                limits.max_reconnect_attempts
            )
        } else {
            reason.to_string()
        };
        events.disconnected(None, reason).await;
        return Retry::Stop;
    }
    *attempt += 1;
    let delay = limits.backoff(*attempt);
    if !events.reconnecting(*attempt, delay, reason) {
        return Retry::Stop;
    }
    tokio::time::sleep(Duration::from_millis(delay)).await;
    Retry::Again
}

async fn pump<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    commands: &mut mpsc::Receiver<Command>,
    events: &mut Emitter,
    options: &WsOptions,
    limits: &super::StreamLimits,
) -> Ended
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let idle = idle_timeout(limits.idle_timeout_ms);
    let ping_every = idle_timeout(options.ping_interval_ms);
    let mut pings = tokio::time::interval(ping_every);
    pings.tick().await;

    loop {
        tokio::select! {
            frame = tokio::time::timeout(idle, socket.next()) => match frame {
                Err(_) => {
                    events.error(
                        format!("nothing arrived for {} ms", limits.idle_timeout_ms),
                        E_STREAM_IDLE,
                    );
                    let _ = socket.close(None).await;
                    events.disconnected(None, "the stream went idle").await;
                    return Ended::Closed;
                }
                Ok(None) => return Ended::Lost("the server closed the connection".to_string()),
                Ok(Some(Err(error))) => {
                    let (message, code) = explain("", &error);
                    if !events.error(message.clone(), code) {
                        return Ended::Consumer;
                    }
                    if code == E_STREAM_LIMIT {
                        events.disconnected(Some(1009), "a message was too big for this stream").await;
                        return Ended::Closed;
                    }
                    return Ended::Lost(message);
                }
                Ok(Some(Ok(Message::Close(frame)))) => {
                    let (code, reason) = match frame {
                        Some(CloseFrame { code, reason }) => (
                            Some(u16::from(code)),
                            if reason.is_empty() {
                                "the server closed the connection".to_string()
                            } else {
                                reason.to_string()
                            },
                        ),
                        None => (None, "the server closed the connection".to_string()),
                    };
                    let _ = socket.close(None).await;
                    events.disconnected(code, reason).await;
                    return Ended::Closed;
                }
                Ok(Some(Ok(message))) => {
                    match incoming_message(message, limits.max_message_bytes) {
                        Ok(Some((payload, meta))) => {
                            if !events.message(Direction::Incoming, payload, meta) {
                                return Ended::Consumer;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            events.error(e.to_string(), E_STREAM_LIMIT);
                            let _ = socket
                                .close(Some(CloseFrame {
                                    code: CloseCode::Size,
                                    reason: "message too big".into(),
                                }))
                                .await;
                            events.disconnected(Some(1009), "a message was too big for this stream").await;
                            return Ended::Closed;
                        }
                    }
                }
            },
            _ = pings.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return Ended::Lost("the connection dropped while pinging".to_string());
                }
            }
            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    let _ = socket
                        .close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "closed by the client".into(),
                        }))
                        .await;
                    events.disconnected(Some(1000), "closed by the client").await;
                    return Ended::Closed;
                }
                Some(Command::Send(message, reply)) => {
                    match outgoing_message(&message, limits.max_message_bytes) {
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                        Ok((frame, payload, meta)) => match socket.send(frame).await {
                            Ok(()) => {
                                let _ = reply.send(Ok(()));
                                if !events.message(Direction::Outgoing, payload, meta) {
                                    return Ended::Consumer;
                                }
                            }
                            Err(error) => {
                                let (message, _) = explain("", &error);
                                let _ = reply.send(Err(CoreError::Stream(message.clone())));
                                return Ended::Lost(message);
                            }
                        },
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{StreamLimits, StreamSpec};

    fn resolved(url: &str) -> Resolved {
        Resolved {
            url: url.to_string(),
            headers: vec![("X-Trace".to_string(), "on".to_string())],
            limits: StreamLimits::default(),
        }
    }

    #[test]
    fn the_handshake_carries_headers_and_subprotocols() {
        let options = WsOptions {
            subprotocols: vec!["chat".to_string(), "v2".to_string()],
            ..WsOptions::default()
        };
        let request = request(&resolved("wss://x.dev/socket"), &options).unwrap();
        assert_eq!(request.headers()["x-trace"], "on");
        assert_eq!(request.headers()["sec-websocket-protocol"], "chat, v2");
        assert_eq!(request.uri().to_string(), "wss://x.dev/socket");
    }

    #[test]
    fn a_refused_connection_says_what_to_check() {
        let error = WsError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        let (message, code) = explain("ws://127.0.0.1:9/socket", &error);
        assert!(
            message.contains("nothing is listening on 127.0.0.1:9"),
            "{message}"
        );
        assert_eq!(code, E_STREAM_CONNECT);
    }

    #[test]
    fn a_rejected_handshake_is_reported_as_auth_not_as_a_generic_failure() {
        let response = http::Response::builder()
            .status(401)
            .body(Some(b"bad token".to_vec()))
            .unwrap();
        let (message, code) = explain("wss://x.dev/socket", &WsError::Http(Box::new(response)));
        assert!(
            message.contains("rejected the credentials with HTTP 401"),
            "{message}"
        );
        assert!(message.contains("bad token"));
        assert_eq!(code, E_STREAM_AUTH);
    }

    #[test]
    fn an_oversized_outgoing_message_is_refused_before_it_is_sent() {
        let err = outgoing_message(&Outgoing::text("x".repeat(20)), 8).unwrap_err();
        assert_eq!(err.code(), "E_STREAM_LIMIT");
        assert!(err.to_string().contains("20 bytes"));
    }

    #[test]
    fn mqtt_shaped_payloads_are_refused_with_a_useful_message() {
        let err = outgoing_message(
            &Outgoing::Publish {
                topic: "a/b".to_string(),
                payload: "1".to_string(),
                qos: 0,
                retain: false,
            },
            1024,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("websockets carry text and binary"),
            "{err}"
        );
    }

    #[test]
    fn a_binary_payload_that_is_not_base64_fails_loud() {
        let err = outgoing_message(
            &Outgoing::Binary {
                base64: "not base64!!".to_string(),
            },
            1024,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not valid base64"), "{err}");
    }

    #[test]
    fn ping_and_pong_frames_are_surfaced_like_any_other() {
        let (payload, meta) = incoming_message(Message::Ping(b"beat".to_vec().into()), 64)
            .unwrap()
            .unwrap();
        assert_eq!(payload.as_text(), Some("beat"));
        assert_eq!(meta.frame.as_deref(), Some("ping"));
        let (_, meta) = incoming_message(Message::Pong(Vec::new().into()), 64)
            .unwrap()
            .unwrap();
        assert_eq!(meta.frame.as_deref(), Some("pong"));
    }

    #[test]
    fn an_oversized_incoming_message_is_rejected() {
        let err = incoming_message(Message::text("x".repeat(99)), 10).unwrap_err();
        assert_eq!(err.code(), "E_STREAM_LIMIT");
    }

    #[test]
    fn the_default_spec_does_not_reconnect_but_sse_does() {
        let spec = StreamSpec::default();
        assert!(!spec.ws.auto_reconnect);
        assert!(spec.sse.auto_reconnect);
    }
}
