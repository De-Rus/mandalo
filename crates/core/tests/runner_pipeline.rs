use mandalo_core::assertions::{Scripts, StatusOp, TestAssertion};
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::request::Auth;
use mandalo_core::runner::{Runner, VarFrame};
use mandalo_core::workspace::{self, Environment, Manifest, SCHEMA_VERSION};
use mandalo_core::{AllowAll, EnvVarStore, NoSecrets, StrictPolicy};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::mpsc::Receiver;

fn serve(bodies: Vec<&'static str>) -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for body in bodies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 8192];
            let mut received = Vec::new();
            loop {
                let Ok(n) = stream.read(&mut buf) else { return };
                received.extend_from_slice(&buf[..n]);
                if n == 0 || received.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = tx.send(String::from_utf8_lossy(&received).to_string());
        }
    });
    (format!("http://{addr}"), rx)
}

fn request(name: &str, url: &str) -> SavedRequest {
    SavedRequest {
        id: name.to_ascii_lowercase().replace(' ', "-"),
        name: name.to_string(),
        kind: "http".to_string(),
        method: "GET".to_string(),
        url: url.to_string(),
        description: None,
        headers: Vec::new(),
        auth: Auth::None,
        body: mandalo_core::Body::None,
        grpc: None,
        scripts: Scripts::default(),
        tests: Vec::new(),
        captures: Vec::new(),
    }
}

fn plain_runner() -> Runner<NoSecrets, AllowAll> {
    Runner::new(NoSecrets, AllowAll)
}

fn workspace_at(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("environments")).unwrap();
    std::fs::create_dir_all(dir.join("collections")).unwrap();
    workspace::write_manifest(
        dir,
        &Manifest {
            schema_version: SCHEMA_VERSION,
            id: "test-workspace".to_string(),
            name: "Test".to_string(),
        },
    )
    .unwrap();
    collection::ensure_collection(dir, "api", "API", None)
        .unwrap()
        .slug
}

