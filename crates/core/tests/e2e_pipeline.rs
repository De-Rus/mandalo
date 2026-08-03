use mandalo_core::assertions::{
    Capture, CaptureScope, DurationOp, HeaderOp, JsonOp, StatusOp, TestAssertion,
};
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::request::Auth;
use mandalo_core::runner::{Runner, VarFrame};
use mandalo_core::{NoSecrets, StrictPolicy};
use mandalo_testkit::fixtures::{environment, request, runner, workspace};
use mandalo_testkit::{MockApi, BASIC_PASSWORD, BASIC_USER, TOKEN};

fn login() -> SavedRequest {
    let mut req = request("Login", "POST", "{{baseUrl}}/auth/login");
    req.headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    req.body = mandalo_core::Body::json(format!(
        "{{\"username\": \"{BASIC_USER}\", \"password\": \"{BASIC_PASSWORD}\"}}"
    ));
    req.scripts.post = Some(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        pm.environment.set("token", pm.response.json().token);
        "#
        .to_string(),
    );
    req
}

#[tokio::test]
async fn a_pre_script_mutation_is_what_the_server_actually_receives() {
    let api = MockApi::start().await;
    let mut req = request("Patched", "GET", &api.url("/get"));
    req.scripts.pre = Some(format!(
        r#"
        pm.request.method = "POST";
        pm.request.url = "{}";
        pm.request.headers.upsert({{ key: "X-Patched", value: "yes" }});
        pm.request.body = '{{"from":"pre"}}';
        pm.environment.set("greeting", "hola");
        "#,
        api.url("/post")
    ));

    let mut vars = VarFrame::default();
    let step = runner()
        .run_request(&req, &mut vars)
        .await
        .expect("the request reached the mock");

    let received = api.last_request().expect("a recorded request");
    assert_eq!(received.method, "POST");
    assert_eq!(received.path, "/post");
    assert_eq!(received.header("x-patched"), Some("yes"));
    assert_eq!(received.body, "{\"from\":\"pre\"}");
    assert_eq!(vars.get("greeting"), Some("hola"));
    assert!(step.passed);
}

#[tokio::test]
async fn post_script_tests_land_in_the_step_and_variable_writes_propagate() {
    let api = MockApi::start().await;
    let mut req = request("Login", "POST", &api.url("/auth/login"));
    req.headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    req.body = mandalo_core::Body::json(format!(
        "{{\"username\": \"{BASIC_USER}\", \"password\": \"{BASIC_PASSWORD}\"}}"
    ));
    req.scripts.post = Some(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        pm.test("token is long", function () { pm.expect(pm.response.json().token.length).to.be.above(500); });
        console.log("logged in as", pm.response.json().user.name);
        pm.environment.set("token", pm.response.json().token);
        "#
        .to_string(),
    );

    let mut vars = VarFrame::default();
    let step = runner()
        .run_request(&req, &mut vars)
        .await
        .expect("the request reached the mock");

    assert_eq!(step.script_tests.len(), 2);
    assert!(step.script_tests[0].passed);
    assert!(!step.script_tests[1].passed);
    assert!(!step.passed);
    assert_eq!(step.logs, vec!["logged in as Ada Lovelace"]);
    assert_eq!(vars.get("token"), Some(TOKEN));
}

#[tokio::test]
async fn every_declarative_assertion_kind_passes_and_fails_against_a_real_response() {
    let api = MockApi::start().await;
    let mut req = request("Json", "GET", &api.url("/json"));
    req.tests = vec![
        TestAssertion::Status {
            op: StatusOp::Eq,
            value: 200,
        },
        TestAssertion::Json {
            path: "$.nested.ok".to_string(),
            op: JsonOp::Eq,
            value: Some(serde_json::json!(true)),
        },
        TestAssertion::Header {
            name: "content-type".to_string(),
            op: HeaderOp::Contains,
            value: Some("application/json".to_string()),
        },
        TestAssertion::Duration {
            op: DurationOp::Lt,
            value: 10_000,
        },
    ];

    let step = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("the request reached the mock");
    assert!(step.passed, "{:?}", step.failures());
    assert_eq!(step.tests.len(), 4);

    req.tests = vec![
        TestAssertion::Status {
            op: StatusOp::Eq,
            value: 404,
        },
        TestAssertion::Json {
            path: "$.name".to_string(),
            op: JsonOp::Eq,
            value: Some(serde_json::json!("someone else")),
        },
        TestAssertion::Header {
            name: "x-missing".to_string(),
            op: HeaderOp::Exists,
            value: None,
        },
        TestAssertion::Duration {
            op: DurationOp::Gt,
            value: 60_000,
        },
    ];
    let failed = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("the request reached the mock");
    assert!(!failed.passed);
    assert_eq!(failed.failures().len(), 4);
}

