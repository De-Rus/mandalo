//! A Postman collection is imported and then *run*. Parsing it is not the
//! promise; the promise is that the workspace works.

use mandalo_core::runner::Runner;
use mandalo_core::{collection, postman, AllowAll, NoSecrets};
use mandalo_testkit::fixtures::{environment, workspace};
use mandalo_testkit::{MockApi, BASIC_PASSWORD, BASIC_USER, TOKEN};

/// The shape that broke: collection-level bearer auth on `{{authToken}}`, a login
/// request that produces it, a request that spends it *and* writes the header
/// itself, and one that opts out of auth entirely.
fn collection_json() -> String {
    serde_json::json!({
        "info": {
            "name": "Acme API",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "auth": {
            "type": "bearer",
            "bearer": [{"key": "token", "value": "{{authToken}}", "type": "string"}]
        },
        "item": [
            {
                "name": "Health",
                "request": {
                    "method": "GET",
                    "url": "{{baseUrl}}/health",
                    "auth": {"type": "noauth"}
                }
            },
            {
                "name": "Auth",
                "item": [
                    {
                        "name": "Login",
                        "request": {
                            "method": "POST",
                            "url": "{{baseUrl}}/auth/login",
                            "header": [{"key": "Content-Type", "value": "application/json"}],
                            "body": {
                                "mode": "raw",
                                "raw": format!("{{\"username\": \"{BASIC_USER}\", \"password\": \"{BASIC_PASSWORD}\"}}"),
                                "options": {"raw": {"language": "json"}}
                            }
                        },
                        "event": [{
                            "listen": "test",
                            "script": {"exec": [
                                "pm.test(\"login works\", function () { pm.response.to.have.status(200); });",
                                "pm.environment.set(\"authToken\", pm.response.json().token);"
                            ]}
                        }]
                    },
                    {
                        "name": "Me",
                        "request": {
                            "method": "GET",
                            "url": "{{baseUrl}}/auth/bearer",
                            "header": [{"key": "Authorization", "value": "Bearer {{authToken}}"}]
                        }
                    },
                    {
                        "name": "Profile",
                        "request": {"method": "GET", "url": "{{baseUrl}}/auth/bearer"}
                    }
                ]
            }
        ],
        "variable": [{"key": "baseUrl", "value": "https://api.acme.dev"}]
    })
    .to_string()
}

#[tokio::test]
async fn a_freshly_imported_collection_with_collection_level_auth_runs() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().unwrap();
    workspace(dir.path());
    environment(dir.path(), "local", &[("baseUrl", &api.base_url())]);

    let report = postman::import(dir.path(), &collection_json()).unwrap();
    assert_eq!(report.imported, 4);

    let run = Runner::new(NoSecrets, AllowAll)
        .with_workspace(dir.path())
        .run_suite(dir.path(), "acme-api", None, Some("local"))
        .await
        .unwrap();

    let failures: Vec<String> = run
        .steps
        .iter()
        .filter(|s| !s.passed)
        .map(|s| format!("{}: {}", s.request_name, s.failures().join("; ")))
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(run.total, 4);
    assert!(run.passed);

    // The request that *makes* the token cannot be the one that demands it.
    let login = api.requests_to("/auth/login");
    assert_eq!(login.len(), 1);
    assert_eq!(login[0].header("authorization"), None);

    // Both spenders send it, and each sends it once.
    let bearer = api.requests_to("/auth/bearer");
    assert_eq!(bearer.len(), 2);
    for received in &bearer {
        assert_eq!(
            received.header("authorization"),
            Some(&format!("Bearer {TOKEN}")[..])
        );
    }
}

#[tokio::test]
async fn the_collection_order_is_the_run_order() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().unwrap();
    workspace(dir.path());
    environment(dir.path(), "local", &[("baseUrl", &api.base_url())]);
    postman::import(dir.path(), &collection_json()).unwrap();

    let paths = mandalo_core::runner::suite_paths(dir.path(), "acme-api", None).unwrap();
    assert_eq!(
        paths,
        vec![
            "health.http#0",
            "auth/1-login.http#0",
            "auth/2-me.http#0",
            "auth/3-profile.http#0"
        ],
        "login has to run before the requests that spend its token"
    );
}

