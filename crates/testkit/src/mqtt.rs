use rumqttd::{Broker, Config, ConnectionSettings, RouterConfig, ServerSettings};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const MQTT_USER: &str = "ada";
pub const MQTT_PASSWORD: &str = "lovelace";

/// A real broker, not a stand-in: rumqttd is a library, so the mqtt tests speak
/// the wire protocol to something that actually implements it.
pub struct MqttBroker {
    tcp: SocketAddr,
    websocket: SocketAddr,
}

/// The broker binds the port itself, so a free one is picked here and released.
/// Two tests racing for the same port is the trade for not patching rumqttd.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("free port");
    let port = listener.local_addr().expect("free port").port();
    drop(listener);
    port
}

fn connections(auth: Option<HashMap<String, String>>) -> ConnectionSettings {
    ConnectionSettings {
        connection_timeout_ms: 5_000,
        max_payload_size: 2 * 1024 * 1024,
        max_inflight_count: 100,
        auth,
        external_auth: None,
        dynamic_filters: true,
    }
}

/// Tests want loopback so nothing else on the machine can reach the broker; the
/// hosted mock has to answer its platform's proxy, which never comes from
/// loopback. `BIND` is the same variable the HTTP server reads.
fn listen_addr() -> IpAddr {
    std::env::var("BIND")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn server(name: &str, port: u16, auth: Option<HashMap<String, String>>) -> ServerSettings {
    ServerSettings {
        name: name.to_string(),
        listen: SocketAddr::new(listen_addr(), port),
        tls: None,
        next_connection_delay_ms: 1,
        connections: connections(auth),
    }
}

impl MqttBroker {
    pub async fn start() -> MqttBroker {
        MqttBroker::start_with(None).await
    }

    pub async fn start_with_credentials() -> MqttBroker {
        MqttBroker::start_with(Some(
            [(MQTT_USER.to_string(), MQTT_PASSWORD.to_string())]
                .into_iter()
                .collect(),
        ))
        .await
    }

    pub async fn start_with(auth: Option<HashMap<String, String>>) -> MqttBroker {
        MqttBroker::start_on(free_port(), free_port(), auth).await
    }

    /// A broker on ports the caller picked. The long-running mock needs a port a
    /// saved `.mqtt` file can name; the tests need one nothing else can take.
    pub async fn start_on(
        tcp_port: u16,
        ws_port: u16,
        auth: Option<HashMap<String, String>>,
    ) -> MqttBroker {
        let config = Config {
            id: 0,
            router: RouterConfig {
                max_connections: 100,
                max_outgoing_packet_count: 1_000,
                max_segment_size: 1024 * 1024,
                max_segment_count: 10,
                ..RouterConfig::default()
            },
            v4: Some(
                [("tcp".to_string(), server("tcp", tcp_port, auth.clone()))]
                    .into_iter()
                    .collect(),
            ),
            ws: Some(
                [("ws".to_string(), server("ws", ws_port, auth))]
                    .into_iter()
                    .collect(),
            ),
            ..Config::default()
        };

        let mut broker = Broker::new(config);
        std::thread::Builder::new()
            .name("mandalo-mock-mqtt".to_string())
            .spawn(move || {
                let _ = broker.start();
            })
            .expect("start the mock broker");

        let broker = MqttBroker {
            tcp: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), tcp_port),
            websocket: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), ws_port),
        };
        broker.wait_until_listening().await;
        broker
    }

    async fn wait_until_listening(&self) {
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(self.tcp).await.is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!(
            "the mock mqtt broker never started listening on {}",
            self.tcp
        );
    }

    pub fn addr(&self) -> SocketAddr {
        self.tcp
    }

    pub fn url(&self) -> String {
        format!("mqtt://{}", self.tcp)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.websocket)
    }
}
