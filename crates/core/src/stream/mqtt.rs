use super::{
    default_port, idle_timeout, wrong_message, Command, ConnectionInfo, Direction, Emitter,
    MessageMeta, MqttVersion, Outgoing, Payload, Resolved, StreamLimits, StreamSpec, Subscription,
    E_STREAM_AUTH, E_STREAM_CONNECT, E_STREAM_IDLE, E_STREAM_PROTOCOL, E_STREAM_TLS,
};
use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use crate::request::Auth;
use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions as ClientOptions, Packet,
    QoS, StateError, Transport,
};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug)]
pub(crate) struct Prepared {
    options: ClientOptions,
    subscriptions: Vec<(String, QoS)>,
}

fn qos(level: u8) -> CoreResult<QoS> {
    match level {
        0 => Ok(QoS::AtMostOnce),
        1 => Ok(QoS::AtLeastOnce),
        2 => Ok(QoS::ExactlyOnce),
        other => Err(CoreError::Stream(format!(
            "qos {other} does not exist — mqtt has qos 0, 1 and 2"
        ))),
    }
}

/// MQTT authenticates in its CONNECT packet, not with headers, so anything that
/// only makes sense as a header has to be rejected instead of silently ignored.
fn credentials(
    auth: &Auth,
    vars: &BTreeMap<String, String>,
) -> CoreResult<Option<(String, String)>> {
    match auth {
        Auth::None => Ok(None),
        Auth::Basic { username, password } => Ok(Some((
            interpolate::apply(username, vars)?,
            interpolate::apply(password, vars)?,
        ))),
        Auth::Bearer { .. } | Auth::Apikey { .. } => Err(CoreError::Unsupported(
            "mqtt signs in with a username and a password — use basic auth, or the mqtt username and password fields"
                .to_string(),
        )),
    }
}