#[test]
fn an_inherited_header_is_written_once_and_says_it_was_inherited() {
    let dir = tempfile::tempdir().unwrap();
    workspace(dir.path());
    postman::import(dir.path(), &collection_json()).unwrap();

    let root = collection::collections_dir(dir.path()).join("acme-api");
    let me = std::fs::read_to_string(root.join("auth/2-me.http")).unwrap();
    assert_eq!(
        me.lines()
            .filter(|l| l.to_ascii_lowercase().starts_with("authorization:"))
            .count(),
        1,
        "the request's own header must not be doubled by the collection default:\n{me}"
    );
    assert!(
        !me.contains("@auth inherited"),
        "a header the request wrote itself is not inherited:\n{me}"
    );

    let profile = std::fs::read_to_string(root.join("auth/3-profile.http")).unwrap();
    assert!(profile.contains("# @auth inherited"), "{profile}");
    assert!(
        profile.contains("Authorization: Bearer {{authToken}}"),
        "{profile}"
    );

    let health = std::fs::read_to_string(root.join("health.http")).unwrap();
    assert!(
        !health.to_ascii_lowercase().contains("authorization"),
        "a request that declares noauth stays unauthenticated:\n{health}"
    );
}

#[tokio::test]
async fn a_request_that_wrote_its_own_header_still_fails_loud_on_an_unset_variable() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().unwrap();
    let slug = workspace(dir.path());
    environment(dir.path(), "local", &[("baseUrl", &api.base_url())]);
    std::fs::write(
        collection::collections_dir(dir.path())
            .join(&slug)
            .join("me.http"),
        "### Me\nGET {{baseUrl}}/auth/bearer\nAuthorization: Bearer {{authToken}}\n",
    )
    .unwrap();

    let run = Runner::new(NoSecrets, AllowAll)
        .with_workspace(dir.path())
        .run_suite(dir.path(), &slug, None, Some("local"))
        .await
        .unwrap();
    assert!(!run.passed);
    assert_eq!(run.steps[0].error_code.as_deref(), Some("E_UNRESOLVED_VAR"));
}

/// A text-only form-data body loses nothing on the way in, so it must arrive at
/// the server as the parts the export described.
#[tokio::test]
async fn an_imported_form_data_body_sends_its_parts() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().unwrap();
    workspace(dir.path());
    environment(dir.path(), "local", &[("baseUrl", &api.base_url())]);

    let json = serde_json::json!({
        "info": {
            "name": "Uploads",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": [{
            "name": "Report",
            "request": {
                "method": "POST",
                "url": "{{baseUrl}}/body/multipart",
                "header": [{"key": "Content-Type", "value": "multipart/form-data; boundary=--Exported"}],
                "body": {"mode": "formdata", "formdata": [
                    {"key": "title", "value": "Q3 expenses", "type": "text"},
                    {"key": "quarter", "value": "3", "type": "text"}
                ]}
            }
        }]
    })
    .to_string();
    let report = postman::import(dir.path(), &json).unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(report.warnings, Vec::<String>::new());
    assert_eq!(report.skipped, Vec::<String>::new());

    let run = Runner::new(NoSecrets, AllowAll)
        .with_workspace(dir.path())
        .run_suite(dir.path(), "uploads", None, Some("local"))
        .await
        .unwrap();
    assert!(run.passed, "{:?}", run.steps[0].failures());

    let echo: serde_json::Value =
        serde_json::from_str(&run.steps[0].response.as_ref().unwrap().body).unwrap();
    assert_eq!(echo["count"], 2);
    assert!(echo["contentType"]
        .as_str()
        .unwrap()
        .starts_with("multipart/form-data; boundary="));
    assert_eq!(echo["parts"][0]["name"], "title");
    assert_eq!(echo["parts"][0]["text"], "Q3 expenses");
    assert_eq!(echo["parts"][0]["filename"], serde_json::Value::Null);
    assert_eq!(echo["parts"][1]["name"], "quarter");
    assert_eq!(echo["parts"][1]["text"], "3");
}
