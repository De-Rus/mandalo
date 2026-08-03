use assert_cmd::Command;
use mandalo_testkit::MockApi;
use serde_json::Value;

fn run(args: Vec<String>) -> (bool, String, String) {
    let output = Command::cargo_bin("mandalo")
        .expect("the mandalo binary")
        .args(args)
        .output()
        .expect("run mandalo listen");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn listening_to_an_event_stream_prints_every_event_and_stops_at_max() {
    let mock = MockApi::start().await;
    let url = mock.url("/sse/basic?n=5");
    let (ok, stdout, stderr) = tokio::task::spawn_blocking(move || {
        run(vec![
            "listen".to_string(),
            url,
            "--max".to_string(),
            "2".to_string(),
            "--json".to_string(),
        ])
    })
    .await
    .unwrap();

    assert!(ok, "listen should exit zero: {stderr}");
    let events: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every line is one json event"))
        .collect();
    let types: Vec<&str> = events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(types, vec!["connecting", "connected", "message", "message"]);
    assert_eq!(events[2]["payload"]["text"], "message 0");
    assert_eq!(events[2]["direction"], "incoming");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_websocket_echo_round_trip_is_visible_in_the_readable_log() {
    let mock = MockApi::start().await;
    let url = format!("ws://{}/ws/echo", mock.addr());
    let (ok, stdout, stderr) = tokio::task::spawn_blocking(move || {
        run(vec![
            "listen".to_string(),
            url,
            "--send".to_string(),
            "hola".to_string(),
            "--max".to_string(),
            "1".to_string(),
        ])
    })
    .await
    .unwrap();

    assert!(ok, "listen should exit zero: {stderr}");
    assert!(stdout.contains("connected"), "{stdout}");
    assert!(stdout.contains("out  hola"), "{stdout}");
    assert!(stdout.contains("in   hola"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_that_cannot_connect_exits_non_zero_and_says_why() {
    let (ok, stdout, _stderr) = tokio::task::spawn_blocking(|| {
        run(vec![
            "listen".to_string(),
            "ws://127.0.0.1:1/socket".to_string(),
        ])
    })
    .await
    .unwrap();

    assert!(!ok, "a refused connection must fail the command");
    assert!(
        stdout.contains("nothing is listening on 127.0.0.1:1"),
        "{stdout}"
    );
}

#[test]
fn an_unknown_scheme_is_refused_before_anything_opens() {
    let (ok, _stdout, stderr) = run(vec!["listen".to_string(), "amqp://x.dev".to_string()]);
    assert!(!ok);
    assert!(stderr.contains("--kind"), "{stderr}");
}

#[test]
fn sending_on_an_event_stream_is_refused() {
    let (ok, _stdout, stderr) = run(vec![
        "listen".to_string(),
        "https://x.dev/events".to_string(),
        "--send".to_string(),
        "nope".to_string(),
    ]);
    assert!(!ok);
    assert!(stderr.contains("from the server to the client"), "{stderr}");
}

#[test]
fn publishing_without_a_topic_is_refused() {
    let (ok, _stdout, stderr) = run(vec![
        "listen".to_string(),
        "mqtt://broker.dev".to_string(),
        "--send".to_string(),
        "21.5".to_string(),
    ]);
    assert!(!ok);
    assert!(stderr.contains("--topic"), "{stderr}");
}
