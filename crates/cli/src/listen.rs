use crate::style::Style;
use mandalo_core::stream::{
    self, Direction, MessageMeta, Outgoing, Payload, StreamEvent, StreamKind, StreamSpec,
};
use mandalo_core::{CoreError, CoreResult, HostPolicy};
use std::sync::Arc;
use std::time::Duration;

pub struct Options {
    pub max_messages: Option<usize>,
    pub timeout_secs: Option<u64>,
    pub send: Vec<Outgoing>,
    pub json: bool,
}

pub fn kind_from_url(url: &str) -> CoreResult<StreamKind> {
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .ok_or_else(|| {
            CoreError::Stream(format!(
                "{url} has no scheme — start it with ws://, wss://, http://, https://, mqtt:// or mqtts://"
            ))
        })?;
    match scheme.as_str() {
        "ws" | "wss" => Ok(StreamKind::WebSocket),
        "http" | "https" => Ok(StreamKind::Sse),
        "mqtt" | "mqtts" => Ok(StreamKind::Mqtt),
        other => Err(CoreError::Stream(format!(
            "{other}:// is not a realtime scheme — pass --kind ws, sse or mqtt to say what it is"
        ))),
    }
}

pub fn header(raw: &str) -> CoreResult<(String, String)> {
    let (name, value) = raw.split_once(':').ok_or_else(|| {
        CoreError::Stream(format!("a header must look like name:value, not {raw:?}"))
    })?;
    if name.trim().is_empty() {
        return Err(CoreError::Stream(format!(
            "a header must look like name:value, not {raw:?}"
        )));
    }
    Ok((name.trim().to_string(), value.trim().to_string()))
}

fn describe_payload(payload: &Payload) -> String {
    match payload {
        Payload::Text { text } => text.clone(),
        Payload::Binary { bytes, base64 } => format!("{bytes} bytes of binary — {base64}"),
    }
}

