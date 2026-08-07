//! `export` then `import` is the backup and handoff path, so it is tested the way
//! it is used: workspace A → bundle → empty workspace B, then A and B compared.

use mandalo_core::{bundle, collection, workspace};
use std::path::Path;

const AUTH_HTTP: &str = "\
### Login
POST {{baseUrl}}/auth/login
Content-Type: application/json

{\"username\": \"ada\", \"password\": \"lovelace\"}

> {%
pm.environment.set(\"authToken\", pm.response.json().token);
%}

### Me
GET {{baseUrl}}/auth/bearer
Authorization: Bearer {{authToken}}

### Profile
# @auth inherited
GET {{baseUrl}}/auth/bearer
Authorization: Bearer {{authToken}}
";

const USERS_HTTP: &str = "\
### List
GET {{baseUrl}}/users

### Create
POST {{baseUrl}}/users

{\"name\": \"nova\"}
";

const LOCAL_ENV: &str = "\
schema_version = 1
name = \"local\"

[vars]
baseUrl = \"https://api.example.com\"
token = { secret = true, hosts = [\"api.example.com\"] }
";

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn empty_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(collection::collections_dir(dir.path())).unwrap();
    std::fs::create_dir_all(dir.path().join("environments")).unwrap();
    dir
}

/// A workspace with what a real one has: several requests in one file, a folder,
/// a second collection, and an environment declaring a secret.
fn seed(dir: &Path) {
    let acme = collection::create_collection(dir, "Acme API").unwrap();
    let root = collection::collections_dir(dir).join(&acme.slug);
    write(&root.join("auth.http"), AUTH_HTTP);
    write(&root.join("users/users.http"), USERS_HTTP);

    let beta = collection::create_collection(dir, "Beta").unwrap();
    write(
        &collection::collections_dir(dir)
            .join(&beta.slug)
            .join("ping.http"),
        "### Ping\nGET https://x.dev/ping\n",
    );

    write(&dir.join("environments/local.toml"), LOCAL_ENV);
}

#[test]
fn a_workspace_survives_export_and_import_into_an_empty_one() {
    let a = empty_workspace();
    let b = empty_workspace();
    seed(a.path());

    let json = bundle::export(a.path()).unwrap().json;
    let report = bundle::import(b.path(), &json).unwrap();

    assert_eq!(
        report.imported, 6,
        "every request, not just the first of a file"
    );
    assert_eq!(report.collections, 2);
    assert_eq!(report.environments, 1);

    let tree_a = collection::list_tree(a.path()).unwrap();
    assert_eq!(collection::list_tree(b.path()).unwrap(), tree_a);

    for node in &tree_a.collections {
        let mut paths: Vec<String> = node.requests.iter().map(|r| r.path.clone()).collect();
        for folder in &node.folders {
            paths.extend(folder.requests.iter().map(|r| r.path.clone()));
        }
        assert!(!paths.is_empty(), "{} has no requests", node.slug);
        for path in paths {
            assert_eq!(
                collection::load_request_source(b.path(), &node.slug, &path).unwrap(),
                collection::load_request_source(a.path(), &node.slug, &path).unwrap(),
                "{}#{path}",
                node.slug
            );
        }
    }

    assert_eq!(
        workspace::list_env_docs(b.path()).unwrap(),
        workspace::list_env_docs(a.path()).unwrap(),
        "an environment travels with its secret declarations"
    );
    let raw = std::fs::read_to_string(b.path().join("environments/local.toml")).unwrap();
    assert!(raw.contains("secret = true"), "{raw}");
    assert!(!raw.contains("s3cr3t"), "a value never travels: {raw}");
}

#[test]
fn every_request_of_a_multi_request_file_keeps_its_index() {
    let a = empty_workspace();
    let b = empty_workspace();
    seed(a.path());
    bundle::import(b.path(), &bundle::export(a.path()).unwrap().json).unwrap();

    for (path, name) in [
        ("auth.http#0", "Login"),
        ("auth.http#1", "Me"),
        ("auth.http#2", "Profile"),
    ] {
        assert_eq!(
            collection::load_request(b.path(), "acme-api", path)
                .unwrap()
                .name,
            name
        );
    }
    assert_eq!(
        collection::load_request_source(b.path(), "acme-api", "auth.http#2")
            .unwrap()
            .auth,
        mandalo_core::request::Auth::inherited(mandalo_core::request::Auth::Bearer {
            token: "{{authToken}}".to_string()
        }),
        "an inherited default is still inherited on the other side"
    );
}

#[test]
fn a_bundle_that_fails_halfway_writes_nothing_at_all() {
    let a = empty_workspace();
    let b = empty_workspace();
    seed(a.path());

    let mut json: serde_json::Value =
        serde_json::from_str(&bundle::export(a.path()).unwrap().json).unwrap();
    let last = json["collections"][1]["requests"][0]["path"].take();
    assert_eq!(last, serde_json::json!("ping.http#0"));
    json["collections"][1]["requests"][0]["path"] = serde_json::json!("../../escape.http");

    let err = bundle::import(b.path(), &json.to_string()).unwrap_err();
    assert_eq!(err.code(), "E_INVALID_NAME", "{err}");

    assert!(
        collection::list_collections(b.path())
            .unwrap()
            .items
            .is_empty(),
        "the collections of the healthy half must not survive a failed import"
    );
    assert!(workspace::list_env_docs(b.path()).unwrap().items.is_empty());
    assert!(!b.path().parent().unwrap().join("escape.http").exists());
}

#[test]
fn a_file_missing_from_a_bundle_is_named_instead_of_written_short() {
    let a = empty_workspace();
    let b = empty_workspace();
    seed(a.path());

    let mut json: serde_json::Value =
        serde_json::from_str(&bundle::export(a.path()).unwrap().json).unwrap();
    let requests = json["collections"][0]["requests"].as_array_mut().unwrap();
    requests.retain(|r| r["path"] != serde_json::json!("auth.http#1"));

    let err = bundle::import(b.path(), &json.to_string())
        .unwrap_err()
        .to_string();
    assert!(err.contains("auth.http"), "{err}");
    assert!(err.contains("every request of a file"), "{err}");
    assert!(collection::list_collections(b.path())
        .unwrap()
        .items
        .is_empty());
}