#[tokio::test]
async fn pre_script_patch_reaches_the_wire() {
    let (url, rx) = serve(vec![r#"{"ok":true}"#]);
    let mut req = request("Patched", &url);
    req.scripts.pre = Some(
        r#"
        pm.request.headers.upsert({ key: "X-Patched", value: "yes" });
        pm.request.method = "POST";
        pm.request.body = '{"from":"pre"}';
        pm.environment.set("greeting", "hola");
        "#
        .to_string(),
    );

    let mut vars = VarFrame::default();
    let step = plain_runner().run_request(&req, &mut vars).await.unwrap();

    let raw = rx.recv().unwrap();
    assert!(raw.starts_with("POST / HTTP/1.1\r\n"), "raw was {raw}");
    assert!(raw.contains("x-patched: yes\r\n"), "raw was {raw}");
    assert!(raw.ends_with("{\"from\":\"pre\"}"), "raw was {raw}");
    assert_eq!(vars.get("greeting"), Some("hola"));
    assert!(step.passed);
}

#[tokio::test]
async fn post_script_tests_and_logs_are_collected() {
    let (url, _rx) = serve(vec![r#"{"token":"t0k","count":2}"#]);
    let mut req = request("Login", &url);
    req.scripts.post = Some(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        pm.test("count is high", function () { pm.expect(pm.response.json().count).to.be.above(10); });
        console.log("checked", pm.info.requestName);
        pm.environment.set("token", pm.response.json().token);
        "#
        .to_string(),
    );

    let mut vars = VarFrame::default();
    let step = plain_runner().run_request(&req, &mut vars).await.unwrap();

    assert_eq!(step.script_tests.len(), 2);
    assert!(step.script_tests[0].passed);
    assert!(!step.script_tests[1].passed);
    assert!(!step.passed);
    assert_eq!(step.logs, vec!["checked Login"]);
    assert_eq!(vars.get("token"), Some("t0k"));
}

#[tokio::test]
async fn a_failing_declarative_assertion_fails_the_step() {
    let (url, _rx) = serve(vec![r#"{"total":0}"#]);
    let mut req = request("Orders", &url);
    req.tests = vec![
        TestAssertion::Status {
            op: StatusOp::Eq,
            value: 200,
        },
        TestAssertion::Status {
            op: StatusOp::Eq,
            value: 404,
        },
    ];

    let step = plain_runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .unwrap();

    assert_eq!(step.tests.len(), 2);
    assert!(step.tests[0].passed);
    assert!(!step.tests[1].passed);
    assert!(!step.passed);
    assert_eq!(step.failures(), vec!["status eq 404: status was 200"]);
}

#[tokio::test]
async fn captures_thread_into_the_next_step_of_a_suite() {
    let (url, rx) = serve(vec![r#"{"token":"abc123"}"#, r#"{"ok":true}"#]);
    let dir = tempfile::tempdir().unwrap();
    let slug = workspace_at(dir.path());

    let mut login = request("Login", &format!("{url}/login"));
    login.scripts.post =
        Some(r#"pm.environment.set("token", pm.response.json().token);"#.to_string());
    let mut orders = request("Orders", "{{base}}/orders");
    orders.headers = vec![("X-Token".to_string(), "{{token}}".to_string())];
    orders.scripts.post = Some(
        r#"pm.test("status is 200", function () { pm.response.to.have.status(200); });"#
            .to_string(),
    );

    collection::put_request(dir.path(), &slug, "1-login.http", &login).unwrap();
    collection::put_request(dir.path(), &slug, "2-orders.http", &orders).unwrap();
    workspace::save_environment(
        dir.path(),
        &Environment {
            name: "local".to_string(),
            vars: [("base".to_string(), url.clone())].into_iter().collect(),
        },
    )
    .unwrap();

    let report = plain_runner()
        .run_suite(dir.path(), &slug, None, Some("local"))
        .await
        .unwrap();

    assert_eq!(report.total, 2);
    assert_eq!(report.failed, 0);
    assert!(report.passed);
    assert_eq!(report.steps[0].var_sets.get("token").unwrap(), "abc123");

    let first = rx.recv().unwrap();
    assert!(first.starts_with("GET /login HTTP/1.1\r\n"));
    let second = rx.recv().unwrap();
    assert!(second.starts_with("GET /orders HTTP/1.1\r\n"));
    assert!(second.contains("x-token: abc123\r\n"), "raw was {second}");
}

#[tokio::test]
async fn a_folder_filter_runs_only_that_subtree() {
    let (url, _rx) = serve(vec![r#"{"ok":true}"#]);
    let dir = tempfile::tempdir().unwrap();
    let slug = workspace_at(dir.path());
    collection::put_request(dir.path(), &slug, "root.http", &request("Root", &url)).unwrap();
    collection::put_request(dir.path(), &slug, "admin/deep.http", &request("Deep", &url)).unwrap();

    let report = plain_runner()
        .run_suite(dir.path(), &slug, Some("admin"), None)
        .await
        .unwrap();

    assert_eq!(report.total, 1);
    assert_eq!(report.steps[0].request_name, "Deep");
    assert_eq!(report.steps[0].path, "admin/deep.http#0");
}

#[tokio::test]
async fn a_transport_failure_becomes_a_failed_step_not_an_aborted_suite() {
    let dir = tempfile::tempdir().unwrap();
    let slug = workspace_at(dir.path());
    collection::put_request(
        dir.path(),
        &slug,
        "broken.http",
        &request("Broken", "http://127.0.0.1:1/nope"),
    )
    .unwrap();

    let report = plain_runner()
        .run_suite(dir.path(), &slug, None, None)
        .await
        .unwrap();

    assert_eq!(report.total, 1);
    assert!(!report.passed);
    assert_eq!(report.steps[0].error_code.as_deref(), Some("E_NETWORK"));
    assert!(report.steps[0].error.is_some());
}

#[tokio::test]
async fn a_secret_resolves_from_the_environment_store() {
    let (url, rx) = serve(vec![r#"{"ok":true}"#]);
    std::env::set_var("MANDALO_SECRET__STAGING__API_TOKEN", "from-env");
    let mut req = request("Secret", &url);
    req.auth = Auth::Bearer {
        token: "{{api-token}}".to_string(),
    };

    let mut vars = VarFrame::new("staging");
    let step = Runner::new(EnvVarStore, AllowAll)
        .run_request(&req, &mut vars)
        .await
        .unwrap();
    std::env::remove_var("MANDALO_SECRET__STAGING__API_TOKEN");

    assert!(step.passed);
    let raw = rx.recv().unwrap();
    assert!(
        raw.contains("authorization: Bearer from-env\r\n"),
        "raw was {raw}"
    );
}

#[tokio::test]
async fn the_strict_policy_refuses_a_loopback_host() {
    let (url, _rx) = serve(vec![r#"{"ok":true}"#]);
    let req = request("Loopback", &url);

    let error = Runner::new(NoSecrets, StrictPolicy::new())
        .run_request(&req, &mut VarFrame::default())
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_HOST_DENIED");
    assert!(error.to_string().contains("loopback"));
}

#[tokio::test]
async fn an_unresolved_variable_fails_loud() {
    let req = request("Ghost", "{{base}}/x");

    let error = plain_runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_UNRESOLVED_VAR");
}