fn describe_meta(meta: &MessageMeta) -> String {
    let mut parts = Vec::new();
    if let Some(topic) = &meta.topic {
        parts.push(topic.clone());
    }
    if let Some(event) = &meta.event {
        parts.push(format!("event {event}"));
    }
    if let Some(id) = &meta.id {
        parts.push(format!("id {id}"));
    }
    if let Some(qos) = meta.qos {
        parts.push(format!("qos {qos}"));
    }
    if meta.retain == Some(true) {
        parts.push("retained".to_string());
    }
    if let Some(frame) = &meta.frame {
        if frame == "ping" || frame == "pong" {
            parts.push(frame.clone());
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("  [{}]", parts.join(" · "))
    }
}

fn render(event: &StreamEvent, style: &Style) -> String {
    match event {
        StreamEvent::Connecting { url, .. } => style.dim(&format!("connecting to {url}")),
        StreamEvent::Connected { info, .. } => {
            let mut detail = Vec::new();
            if let Some(status) = info.status {
                detail.push(status.to_string());
            }
            if let Some(protocol) = &info.protocol {
                detail.push(protocol.clone());
            }
            if let Some(session) = info.session_present {
                detail.push(format!("session present: {session}"));
            }
            style.pass(&format!("connected  {}", detail.join(" · ")))
        }
        StreamEvent::Message {
            direction,
            payload,
            meta,
            ..
        } => {
            let arrow = match direction {
                Direction::Incoming => style.bold("in "),
                Direction::Outgoing => style.dim("out"),
            };
            format!(
                "{arrow}  {}{}",
                describe_payload(payload),
                style.dim(&describe_meta(meta))
            )
        }
        StreamEvent::Reconnecting {
            attempt,
            delay_ms,
            reason,
            ..
        } => style.warn(&format!(
            "reconnecting  attempt {attempt} in {delay_ms}ms — {reason}"
        )),
        StreamEvent::Dropped { count, reason, .. } => {
            style.warn(&format!("dropped {count} messages — {reason}"))
        }
        StreamEvent::Disconnected { code, reason, .. } => {
            let code = code.map(|c| format!("{c} ")).unwrap_or_default();
            style.dim(&format!("closed  {code}{reason}"))
        }
        StreamEvent::Error {
            message, code: c, ..
        } => style.fail(&format!("error  {message} [{c}]")),
    }
}

/// Prints every event as it arrives and returns whether the stream ended cleanly,
/// so `mandalo listen` exits non-zero exactly when something went wrong.
pub async fn listen(
    spec: StreamSpec,
    policy: Arc<dyn HostPolicy>,
    style: &Style,
    options: Options,
) -> CoreResult<bool> {
    let (tx, mut events) = stream::event_channel(&spec.limits);
    let handle = stream::open(spec, policy, tx).await?;

    let deadline = options
        .timeout_secs
        .map(|secs| tokio::time::Instant::now() + Duration::from_secs(secs));
    let mut messages = 0usize;
    let mut failed = false;
    let mut queued = options.send;

    loop {
        let event = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(event) => event,
                Err(_) => {
                    handle.close().await?;
                    break;
                }
            },
            None => events.recv().await,
        };
        let Some(event) = event else { break };

        if options.json {
            println!(
                "{}",
                serde_json::to_string(&event)
                    .map_err(|e| CoreError::Parse(format!("cannot render the event: {e}")))?
            );
        } else {
            println!("{}", render(&event, style));
        }

        match &event {
            StreamEvent::Connected { .. } => {
                for message in queued.drain(..) {
                    if let Err(e) = handle.send(message).await {
                        failed = true;
                        eprintln!("{}", style.fail(&e.to_string()));
                    }
                }
            }
            StreamEvent::Error { .. } => failed = true,
            StreamEvent::Message {
                direction: Direction::Incoming,
                ..
            } => {
                messages += 1;
                if options.max_messages.is_some_and(|max| messages >= max) {
                    handle.close().await?;
                    break;
                }
            }
            StreamEvent::Disconnected { .. } => break,
            _ => {}
        }
    }

    Ok(!failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheme_decides_the_protocol() {
        assert_eq!(kind_from_url("wss://x.dev").unwrap(), StreamKind::WebSocket);
        assert_eq!(kind_from_url("https://x.dev/e").unwrap(), StreamKind::Sse);
        assert_eq!(kind_from_url("mqtts://b.dev").unwrap(), StreamKind::Mqtt);
        assert!(kind_from_url("x.dev")
            .unwrap_err()
            .to_string()
            .contains("no scheme"));
        assert!(kind_from_url("amqp://x.dev")
            .unwrap_err()
            .to_string()
            .contains("--kind"));
    }

    #[test]
    fn headers_are_name_colon_value() {
        assert_eq!(
            header("X-Trace: abc").unwrap(),
            ("X-Trace".to_string(), "abc".to_string())
        );
        assert_eq!(
            header("Authorization:Bearer x").unwrap(),
            ("Authorization".to_string(), "Bearer x".to_string())
        );
        assert!(header("nope").is_err());
        assert!(header(": empty").is_err());
    }

    #[test]
    fn every_event_renders_as_one_readable_line() {
        let style = Style::plain();
        let events = vec![
            StreamEvent::Connecting {
                at: 1,
                url: "ws://x.dev".to_string(),
            },
            StreamEvent::Message {
                at: 2,
                direction: Direction::Incoming,
                payload: Payload::text("hola"),
                meta: MessageMeta {
                    topic: Some("a/b".to_string()),
                    qos: Some(1),
                    ..MessageMeta::default()
                },
            },
            StreamEvent::Reconnecting {
                at: 3,
                attempt: 2,
                delay_ms: 500,
                reason: "the server closed the connection".to_string(),
            },
            StreamEvent::Dropped {
                at: 4,
                count: 7,
                reason: "the event buffer was full".to_string(),
            },
            StreamEvent::Disconnected {
                at: 5,
                code: Some(1000),
                reason: "closed by the client".to_string(),
            },
            StreamEvent::Error {
                at: 6,
                message: "nothing is listening".to_string(),
                code: "E_STREAM_CONNECT".to_string(),
            },
        ];
        for event in &events {
            let line = render(event, &style);
            assert!(!line.is_empty());
            assert!(!line.contains('\n'), "{line}");
        }
        assert!(render(&events[1], &style).contains("a/b · qos 1"));
        assert!(render(&events[5], &style).contains("[E_STREAM_CONNECT]"));
    }

    #[test]
    fn a_binary_payload_is_described_not_dumped_raw() {
        let described = describe_payload(&Payload::binary(&[0xff, 0x00]));
        assert!(described.starts_with("2 bytes of binary"), "{described}");
    }
}
