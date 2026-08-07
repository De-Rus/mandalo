use mandalo_testkit::{MockApi, MqttBroker, Options};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

const DEFAULT_PORT: u16 = 8787;
const DEFAULT_GRPC_PORT: u16 = 50051;
const DEFAULT_MQTT_PORT: u16 = 1883;

fn flag(args: &[String], name: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
        if arg == name {
            return iter.next().cloned();
        }
    }
    None
}

fn port(args: &[String], flag_name: &str, env_name: &str, fallback: u16) -> u16 {
    flag(args, flag_name)
        .or_else(|| std::env::var(env_name).ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

/// A predictable path, because a saved gRPC request stores it literally — `protoPaths`
/// is the one request field the runner does not interpolate.
fn proto_dir(args: &[String]) -> PathBuf {
    if let Some(dir) = flag(args, "--proto-dir").or_else(|| std::env::var("PROTO_DIR").ok()) {
        return PathBuf::from(dir);
    }
    let tmp = PathBuf::from("/tmp");
    if tmp.is_dir() {
        tmp.join("mandalo-mock")
    } else {
        std::env::temp_dir().join("mandalo-mock")
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let http_port = port(&args, "--port", "PORT", DEFAULT_PORT);
    let bind: IpAddr = flag(&args, "--bind")
        .or_else(|| std::env::var("BIND").ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let requests_per_minute = flag(&args, "--rate-limit")
        .or_else(|| std::env::var("RATE_LIMIT_PER_MINUTE").ok())
        .and_then(|v| v.parse().ok())
        .or_else(|| (!bind.is_loopback()).then_some(600));

    let api = MockApi::start_with(Options {
        bind,
        http_port,
        grpc_port: port(&args, "--grpc-port", "GRPC_PORT", DEFAULT_GRPC_PORT),
        log: true,
        proto_dir: Some(proto_dir(&args)),
        requests_per_minute,
    })
    .await;

    let mqtt_port = port(&args, "--mqtt-port", "MQTT_PORT", DEFAULT_MQTT_PORT);
    let mqtt = MqttBroker::start_on(mqtt_port, mqtt_port + 1, None).await;

    println!("{}", api.index());
    println!("\n  MQTT  {}  (websocket {})", mqtt.url(), mqtt.ws_url());
    if let Some(limit) = requests_per_minute {
        println!("rate limit {limit} requests per minute per client address");
    }
    println!("\nctrl-c to stop\n");

    tokio::signal::ctrl_c().await.expect("listen for ctrl-c");
    println!("\nstopped");
}
