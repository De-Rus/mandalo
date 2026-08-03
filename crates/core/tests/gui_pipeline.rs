//! The desktop app used to compose its own send pipeline. It now calls the same
//! runner the CLI does, through `Runner::run_one`, with the request it has on
//! screen — saved or not. These tests pin what that entry point returns.

use mandalo_core::assertions::{Capture, CaptureScope};
use mandalo_core::capability::MemorySecrets;
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::request::Auth;
use mandalo_core::runner::{Runner, StepResult};
use mandalo_core::workspace::{self, EnvDoc, VarDef};
use mandalo_core::{AllowAll, SecretWriter};
use mandalo_testkit::fixtures;
use mandalo_testkit::MockApi;
use std::path::Path;

const SECRET: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

fn env_with_secret(dir: &Path, hosts: &[&str], store: &MemorySecrets) {
    let mut doc = EnvDoc::new("prod");
    doc.vars.insert("token".to_string(), VarDef::secret(hosts));
    doc.vars
        .insert("greeting".to_string(), VarDef::plain("hola".to_string()));
    workspace::save_env_doc(dir, &doc).unwrap();
    store.set("prod", "token", SECRET).unwrap();
}

fn authed(url: &str) -> SavedRequest {
    let mut request = fixtures::request("Whoami", "GET", url);
    request.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    request
}

#[tokio::test]
async fn a_declared_secret_resolves_and_reaches_the_wire_from_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    let store = MemorySecrets::new();
    env_with_secret(dir.path(), &["127.0.0.1"], &store);

    let runner = Runner::new(store, AllowAll);
    let step = runner
        .run_one(
            dir.path(),
            &authed(&mock.url("/headers/echo")),
            Some("prod"),
        )
        .await
        .unwrap();

    assert!(step.error.is_none(), "{:?}", step.error);
    assert_eq!(
        mock.last_request().unwrap().header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert!(step.unbound_secrets.is_empty());
}

#[tokio::test]
async fn an_unbound_secret_is_reported_once_and_never_again_after_binding() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    let store = MemorySecrets::new();
    env_with_secret(dir.path(), &[], &store);

    let runner = Runner::new(store, AllowAll);
    let url = mock.url("/headers/echo");
    let step = runner
        .run_one(dir.path(), &authed(&url), Some("prod"))
        .await
        .unwrap();

    assert_eq!(step.unbound_secrets.len(), 1);
    assert_eq!(step.unbound_secrets[0].name, "token");
    assert_eq!(step.unbound_secrets[0].host, "127.0.0.1");

    workspace::bind_secret_host(dir.path(), "prod", "token", "127.0.0.1").unwrap();
    let again = runner
        .run_one(dir.path(), &authed(&url), Some("prod"))
        .await
        .unwrap();
    assert!(again.unbound_secrets.is_empty());
}

#[tokio::test]
async fn script_writes_from_both_phases_are_merged_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    fixtures::environment(dir.path(), "prod", &[("stale", "yes")]);

    let mut request = fixtures::request("Merge", "GET", &mock.url("/json"));
    request.scripts.pre = Some(
        r#"
        pm.environment.set("from_pre", "1");
        pm.environment.set("overwritten", "pre");
        pm.environment.unset("gone_then_back");
        "#
        .to_string(),
    );
    request.scripts.post = Some(
        r#"
        pm.environment.set("overwritten", "post");
        pm.environment.set("gone_then_back", "back");
        pm.environment.unset("stale");
        "#
        .to_string(),
    );

    let step = fixtures::runner()
        .run_one(dir.path(), &request, Some("prod"))
        .await
        .unwrap();

    assert_eq!(step.var_sets.get("from_pre").map(String::as_str), Some("1"));
    assert_eq!(
        step.var_sets.get("overwritten").map(String::as_str),
        Some("post")
    );
    assert_eq!(
        step.var_sets.get("gone_then_back").map(String::as_str),
        Some("back")
    );
    assert_eq!(step.var_unsets, vec!["stale".to_string()]);
    assert!(step.secret_var_sets.is_empty());
}