#[tokio::test]
async fn captures_read_from_the_body_the_headers_and_the_status() {
    let api = MockApi::start().await;
    let mut req = request("Cookie", "GET", &api.url("/cookies/set"));
    req.captures = vec![
        Capture {
            from: "body.$.cookie".to_string(),
            into: "cookie".to_string(),
            scope: CaptureScope::Run,
        },
        Capture {
            from: "header.content-type".to_string(),
            into: "content_type".to_string(),
            scope: CaptureScope::Run,
        },
        Capture {
            from: "status".to_string(),
            into: "code".to_string(),
            scope: CaptureScope::Run,
        },
    ];

    let mut vars = VarFrame::default();
    let step = runner()
        .run_request(&req, &mut vars)
        .await
        .expect("the request reached the mock");

    assert_eq!(
        step.captured.get("cookie").map(String::as_str),
        Some("mock_session=abc123")
    );
    assert_eq!(vars.get("code"), Some("200"));
    assert!(vars
        .get("content_type")
        .expect("captured content type")
        .contains("application/json"));
}

#[tokio::test]
async fn a_capture_that_matches_nothing_fails_loud() {
    let api = MockApi::start().await;
    let mut req = request("Json", "GET", &api.url("/json"));
    req.captures = vec![Capture {
        from: "body.$.ghost".to_string(),
        into: "ghost".to_string(),
        scope: CaptureScope::Run,
    }];

    let error = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("nothing matches that path");

    assert_eq!(error.code(), "E_CAPTURE");
}

#[tokio::test]
async fn a_captured_token_authenticates_the_next_request_of_the_suite() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    environment(dir.path(), "local", &[("baseUrl", &api.base_url())]);

    let mut authenticated = request("Me", "GET", "{{baseUrl}}/auth/bearer");
    authenticated.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    authenticated.scripts.post = Some(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        pm.test("the token was accepted", function () {
          pm.expect(pm.response.json().authenticated).to.equal(true);
        });
        "#
        .to_string(),
    );
    collection::put_request(dir.path(), &slug, "1-login.http", &login()).expect("save login");
    collection::put_request(dir.path(), &slug, "2-me.http", &authenticated).expect("save me");

    let report = runner()
        .run_suite(dir.path(), &slug, None, Some("local"))
        .await
        .expect("the suite runs");

    assert_eq!(report.total, 2);
    assert_eq!(report.failed, 0, "{:?}", report.steps);
    assert!(report.passed);
    assert_eq!(
        report.steps[0].var_sets.get("token").map(String::as_str),
        Some(TOKEN)
    );

    let guarded = api
        .requests_to("/auth/bearer")
        .pop()
        .expect("the guarded call");
    assert_eq!(
        guarded.headers_named("authorization"),
        vec![format!("Bearer {TOKEN}")]
    );
}

#[tokio::test]
async fn a_suite_runs_every_request_and_counts_what_it_ran() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    for (index, path) in ["1-first.http", "2-second.http", "3-third.http"]
        .iter()
        .enumerate()
    {
        let req = request(
            &format!("Step {index}"),
            "GET",
            &api.url(&format!("/get?step={index}")),
        );
        collection::put_request(dir.path(), &slug, path, &req).expect("save request");
    }

    let report = runner()
        .run_suite(dir.path(), &slug, None, None)
        .await
        .expect("the suite runs");

    assert_eq!(report.total, 3);
    assert!(report.passed);
    let order: Vec<String> = api
        .requests_to("/get")
        .iter()
        .map(|r| r.query.get("step").cloned().unwrap_or_default())
        .collect();
    assert_eq!(order, vec!["0", "1", "2"]);
}

