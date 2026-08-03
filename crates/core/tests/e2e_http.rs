use mandalo_core::assertions::{StatusOp, TestAssertion};
use mandalo_core::request::Auth;
use mandalo_core::runner::{StepResult, VarFrame};
use mandalo_testkit::fixtures::{request, runner};
use mandalo_testkit::{MockApi, API_KEY_NAME, API_KEY_VALUE, BASIC_PASSWORD, BASIC_USER, TOKEN};

trait BodyJson {
    fn body_json(&self) -> serde_json::Value;
}

impl BodyJson for mandalo_core::request::ResponseData {
    fn body_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

async fn send(req: &mandalo_core::SavedRequest) -> StepResult {
    runner()
        .run_request(req, &mut VarFrame::default())
        .await
        .expect("the request reached the mock")
}

#[tokio::test]
async fn every_method_reaches_the_mock_with_its_own_verb() {
    let api = MockApi::start().await;
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] {
        let path = format!("/{}", method.to_ascii_lowercase());
        let mut req = request(method, method, &api.url(&path));
        if matches!(method, "POST" | "PUT" | "PATCH") {
            req.body = mandalo_core::Body::json("{\"hello\":\"mock\"}");
            req.headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        }
        let step = send(&req).await;
        let response = step.response.expect("a response");
        assert_eq!(response.status, 200, "{method} {path}");

        let received = api.requests_to(&path);
        assert_eq!(received.len(), 1, "{method} {path}");
        assert_eq!(received[0].method, method);
        if matches!(method, "POST" | "PUT" | "PATCH") {
            assert_eq!(received[0].body, "{\"hello\":\"mock\"}");
        }
        if method != "HEAD" {
            assert_eq!(response.body_json()["method"], method);
        }
    }
}

#[tokio::test]
async fn query_params_and_custom_headers_reach_the_wire() {
    let api = MockApi::start().await;
    let mut req = request("Search", "GET", &api.url("/get?page=2&q=hola%20mundo"));
    req.headers = vec![
        ("X-Trace".to_string(), "t-1".to_string()),
        ("X-Tenant".to_string(), "acme".to_string()),
    ];

    let step = send(&req).await;

    let received = api.last_request().expect("a recorded request");
    assert_eq!(received.query.get("page").map(String::as_str), Some("2"));
    assert_eq!(
        received.query.get("q").map(String::as_str),
        Some("hola mundo")
    );
    assert_eq!(received.header("x-trace"), Some("t-1"));
    assert_eq!(received.header("x-tenant"), Some("acme"));
    assert!(step.passed);
}

#[tokio::test]
async fn bearer_auth_passes_its_guard_and_a_wrong_token_is_a_401() {
    let api = MockApi::start().await;
    let mut req = request("Bearer", "GET", &api.url("/auth/bearer"));
    req.auth = Auth::Bearer {
        token: TOKEN.to_string(),
    };
    let ok = send(&req).await.response.expect("a response");
    assert_eq!(ok.status, 200);
    assert_eq!(ok.body_json()["authenticated"], true);

    req.auth = Auth::Bearer {
        token: "wrong".to_string(),
    };
    let denied = send(&req).await.response.expect("a response");
    assert_eq!(denied.status, 401);
    assert_eq!(denied.body_json()["error"], "unauthorized");
}

#[tokio::test]
async fn basic_auth_passes_its_guard_and_wrong_credentials_are_a_401() {
    let api = MockApi::start().await;
    let mut req = request("Basic", "GET", &api.url("/auth/basic"));
    req.auth = Auth::Basic {
        username: BASIC_USER.to_string(),
        password: BASIC_PASSWORD.to_string(),
    };
    assert_eq!(send(&req).await.response.expect("a response").status, 200);

    req.auth = Auth::Basic {
        username: BASIC_USER.to_string(),
        password: "nope".to_string(),
    };
    assert_eq!(send(&req).await.response.expect("a response").status, 401);
}

#[tokio::test]
async fn apikey_auth_works_in_the_header_and_in_the_query() {
    let api = MockApi::start().await;
    let mut req = request("Api key", "GET", &api.url("/auth/apikey"));
    for placement in ["header", "query"] {
        req.auth = Auth::Apikey {
            key: API_KEY_NAME.to_string(),
            value: API_KEY_VALUE.to_string(),
            placement: placement.to_string(),
        };
        let response = send(&req).await.response.expect("a response");
        assert_eq!(response.status, 200, "{placement}");
        assert_eq!(response.body_json()["placement"], placement);
    }

    req.auth = Auth::Apikey {
        key: API_KEY_NAME.to_string(),
        value: "wrong".to_string(),
        placement: "header".to_string(),
    };
    assert_eq!(send(&req).await.response.expect("a response").status, 401);
}

