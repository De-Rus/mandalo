use std::io::{Read, Write};
use std::net::TcpListener;

fn spawn_server_bytes(response: Vec<u8>) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 8192];
        let mut received = Vec::new();
        loop {
            let n = stream.read(&mut buf).unwrap();
            received.extend_from_slice(&buf[..n]);
            if n == 0 || received.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        stream.write_all(&response).unwrap();
        tx.send(String::from_utf8_lossy(&received).to_string()).unwrap();
    });
    (format!("http://{addr}"), rx)
}

fn spawn_server(response: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
    spawn_server_bytes(response.as_bytes().to_vec())
}

#[tokio::test]
async fn send_request_hits_local_server_and_applies_auth() {
    let (url, rx) = spawn_server(
        "HTTP/1.1 418 I'm a teapot\r\nContent-Length: 2\r\nX-Flavor: mate\r\nConnection: close\r\n\r\nok",
    );
    let spec = serde_json::from_value(serde_json::json!({
        "kind": "http",
        "method": "GET",
        "url": format!("{url}/{{{{path}}}}"),
        "headers": [["X-Trace", "{{trace}}"]],
        "auth": {"type": "bearer", "token": "{{token}}"},
        "vars": {"path": "brew", "trace": "t-1", "token": "secreto"}
    }))
    .unwrap();

    let resp = mandalo_lib::request::send_request(spec).await.unwrap();

    assert_eq!(resp.status, 418);
    assert_eq!(resp.status_text, "I'm a teapot");
    assert_eq!(resp.body, "ok");
    assert!(!resp.binary);
    assert_eq!(resp.size_bytes, 2);
    assert!(resp
        .headers
        .iter()
        .any(|(k, v)| k == "x-flavor" && v == "mate"));

    let raw = rx.recv().unwrap();
    assert!(raw.starts_with("GET /brew HTTP/1.1\r\n"));
    assert!(raw.contains("authorization: Bearer secreto\r\n"));
    assert!(raw.contains("x-trace: t-1\r\n"));
}

#[tokio::test]
async fn status_text_keeps_the_wire_reason_phrase() {
    let (url, _rx) = spawn_server(
        "HTTP/1.1 500 Todo Explotado\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    let spec = serde_json::from_value(serde_json::json!({
        "kind": "http", "method": "GET", "url": url
    }))
    .unwrap();

    let resp = mandalo_lib::request::send_request(spec).await.unwrap();

    assert_eq!(resp.status, 500);
    assert_eq!(resp.status_text, "Todo Explotado");
}

#[tokio::test]
async fn non_utf8_body_is_flagged_binary_with_wire_size() {
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
            .to_vec();
    response.extend_from_slice(&[0xff, 0xfe, 0x01, 0x02]);
    let (url, _rx) = spawn_server_bytes(response);
    let spec = serde_json::from_value(serde_json::json!({
        "kind": "http", "method": "GET", "url": url
    }))
    .unwrap();

    let resp = mandalo_lib::request::send_request(spec).await.unwrap();

    assert_eq!(resp.status, 200);
    assert!(resp.binary);
    assert_eq!(resp.size_bytes, 4);
    assert!(resp.body.contains('\u{fffd}'));
}
