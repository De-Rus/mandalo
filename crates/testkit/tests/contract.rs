use mandalo_testkit::MockApi;
use serde_json::Value;

fn contract() -> Value {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/contract.json"))
        .expect("contract.json");
    serde_json::from_str(&raw).expect("contract.json is valid JSON")
}

fn contains(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected
            .iter()
            .all(|(k, v)| actual.get(k).map(|a| contains(a, v)).unwrap_or(false)),
        (Value::Array(actual), Value::Array(expected)) => {
            expected.len() <= actual.len()
                && expected
                    .iter()
                    .zip(actual.iter())
                    .all(|(e, a)| contains(a, e))
        }
        _ => actual == expected,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rust_mock_honours_every_case_in_the_contract() {
    let api = MockApi::start().await;
    let contract = contract();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client");
    let redirecting = reqwest::Client::new();

    for case in contract["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("a case name");
        let method =
            reqwest::Method::from_bytes(case["method"].as_str().expect("a method").as_bytes())
                .expect("a valid method");
        let url = format!(
            "{}{}",
            api.base_url(),
            case["path"].as_str().expect("a path")
        );
        let follow = case["followRedirects"].as_bool().unwrap_or(true);
        let mut request = if follow {
            redirecting.request(method, &url)
        } else {
            client.request(method, &url)
        };
        if let Some(headers) = case["headers"].as_object() {
            for (key, value) in headers {
                request = request.header(key, value.as_str().expect("a header value"));
            }
        }
        if let Some(body) = case["body"].as_str() {
            request = request.body(body.to_string());
        }

        let response = request
            .send()
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let body = response.text().await.unwrap_or_default();

        assert_eq!(
            status,
            case["status"].as_u64().expect("a status") as u16,
            "{name}: body was {body}"
        );

        if let Some(expected) = case.get("json").filter(|v| !v.is_null()) {
            let actual: Value =
                serde_json::from_str(&body).unwrap_or_else(|e| panic!("{name}: {e} in {body}"));
            assert!(
                contains(&actual, expected),
                "{name}: {actual} lacks {expected}"
            );
        }
        if let Some(fragment) = case["bodyContains"].as_str() {
            assert!(body.contains(fragment), "{name}: {body}");
        }
        if let Some(exact) = case["bodyEquals"].as_str() {
            assert_eq!(body, exact, "{name}");
        }
        if let Some(max) = case["maxBytes"].as_u64() {
            assert!(body.len() as u64 <= max, "{name}: {} bytes", body.len());
        }
        if let Some(pair) = case["headerEcho"].as_array() {
            let echoed: Value = serde_json::from_str(&body).expect("json body");
            let wanted = serde_json::json!([pair[0], pair[1]]);
            assert!(
                echoed["headers"]
                    .as_array()
                    .expect("headers")
                    .contains(&wanted),
                "{name}: {echoed}"
            );
        }
        if let Some(pair) = case["responseHeaderContains"].as_array() {
            let key = pair[0].as_str().expect("a header name");
            let fragment = pair[1].as_str().expect("a header fragment");
            assert!(
                headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case(key) && v.contains(fragment)),
                "{name}: {headers:?}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_response_carries_the_cors_headers_a_browser_needs() {
    let api = MockApi::start().await;
    let client = reqwest::Client::new();

    let response = client
        .get(api.url("/get"))
        .header("origin", "https://mandalo.dev")
        .send()
        .await
        .expect("a response");
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-expose-headers")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );

    let preflight = client
        .request(reqwest::Method::OPTIONS, api.url("/post"))
        .header("origin", "https://mandalo.dev")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "authorization")
        .send()
        .await
        .expect("a preflight response");
    assert_eq!(preflight.status(), 204);
    assert!(preflight
        .headers()
        .get("access-control-allow-methods")
        .expect("allowed methods")
        .to_str()
        .expect("ascii")
        .contains("PATCH"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_options_request_still_reaches_the_options_echo_route() {
    let api = MockApi::start().await;

    let response = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, api.url("/options"))
        .send()
        .await
        .expect("a response");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json body");
    assert_eq!(body["method"], "OPTIONS");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rate_limiter_answers_429_once_the_bucket_is_empty() {
    let api = MockApi::start_with(mandalo_testkit::Options {
        requests_per_minute: Some(3),
        ..Default::default()
    })
    .await;
    let client = reqwest::Client::new();

    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(
            client
                .get(api.url("/get"))
                .send()
                .await
                .expect("a response")
                .status()
                .as_u16(),
        );
    }

    assert_eq!(statuses, vec![200, 200, 200, 429, 429]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_over_the_cap_is_refused_with_413() {
    let api = MockApi::start().await;

    let response = reqwest::Client::new()
        .post(api.url("/post"))
        .body("x".repeat(mandalo_testkit::MAX_BODY_BYTES + 1))
        .send()
        .await
        .expect("a response");

    assert_eq!(response.status(), 413);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_generative_endpoints_are_clamped() {
    let api = MockApi::start().await;
    let client = reqwest::Client::new();

    let big = client
        .get(api.url("/big?bytes=999999999"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        big.len() <= mandalo_testkit::MAX_BIG_BYTES + 32,
        "{}",
        big.len()
    );

    let started = std::time::Instant::now();
    client
        .get(api.url("/slow?ms=999999"))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .expect("a response");
    assert!(started.elapsed().as_millis() <= 12_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shipped_proto_matches_the_one_the_mock_writes() {
    let api = MockApi::start().await;
    let written = std::fs::read_to_string(api.proto_path()).expect("mock.proto");
    assert_eq!(written, mandalo_testkit::PROTO);
}

#[test]
fn the_example_workspace_proto_matches_the_one_the_mock_serves() {
    let shipped = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/mock-workspace/protos/mock.proto"
    ))
    .expect("examples/mock-workspace/protos/mock.proto");
    assert_eq!(
        shipped,
        mandalo_testkit::PROTO,
        "the browser build seeds this copy, so it may not drift from the mock's own proto"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_grpc_port_also_speaks_grpc_web_for_the_browser_build() {
    let api = MockApi::start().await;
    let frame = [
        0x00, 0x00, 0x00, 0x00, 0x06, 0x0a, 0x04, b'h', b'o', b'l', b'a',
    ];

    let response = reqwest::Client::new()
        .post(format!("{}/mock.v1.Mock/Say", api.grpc_url()))
        .header("content-type", "application/grpc-web+proto")
        .header("x-trace", "web-1")
        .body(frame.to_vec())
        .send()
        .await
        .expect("a grpc-web response");

    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/grpc-web+proto")
    );
    let body = String::from_utf8_lossy(&response.bytes().await.expect("a body")).into_owned();
    assert!(body.contains("hola"), "{body:?}");
    assert!(body.contains("web-1"), "{body:?}");
    assert!(body.contains("grpc-status:0"), "{body:?}");
}