#[tokio::test]
async fn auth_replaces_a_user_authorization_header_instead_of_duplicating_it() {
    let api = MockApi::start().await;
    let mut req = request("No duplicates", "GET", &api.url("/headers/echo"));
    req.headers = vec![
        ("Authorization".to_string(), "stale".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    req.auth = Auth::Bearer {
        token: TOKEN.to_string(),
    };

    send(&req).await;

    let received = api.last_request().expect("a recorded request");
    assert_eq!(
        received.headers_named("authorization"),
        vec![format!("Bearer {TOKEN}")]
    );
    assert_eq!(
        received.headers_named("content-type"),
        vec!["application/json"]
    );
}

#[tokio::test]
async fn a_non_2xx_status_is_a_result_not_an_error() {
    let api = MockApi::start().await;
    for code in [200u16, 201, 204, 400, 401, 404, 418, 500] {
        let req = request("Status", "GET", &api.url(&format!("/status/{code}")));
        let step = send(&req).await;
        assert!(step.error.is_none(), "{code} became an error");
        assert_eq!(step.response.expect("a response").status, code);
    }
}

#[tokio::test]
async fn a_3xx_status_is_followed_to_its_target() {
    let api = MockApi::start().await;
    let step = send(&request("Moved", "GET", &api.url("/status/301"))).await;
    let response = step.response.expect("a response");
    assert_eq!(response.status, 200);
    assert_eq!(response.body_json()["path"], "/get");
}

#[tokio::test]
async fn a_redirect_chain_is_followed_and_the_hop_cap_fails_loud() {
    let api = MockApi::start().await;
    let step = send(&request("Chain", "GET", &api.url("/redirect/3"))).await;
    let response = step.response.expect("a response");
    assert_eq!(response.status, 200);
    assert_eq!(response.body_json()["path"], "/get");
    assert_eq!(api.requests_to("/get").len(), 1);

    let error = runner()
        .run_request(
            &request("Too many", "GET", &api.url("/redirect/10")),
            &mut VarFrame::default(),
        )
        .await
        .expect_err("11 hops is over the reqwest redirect cap");
    assert_eq!(error.code(), "E_NETWORK");
    assert!(
        error.to_string().to_lowercase().contains("redirect"),
        "{error}"
    );
}

#[tokio::test]
async fn a_cross_host_redirect_strips_the_authorization_header() {
    let api = MockApi::start().await;
    let other = MockApi::start().await;
    let mut req = request(
        "Cross host",
        "GET",
        &api.url(&format!(
            "/redirect-to?url={}",
            other.url("/headers/echo").replace(':', "%3A")
        )),
    );
    req.auth = Auth::Bearer {
        token: TOKEN.to_string(),
    };

    let step = send(&req).await;
    assert_eq!(step.response.expect("a response").status, 200);

    let first = api.last_request().expect("the first hop");
    assert_eq!(
        first.headers_named("authorization"),
        vec![format!("Bearer {TOKEN}")]
    );
    let second = other.last_request().expect("the second hop");
    assert!(
        second.headers_named("authorization").is_empty(),
        "the token survived a cross-host redirect: {:?}",
        second.headers
    );
}

#[tokio::test]
async fn gzip_and_brotli_bodies_are_decoded() {
    let api = MockApi::start().await;
    for (path, encoding) in [("/gzip", "gzip"), ("/brotli", "brotli")] {
        let step = send(&request(encoding, "GET", &api.url(path))).await;
        let response = step.response.expect("a response");
        assert_eq!(response.status, 200);
        assert_eq!(response.body_json()["encoding"], encoding);
        assert!(!response.binary);
    }
}

#[tokio::test]
async fn a_non_utf8_body_is_flagged_binary_with_its_wire_size() {
    let api = MockApi::start().await;
    let step = send(&request("Binary", "GET", &api.url("/binary"))).await;
    let response = step.response.expect("a response");
    assert!(response.binary);
    assert_eq!(response.size_bytes, 6);
    assert!(response.body.contains('\u{fffd}'));
}

#[tokio::test]
async fn an_empty_body_is_a_204_with_no_bytes() {
    let api = MockApi::start().await;
    let step = send(&request("Empty", "GET", &api.url("/empty"))).await;
    let response = step.response.expect("a response");
    assert_eq!(response.status, 204);
    assert_eq!(response.size_bytes, 0);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn a_large_body_arrives_whole() {
    let api = MockApi::start().await;
    let step = send(&request("Big", "GET", &api.url("/big?bytes=250000"))).await;
    let response = step.response.expect("a response");
    assert_eq!(
        response.body_json()["filler"].as_str().map(str::len),
        Some(250_000)
    );
    assert!(response.size_bytes >= 250_000);
}

#[tokio::test]
async fn a_slow_response_is_measured_and_still_arrives() {
    let api = MockApi::start().await;
    let mut req = request("Slow", "GET", &api.url("/slow?ms=300"));
    req.tests = vec![TestAssertion::Status {
        op: StatusOp::Eq,
        value: 200,
    }];

    let step = send(&req).await;

    let response = step.response.expect("a response");
    assert!(response.duration_ms >= 300, "{}ms", response.duration_ms);
    assert!(step.passed);
}

#[tokio::test]
async fn a_set_cookie_header_is_surfaced_and_not_replayed() {
    let api = MockApi::start().await;
    let step = send(&request("Cookie", "GET", &api.url("/cookies/set"))).await;
    let response = step.response.expect("a response");
    assert!(response
        .headers
        .iter()
        .any(|(k, v)| k == "set-cookie" && v.contains("mock_session=abc123")));

    send(&request("After cookie", "GET", &api.url("/get"))).await;
    let followup = api.last_request().expect("a recorded request");
    assert_eq!(followup.header("cookie"), None);
}

#[tokio::test]
async fn a_refused_port_is_a_network_error() {
    let error = runner()
        .run_request(
            &request("Refused", "GET", "http://127.0.0.1:1/nope"),
            &mut VarFrame::default(),
        )
        .await
        .expect_err("nothing listens on port 1");
    assert_eq!(error.code(), "E_NETWORK");
}