pub(crate) fn prepare(spec: &StreamSpec, resolved: &Resolved) -> CoreResult<Prepared> {
    if spec.mqtt.protocol_version == MqttVersion::V5 {
        return Err(CoreError::Unsupported(
            "mqtt 5 is not wired up yet — connect with 3.1.1 (the default) or open an issue"
                .to_string(),
        ));
    }
    let vars = &spec.vars;
    let url = reqwest::Url::parse(&resolved.url)
        .map_err(|e| CoreError::Stream(format!("invalid mqtt url {:?}: {e}", resolved.url)))?;
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url
        .host_str()
        .ok_or_else(|| CoreError::Stream(format!("mqtt url has no host: {}", resolved.url)))?
        .to_string();
    let port = url
        .port()
        .or_else(|| default_port(&scheme))
        .ok_or_else(|| CoreError::Stream(format!("mqtt url has no port: {}", resolved.url)))?;

    let client_id = match spec.mqtt.client_id.as_deref() {
        Some(id) => interpolate::apply(id, vars)?,
        None => format!(
            "mandalo-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        ),
    };
    if client_id.trim().is_empty() {
        return Err(CoreError::Stream(
            "the mqtt client id cannot be empty".to_string(),
        ));
    }

    let over_websocket = matches!(scheme.as_str(), "ws" | "wss");
    let address = if over_websocket {
        resolved.url.clone()
    } else {
        host.clone()
    };
    let mut options = ClientOptions::new(client_id, address, port);
    options.set_keep_alive(Duration::from_secs(spec.mqtt.keep_alive_secs.max(1)));
    options.set_clean_session(spec.mqtt.clean_session);
    options.set_max_packet_size(
        resolved.limits.max_message_bytes,
        resolved.limits.max_message_bytes,
    );
    options.set_transport(match scheme.as_str() {
        "mqtt" => Transport::Tcp,
        "mqtts" => Transport::tls_with_default_config(),
        "ws" => Transport::Ws,
        "wss" => Transport::wss_with_default_config(),
        other => {
            return Err(CoreError::Stream(format!(
                "{other}:// is not an mqtt transport — use mqtt://, mqtts://, ws:// or wss://"
            )))
        }
    });

    let explicit = match (spec.mqtt.username.as_deref(), spec.mqtt.password.as_deref()) {
        (Some(user), password) => Some((
            interpolate::apply(user, vars)?,
            interpolate::apply(password.unwrap_or_default(), vars)?,
        )),
        (None, _) => None,
    };
    if let Some((user, password)) = explicit.or(credentials(&spec.auth, vars)?) {
        options.set_credentials(user, password);
    }

    let mut subscriptions = Vec::with_capacity(spec.mqtt.subscriptions.len());
    for Subscription { topic, qos: level } in &spec.mqtt.subscriptions {
        let topic = interpolate::apply(topic, vars)?;
        if topic.trim().is_empty() {
            return Err(CoreError::Stream(
                "an mqtt subscription needs a topic filter".to_string(),
            ));
        }
        subscriptions.push((topic, qos(*level)?));
    }

    Ok(Prepared {
        options,
        subscriptions,
    })
}

fn explain(url: &str, error: &ConnectionError, handshaken: bool) -> (String, &'static str, bool) {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string());
    // A broker that drops the socket instead of answering CONNACK is the usual
    // shape of a rejected client id or a rejected password, and "connection
    // closed abruptly" on its own sends nobody anywhere.
    if !handshaken
        && matches!(
            error,
            ConnectionError::MqttState(StateError::ConnectionAborted)
                | ConnectionError::MqttState(StateError::Io(_))
        )
    {
        return (
            format!(
                "{host} closed the connection during the mqtt handshake — the usual causes are a rejected username or password, a rejected client id, or a broker that only accepts TLS"
            ),
            E_STREAM_AUTH,
            true,
        );
    }
    match error {
        ConnectionError::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => (
            format!("no mqtt broker is listening on {url} — check the host, the port and that the broker is running"),
            E_STREAM_CONNECT,
            true,
        ),
        ConnectionError::Io(e) => (format!("cannot reach {host}: {e}"), E_STREAM_CONNECT, true),
        ConnectionError::Tls(e) => (
            format!("the TLS handshake with {host} failed: {e} — check the certificate, or use mqtt:// if the broker is not using TLS"),
            E_STREAM_TLS,
            false,
        ),
        ConnectionError::ConnectionRefused(code) => {
            let (what, retryable) = match code {
                ConnectReturnCode::BadUserNamePassword => {
                    ("the broker rejected the username or password", false)
                }
                ConnectReturnCode::NotAuthorized => {
                    ("the broker says this client is not authorized", false)
                }
                ConnectReturnCode::BadClientId => {
                    ("the broker rejected the client id", false)
                }
                ConnectReturnCode::RefusedProtocolVersion => {
                    ("the broker does not speak mqtt 3.1.1", false)
                }
                ConnectReturnCode::ServiceUnavailable => {
                    ("the broker is not accepting connections right now", true)
                }
                ConnectReturnCode::Success => ("the broker refused the connection", true),
            };
            (
                format!("{what} ({host})"),
                if retryable { E_STREAM_CONNECT } else { E_STREAM_AUTH },
                retryable,
            )
        }
        ConnectionError::NetworkTimeout | ConnectionError::FlushTimeout => (
            format!("{host} stopped answering"),
            E_STREAM_CONNECT,
            true,
        ),
        ConnectionError::RequestsDone => (
            "the mqtt client was shut down".to_string(),
            E_STREAM_PROTOCOL,
            false,
        ),
        other => (
            format!("the mqtt connection to {host} failed: {other}"),
            E_STREAM_PROTOCOL,
            true,
        ),
    }
}

async fn subscribe_all(
    client: &AsyncClient,
    subscriptions: &[(String, QoS)],
    events: &mut Emitter,
) -> bool {
    for (topic, level) in subscriptions {
        if let Err(e) = client.subscribe(topic.clone(), *level).await {
            if !events.error(
                format!("cannot subscribe to {topic}: {e}"),
                E_STREAM_PROTOCOL,
            ) {
                return false;
            }
        }
    }
    true
}

async fn send(
    client: &AsyncClient,
    message: &Outgoing,
    limits: &StreamLimits,
) -> CoreResult<(Payload, MessageMeta)> {
    match message {
        Outgoing::Publish {
            topic,
            payload,
            qos: level,
            retain,
        } => {
            if payload.len() > limits.max_message_bytes {
                return Err(super::too_big(payload.len(), limits.max_message_bytes));
            }
            let level = qos(*level)?;
            client
                .publish(topic.clone(), level, *retain, payload.clone())
                .await
                .map_err(|e| CoreError::Stream(format!("cannot publish to {topic}: {e}")))?;
            Ok((
                Payload::text(payload),
                MessageMeta {
                    topic: Some(topic.clone()),
                    qos: Some(level as u8),
                    retain: Some(*retain),
                    frame: Some("publish".to_string()),
                    ..MessageMeta::default()
                },
            ))
        }
        Outgoing::Subscribe { topic, qos: level } => {
            let level = qos(*level)?;
            client
                .subscribe(topic.clone(), level)
                .await
                .map_err(|e| CoreError::Stream(format!("cannot subscribe to {topic}: {e}")))?;
            Ok((
                Payload::text(topic),
                MessageMeta {
                    topic: Some(topic.clone()),
                    qos: Some(level as u8),
                    frame: Some("subscribe".to_string()),
                    ..MessageMeta::default()
                },
            ))
        }
        Outgoing::Unsubscribe { topic } => {
            client
                .unsubscribe(topic.clone())
                .await
                .map_err(|e| CoreError::Stream(format!("cannot unsubscribe from {topic}: {e}")))?;
            Ok((
                Payload::text(topic),
                MessageMeta {
                    topic: Some(topic.clone()),
                    frame: Some("unsubscribe".to_string()),
                    ..MessageMeta::default()
                },
            ))
        }
        other => Err(wrong_message(
            "mqtt",
            other,
            "every mqtt message needs a topic — publish, subscribe or unsubscribe",
        )),
    }
}

pub(crate) async fn run(
    resolved: Resolved,
    prepared: Prepared,
    mut commands: mpsc::Receiver<Command>,
    mut events: Emitter,
) {
    let limits = resolved.limits;
    let Prepared {
        options,
        subscriptions,
    } = prepared;
    let (client, mut eventloop) = AsyncClient::new(options, 32);

    if !events.connecting(&resolved.url) {
        return;
    }

    // The event loop is polled on its own task: `poll()` is not cancel-safe, so
    // it must never sit in a `select!` arm next to the command channel.
    let (packets_tx, mut packets) = mpsc::channel(64);
    let (resume_tx, mut resume) = mpsc::channel::<()>(1);
    let poller = tokio::spawn(async move {
        loop {
            let outcome = eventloop.poll().await;
            let failed = outcome.is_err();
            if packets_tx.send(outcome).await.is_err() {
                return;
            }
            if failed && resume.recv().await.is_none() {
                return;
            }
        }
    });

    let idle = idle_timeout(limits.idle_timeout_ms);
    let mut attempt: u32 = 0;
    let mut connected = false;
    let mut ever_connected = false;

    loop {
        tokio::select! {
            packet = tokio::time::timeout(idle, packets.recv()) => match packet {
                Err(_) => {
                    events.error(
                        format!("nothing arrived for {} ms", limits.idle_timeout_ms),
                        E_STREAM_IDLE,
                    );
                    let _ = client.disconnect().await;
                    events.disconnected(None, "the stream went idle").await;
                    break;
                }
                Ok(None) => {
                    events.disconnected(None, "the mqtt event loop stopped").await;
                    break;
                }
                Ok(Some(Ok(Event::Incoming(Packet::ConnAck(ack))))) => {
                    attempt = 0;
                    connected = true;
                    ever_connected = true;
                    if !events.connected(ConnectionInfo {
                        url: resolved.url.clone(),
                        status: None,
                        protocol: Some("mqtt/3.1.1".to_string()),
                        headers: Vec::new(),
                        session_present: Some(ack.session_present),
                    }) {
                        break;
                    }
                    if !subscribe_all(&client, &subscriptions, &mut events).await {
                        break;
                    }
                }
                Ok(Some(Ok(Event::Incoming(Packet::Publish(publish))))) => {
                    let meta = MessageMeta {
                        topic: Some(publish.topic.clone()),
                        qos: Some(publish.qos as u8),
                        retain: Some(publish.retain),
                        frame: Some("publish".to_string()),
                        ..MessageMeta::default()
                    };
                    if !events.message(
                        Direction::Incoming,
                        Payload::detect(&publish.payload),
                        meta,
                    ) {
                        break;
                    }
                }
                Ok(Some(Ok(Event::Incoming(Packet::Disconnect)))) => {
                    events.disconnected(None, "the broker closed the connection").await;
                    break;
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(error))) => {
                    let (message, code, retryable) = explain(&resolved.url, &error, ever_connected);
                    if !events.error(message.clone(), code) {
                        break;
                    }
                    if !retryable || attempt >= limits.max_reconnect_attempts {
                        let reason = if retryable {
                            format!(
                                "{message} — gave up after {} attempts",
                                limits.max_reconnect_attempts
                            )
                        } else {
                            message
                        };
                        events.disconnected(None, reason).await;
                        break;
                    }
                    attempt += 1;
                    connected = false;
                    let delay = limits.backoff(attempt);
                    if !events.reconnecting(attempt, delay, &message) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    if !events.connecting(&resolved.url) || resume_tx.send(()).await.is_err() {
                        break;
                    }
                }
            },
            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    let _ = client.disconnect().await;
                    events.disconnected(None, "closed by the client").await;
                    break;
                }
                Some(Command::Send(message, reply)) => {
                    if !connected {
                        let _ = reply.send(Err(CoreError::Stream(
                            "the mqtt connection is not up yet — wait for the connected event"
                                .to_string(),
                        )));
                        continue;
                    }
                    match send(&client, &message, &limits).await {
                        Ok((payload, meta)) => {
                            let _ = reply.send(Ok(()));
                            if !events.message(Direction::Outgoing, payload, meta) {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                }
            },
        }
    }

    poller.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{MqttOptions, StreamKind, StreamLimits};

    fn spec(url: &str) -> StreamSpec {
        StreamSpec::new(StreamKind::Mqtt, url)
    }

    fn resolved(url: &str) -> Resolved {
        Resolved {
            url: url.to_string(),
            headers: Vec::new(),
            limits: StreamLimits::default(),
        }
    }

    #[test]
    fn plain_tls_and_websocket_urls_all_map_to_a_transport() {
        for (url, port) in [
            ("mqtt://broker.dev", 1883),
            ("mqtts://broker.dev", 8883),
            ("ws://broker.dev/mqtt", 80),
            ("wss://broker.dev/mqtt", 443),
        ] {
            let prepared = prepare(&spec(url), &resolved(url)).unwrap();
            assert_eq!(prepared.options.broker_address().1, port, "{url}");
        }
        let explicit = "mqtt://broker.dev:1885";
        let prepared = prepare(&spec(explicit), &resolved(explicit)).unwrap();
        assert_eq!(prepared.options.broker_address().1, 1885);
    }

    #[test]
    fn a_generated_client_id_is_unique_and_a_given_one_is_interpolated() {
        let url = "mqtt://broker.dev";
        let first = prepare(&spec(url), &resolved(url)).unwrap();
        let second = prepare(&spec(url), &resolved(url)).unwrap();
        assert!(first.options.client_id().starts_with("mandalo-"));
        assert_ne!(first.options.client_id(), second.options.client_id());

        let mut with_id = spec(url);
        with_id.mqtt.client_id = Some("probe-{{env}}".to_string());
        with_id.vars = [("env".to_string(), "ci".to_string())]
            .into_iter()
            .collect();
        let prepared = prepare(&with_id, &resolved(url)).unwrap();
        assert_eq!(prepared.options.client_id(), "probe-ci");
    }

    #[test]
    fn basic_auth_and_the_mqtt_fields_both_become_credentials() {
        let url = "mqtt://broker.dev";
        let mut with_auth = spec(url);
        with_auth.auth = Auth::Basic {
            username: "ada".to_string(),
            password: "{{p}}".to_string(),
        };
        with_auth.vars = [("p".to_string(), "lovelace".to_string())]
            .into_iter()
            .collect();
        let prepared = prepare(&with_auth, &resolved(url)).unwrap();
        let login = prepared.options.credentials().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("ada", "lovelace")
        );

        let mut with_fields = spec(url);
        with_fields.mqtt = MqttOptions {
            username: Some("nova".to_string()),
            password: Some("star".to_string()),
            ..MqttOptions::default()
        };
        let prepared = prepare(&with_fields, &resolved(url)).unwrap();
        let login = prepared.options.credentials().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("nova", "star")
        );
    }

    #[test]
    fn header_shaped_auth_is_refused_because_mqtt_has_no_headers() {
        let url = "mqtt://broker.dev";
        let mut with_bearer = spec(url);
        with_bearer.auth = Auth::Bearer {
            token: "t".to_string(),
        };
        let err = prepare(&with_bearer, &resolved(url)).unwrap_err();
        assert!(err.to_string().contains("username and a password"), "{err}");
    }

    #[test]
    fn subscriptions_are_interpolated_and_their_qos_is_checked() {
        let url = "mqtt://broker.dev";
        let mut with_topics = spec(url);
        with_topics.mqtt.subscriptions = vec![
            Subscription {
                topic: "sensors/{{room}}/#".to_string(),
                qos: 2,
            },
            Subscription {
                topic: "status".to_string(),
                qos: 0,
            },
        ];
        with_topics.vars = [("room".to_string(), "kitchen".to_string())]
            .into_iter()
            .collect();
        let prepared = prepare(&with_topics, &resolved(url)).unwrap();
        assert_eq!(
            prepared.subscriptions,
            vec![
                ("sensors/kitchen/#".to_string(), QoS::ExactlyOnce),
                ("status".to_string(), QoS::AtMostOnce),
            ]
        );

        let mut bad_qos = spec(url);
        bad_qos.mqtt.subscriptions = vec![Subscription {
            topic: "a".to_string(),
            qos: 7,
        }];
        let err = prepare(&bad_qos, &resolved(url)).unwrap_err();
        assert!(err.to_string().contains("qos 7 does not exist"), "{err}");
    }

    #[test]
    fn mqtt_5_is_refused_out_loud_rather_than_downgraded() {
        let url = "mqtt://broker.dev";
        let mut v5 = spec(url);
        v5.mqtt.protocol_version = MqttVersion::V5;
        let err = prepare(&v5, &resolved(url)).unwrap_err();
        assert!(err.to_string().contains("mqtt 5 is not wired up"), "{err}");
    }

    #[test]
    fn a_refused_broker_and_bad_credentials_read_differently() {
        let (message, code, retryable) = explain(
            "mqtt://127.0.0.1:1",
            &ConnectionError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "refused",
            )),
            true,
        );
        assert!(message.contains("no mqtt broker is listening"), "{message}");
        assert_eq!(code, E_STREAM_CONNECT);
        assert!(retryable);

        let (message, code, retryable) = explain(
            "mqtt://broker.dev",
            &ConnectionError::ConnectionRefused(ConnectReturnCode::BadUserNamePassword),
            true,
        );
        assert!(
            message.contains("rejected the username or password"),
            "{message}"
        );
        assert_eq!(code, E_STREAM_AUTH);
        assert!(!retryable, "bad credentials must not be retried");
    }
}
