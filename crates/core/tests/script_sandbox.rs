use mandalo_core::script::{run_script, Limits, ScriptContext, ScriptRequest, ScriptResponse};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn context(response: Option<ScriptResponse>) -> ScriptContext {
    ScriptContext {
        vars: BTreeMap::from([("base".to_string(), "https://x.dev".to_string())]),
        request_name: "Login".to_string(),
        request: ScriptRequest {
            method: "POST".to_string(),
            url: "https://x.dev/login".to_string(),
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: Some(r#"{"user":"nova"}"#.to_string()),
        },
        response,
    }
}

fn ok_response() -> ScriptResponse {
    ScriptResponse {
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: r#"{"token":"t0k","expiresIn":3600}"#.to_string(),
        duration_ms: 17,
    }
}

#[test]
fn post_script_records_tests_and_variables() {
    let outcome = run_script(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        var json = pm.response.json();
        pm.environment.set("token", json.token);
        pm.test("has token", function () { pm.expect(json.token).to.be.a("string"); });
        pm.test("expires soon", function () { pm.expect(json.expiresIn).to.be.below(60); });
        console.log("done", pm.info.requestName);
        "#,
        context(Some(ok_response())),
        Limits::default(),
    )
    .unwrap();

    assert_eq!(outcome.tests.len(), 3);
    assert!(outcome.tests[0].passed);
    assert!(outcome.tests[1].passed);
    assert!(!outcome.tests[2].passed);
    assert_eq!(outcome.var_sets.get("token").unwrap(), "t0k");
    assert_eq!(outcome.logs, vec!["done Login"]);
    assert!(outcome.request_patch.is_none());
}

#[test]
fn pre_script_patches_the_request() {
    let outcome = run_script(
        r#"
        pm.request.headers.upsert({ key: "Authorization", value: "Bearer " + pm.environment.get("base") });
        pm.request.url = pm.environment.get("base") + "/v2/login";
        pm.request.body = JSON.stringify({ user: "nova", retry: true });
        "#,
        context(None),
        Limits::default(),
    )
    .unwrap();

    let patch = outcome.request_patch.unwrap();
    assert_eq!(patch.method, "POST");
    assert_eq!(patch.url, "https://x.dev/v2/login");
    assert_eq!(patch.body.unwrap(), r#"{"user":"nova","retry":true}"#);
    assert_eq!(
        patch.headers,
        vec![
            ("Accept".to_string(), "application/json".to_string()),
            (
                "Authorization".to_string(),
                "Bearer https://x.dev".to_string()
            ),
        ]
    );
}

#[test]
fn the_sandbox_has_no_host_capabilities() {
    let outcome = run_script(
        r#"
        var blocked = ["require","process","fetch","XMLHttpRequest","setTimeout","setInterval","Deno","window","document"];
        for (var i = 0; i < blocked.length; i++) {
          if (typeof globalThis[blocked[i]] !== "undefined") console.log("LEAK:" + blocked[i]);
        }
        try { pm.sendRequest("https://x.dev"); } catch (e) { console.log(e.message); }
        "#,
        context(None),
        Limits::default(),
    )
    .unwrap();

    assert_eq!(outcome.logs.len(), 1);
    assert!(outcome.logs[0].contains("pm.sendRequest is not available"));
}

#[test]
fn an_infinite_loop_is_killed_by_the_timeout() {
    let limits = Limits {
        memory_bytes: 32 * 1024 * 1024,
        timeout_ms: 200,
    };
    let started = Instant::now();
    let err = run_script("while (true) {}", context(None), limits).unwrap_err();
    assert_eq!(err.code(), "E_SCRIPT_TIMEOUT");
    assert_eq!(err.to_string(), "script exceeded 200ms");
    assert!(started.elapsed() < Duration::from_secs(5));
}