#[tokio::test]
async fn fail_fast_stops_at_the_first_failure_and_the_default_runs_everything() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    let mut failing = request("1 Failing", "GET", &api.url("/status/500"));
    failing.scripts.post = Some(
        r#"pm.test("status is 200", function () { pm.response.to.have.status(200); });"#
            .to_string(),
    );
    collection::put_request(dir.path(), &slug, "1-fail.http", &failing).expect("save request");
    collection::put_request(
        dir.path(),
        &slug,
        "2-after.http",
        &request("2 After", "GET", &api.url("/get")),
    )
    .expect("save request");

    let stopped = runner()
        .run_suite_with(dir.path(), &slug, None, None, true)
        .await
        .expect("the suite runs");
    assert_eq!(stopped.total, 1);
    assert_eq!(stopped.failed, 1);
    assert!(!stopped.passed);

    let full = runner()
        .run_suite_with(dir.path(), &slug, None, None, false)
        .await
        .expect("the suite runs");
    assert_eq!(full.total, 2);
    assert_eq!(full.failed, 1);
    assert!(full.steps[1].passed);
}

#[tokio::test]
async fn a_transport_failure_is_a_failing_step_and_the_suite_keeps_going() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    collection::put_request(
        dir.path(),
        &slug,
        "1-broken.http",
        &request("Broken", "GET", "http://127.0.0.1:1/nope"),
    )
    .expect("save request");
    collection::put_request(
        dir.path(),
        &slug,
        "2-fine.http",
        &request("Fine", "GET", &api.url("/get")),
    )
    .expect("save request");

    let report = runner()
        .run_suite(dir.path(), &slug, None, None)
        .await
        .expect("the suite runs");

    assert_eq!(report.total, 2);
    assert_eq!(report.failed, 1);
    assert!(!report.passed);
    assert_eq!(report.steps[0].error_code.as_deref(), Some("E_NETWORK"));
    assert!(report.steps[1].passed);
}

/// The `1-login` / `2-orders` convention the product documents only works when run
/// order follows the file name; sorting by display name broke a capture chain the
/// moment a request was renamed.
#[tokio::test]
async fn a_suite_runs_in_file_name_order() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    collection::put_request(
        dir.path(),
        &slug,
        "1-login.http",
        &request("Zebra", "GET", &api.url("/get?step=first")),
    )
    .expect("save request");
    collection::put_request(
        dir.path(),
        &slug,
        "2-orders.http",
        &request("Alpha", "GET", &api.url("/get?step=second")),
    )
    .expect("save request");

    let report = runner()
        .run_suite(dir.path(), &slug, None, None)
        .await
        .expect("the suite runs");

    assert_eq!(report.steps[0].path, "1-login.http#0");
    assert_eq!(report.steps[1].path, "2-orders.http#0");
}

#[tokio::test]
async fn a_folder_filter_runs_only_that_subtree() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let slug = workspace(dir.path());
    collection::put_request(
        dir.path(),
        &slug,
        "root.http",
        &request("Root", "GET", &api.url("/get")),
    )
    .expect("save request");
    collection::put_request(
        dir.path(),
        &slug,
        "auth/guarded.http",
        &request("Guarded", "GET", &api.url("/json")),
    )
    .expect("save request");

    let report = runner()
        .run_suite(dir.path(), &slug, Some("auth"), None)
        .await
        .expect("the suite runs");

    assert_eq!(report.total, 1);
    assert_eq!(report.steps[0].request_name, "Guarded");
    assert!(api.requests_to("/get").is_empty());
}

#[tokio::test]
async fn an_unresolved_variable_fails_loud_before_anything_reaches_the_wire() {
    let api = MockApi::start().await;
    let error = runner()
        .run_request(
            &request("Ghost", "GET", "{{baseUrl}}/get"),
            &mut VarFrame::default(),
        )
        .await
        .expect_err("baseUrl is not set");

    assert_eq!(error.code(), "E_UNRESOLVED_VAR");
    assert!(api.received_requests().is_empty());
}

#[tokio::test]
async fn the_strict_policy_blocks_the_loopback_mock_and_allow_all_permits_it() {
    let api = MockApi::start().await;
    let req = request("Loopback", "GET", &api.url("/get"));

    let denied = Runner::new(NoSecrets, StrictPolicy::new())
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("loopback is denied under the strict policy");
    assert_eq!(denied.code(), "E_HOST_DENIED");
    assert!(denied.to_string().contains("loopback"));
    assert!(api.received_requests().is_empty());

    let allowed = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("allow-all permits the mock");
    assert!(allowed.passed);
    assert_eq!(api.received_requests().len(), 1);
}