#[tokio::test]
async fn a_later_unset_erases_an_earlier_set() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    fixtures::environment(dir.path(), "prod", &[]);

    let mut request = fixtures::request("Undo", "GET", &mock.url("/json"));
    request.scripts.pre = Some(r#"pm.environment.set("temp", "value");"#.to_string());
    request.scripts.post = Some(r#"pm.environment.unset("temp");"#.to_string());

    let step = fixtures::runner()
        .run_one(dir.path(), &request, Some("prod"))
        .await
        .unwrap();

    assert!(step.var_sets.is_empty());
    assert_eq!(step.var_unsets, vec!["temp".to_string()]);
}

#[tokio::test]
async fn a_write_to_a_declared_secret_is_reported_by_name_only() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    let store = MemorySecrets::new();
    env_with_secret(dir.path(), &["127.0.0.1"], &store);

    let mut request = fixtures::request("Rotate", "GET", &mock.url("/json"));
    request.scripts.post = Some(
        r#"
        pm.environment.set("token", "rotated-secret-value");
        pm.environment.set("plain", "safe");
        "#
        .to_string(),
    );

    let step = Runner::new(store, AllowAll)
        .run_one(dir.path(), &request, Some("prod"))
        .await
        .unwrap();

    assert_eq!(step.secret_var_sets, vec!["token".to_string()]);
    assert!(!step.var_sets.contains_key("token"));
    assert_eq!(step.var_sets.get("plain").map(String::as_str), Some("safe"));
    let json = serde_json::to_string(&step).unwrap();
    assert!(
        !json.contains("rotated-secret-value"),
        "a secret value must not cross the IPC boundary: {json}"
    );
}

#[tokio::test]
async fn login_capture_and_the_authenticated_call_still_chain() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    fixtures::workspace(dir.path());
    fixtures::environment(dir.path(), "prod", &[]);

    let mut login = fixtures::request("Login", "POST", &mock.url("/auth/login"));
    login.headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    login.body =
        mandalo_core::body::Body::json(r#"{"username":"ada","password":"lovelace"}"#.to_string());
    login.captures = vec![Capture {
        from: "body.$.token".to_string(),
        into: "token".to_string(),
        scope: CaptureScope::Persist,
    }];
    login.scripts.post =
        Some(r#"pm.environment.set("user_id", pm.response.json().user.id);"#.to_string());

    let runner = fixtures::runner();
    let mut vars = mandalo_core::runner::env_frame(dir.path(), Some("prod")).unwrap();
    let first = runner.run_request(&login, &mut vars).await.unwrap();

    assert!(first.error.is_none(), "{:?}", first.error);
    assert!(first.captured.contains_key("token"));
    assert_eq!(
        first.var_sets.get("user_id").map(String::as_str),
        Some("u-1")
    );

    let second = runner
        .run_request(&authed(&mock.url("/auth/bearer")), &mut vars)
        .await
        .unwrap();
    assert_eq!(second.response.as_ref().unwrap().status, 200);
}

fn comparable(step: &StepResult) -> (Vec<String>, Vec<String>, Vec<(String, String)>) {
    (
        step.tests
            .iter()
            .map(|t| format!("{}|{}|{:?}", t.name, t.passed, t.detail))
            .collect(),
        step.script_tests
            .iter()
            .map(|t| format!("{}|{}|{:?}", t.name, t.passed, t.error))
            .collect(),
        step.captured
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    )
}

/// The guard against the two pipelines drifting apart again: what the desktop app
/// runs and what `mandalo run` runs must be the same run.
#[tokio::test]
async fn the_gui_path_and_the_cli_path_agree_on_the_same_request() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let slug = fixtures::workspace(dir.path());
    let store = MemorySecrets::new();
    env_with_secret(dir.path(), &["127.0.0.1"], &store);

    let mut request = fixtures::request("Parity", "GET", &mock.url("/headers/echo"));
    request.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    request.scripts.pre = Some(r#"pm.environment.set("run_id", "abc");"#.to_string());
    request.scripts.post = Some(
        r#"
        pm.test("status is 200", function () { pm.response.to.have.status(200); });
        pm.test("status is 500", function () { pm.response.to.have.status(500); });
        pm.test("greeting travelled", function () { pm.expect(pm.environment.get("greeting")).to.equal("hola"); });
        console.log("done");
        pm.environment.set("header_count", String(pm.response.json().count));
        pm.environment.set("last_status", String(pm.response.code));
        "#
        .to_string(),
    );
    collection::save_request(dir.path(), &slug, None, None, &request).unwrap();

    let gui = Runner::new(MemorySecrets::with(&[("prod", "token", SECRET)]), AllowAll)
        .run_one(dir.path(), &request, Some("prod"))
        .await
        .unwrap();
    let mut report = Runner::new(MemorySecrets::with(&[("prod", "token", SECRET)]), AllowAll)
        .run_suite(dir.path(), &slug, None, Some("prod"))
        .await
        .unwrap();
    assert_eq!(report.steps.len(), 1);
    let cli = report.steps.remove(0);

    assert_eq!(comparable(&gui), comparable(&cli));
    assert_eq!(gui.var_sets, cli.var_sets);
    assert_eq!(gui.logs, cli.logs);
    assert_eq!(gui.passed, cli.passed);
    assert!(!gui.passed, "the deliberately failing assertion must fail");
}
