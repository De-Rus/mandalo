use mandalo_core::bundle::{self, CollectionSelection, ExportSelection};
use mandalo_core::workspace::{self, EnvDoc, VarDef};
use mandalo_core::{collection, Body, SavedRequest};
use std::path::Path;

fn request(name: &str) -> SavedRequest {
    SavedRequest {
        id: uuid_like(name),
        name: name.to_string(),
        kind: "http".to_string(),
        method: "GET".to_string(),
        url: "{{base}}/things".to_string(),
        description: None,
        headers: Vec::new(),
        auth: mandalo_core::request::Auth::None,
        body: Body::None,
        grpc: None,
        stream: None,
        scripts: Default::default(),
        tests: Vec::new(),
        captures: Vec::new(),
    }
}

fn uuid_like(seed: &str) -> String {
    let n: u32 = seed.bytes().map(|b| b as u32).sum();
    format!("{n:08x}-1111-4111-8111-111111111111")
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(collection::collections_dir(ws)).unwrap();

    let acme = collection::create_collection(ws, "Acme API").unwrap();
    collection::create_folder(ws, &acme.slug, "users").unwrap();
    collection::create_folder(ws, &acme.slug, "orders").unwrap();
    collection::save_request(ws, &acme.slug, None, None, &request("Root")).unwrap();
    collection::save_request(ws, &acme.slug, None, Some("users"), &request("Alpha")).unwrap();
    collection::save_request(ws, &acme.slug, None, Some("users"), &request("Beta")).unwrap();
    collection::save_request(ws, &acme.slug, None, Some("orders"), &request("Gamma")).unwrap();

    let beta = collection::create_collection(ws, "Beta").unwrap();
    collection::save_request(ws, &beta.slug, None, None, &request("Ping")).unwrap();

    for (name, secret) in [("staging", true), ("prod", true), ("plain", false)] {
        let mut doc = EnvDoc::new(name);
        doc.vars.insert(
            "base".to_string(),
            VarDef::shared(format!("https://{name}.example.test")),
        );
        if secret {
            doc.vars
                .insert("token".to_string(), VarDef::secret(&["example.test"]));
        }
        workspace::save_env_doc(ws, &doc).unwrap();
    }
    Fixture { dir }
}

fn plan(ws: &Path, selection: &ExportSelection) -> bundle::ExportPlan {
    bundle::plan_export(ws, selection).expect("plan")
}

#[test]
fn the_default_selection_is_the_whole_workspace() {
    let f = fixture();
    let plan = plan(f.path(), &ExportSelection::default());
    assert_eq!(plan.included.request_count, 5);
    assert_eq!(plan.included.collections.len(), 2);
    assert_eq!(plan.included.environments, vec!["plain", "prod", "staging"]);
    assert_eq!(plan.excluded.requests, 0);
    assert!(plan.excluded.collections.is_empty());
    assert!(plan.bytes > 0);
}

#[test]
fn narrowing_to_one_collection_leaves_the_others_out() {
    let f = fixture();
    let plan = plan(
        f.path(),
        &ExportSelection {
            collections: Some(vec![CollectionSelection::whole("beta")]),
            environments: None,
        },
    );
    assert_eq!(plan.included.collections.len(), 1);
    assert_eq!(plan.included.collections[0].slug, "beta");
    assert_eq!(plan.included.request_count, 1);
    assert_eq!(plan.excluded.collections, vec!["acme-api"]);
    assert_eq!(plan.excluded.requests, 4);
}

#[test]
fn narrowing_to_a_folder_keeps_only_what_is_under_it() {
    let f = fixture();
    let plan = plan(
        f.path(),
        &ExportSelection {
            collections: Some(vec![CollectionSelection {
                slug: "acme-api".to_string(),
                folders: vec!["users".to_string()],
                requests: Vec::new(),
            }]),
            environments: None,
        },
    );
    assert_eq!(plan.included.request_count, 2);
    let paths: Vec<&str> = plan.included.collections[0]
        .requests
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    assert!(paths.iter().all(|p| p.starts_with("users/")), "{paths:?}");
    assert_eq!(plan.excluded.requests, 3);
}

#[test]
fn narrowing_to_one_request_keeps_exactly_that_one() {
    let f = fixture();
    let whole = plan(f.path(), &ExportSelection::default());
    let wanted = whole
        .included
        .collections
        .iter()
        .find(|c| c.slug == "acme-api")
        .unwrap()
        .requests
        .iter()
        .find(|r| r.name == "Beta")
        .expect("seeded request")
        .path
        .clone();

    let plan = plan(
        f.path(),
        &ExportSelection {
            collections: Some(vec![CollectionSelection {
                slug: "acme-api".to_string(),
                folders: Vec::new(),
                requests: vec![wanted.clone()],
            }]),
            environments: Some(Vec::new()),
        },
    );
    assert_eq!(plan.included.request_count, 1);
    assert_eq!(plan.included.collections[0].requests[0].path, wanted);
    assert!(plan.included.environments.is_empty());
    assert_eq!(plan.excluded.environments.len(), 3);
}

#[test]
fn an_environment_subset_carries_only_the_named_ones() {
    let f = fixture();
    let plan = plan(
        f.path(),
        &ExportSelection {
            collections: None,
            environments: Some(vec!["prod".to_string()]),
        },
    );
    assert_eq!(plan.included.environments, vec!["prod"]);
    assert_eq!(plan.excluded.environments, vec!["plain", "staging"]);
    let json: serde_json::Value = serde_json::from_str(plan.json()).unwrap();
    let names: Vec<&str> = json["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["prod"]);
}

#[test]
fn the_plan_counts_the_secret_values_it_is_not_carrying() {
    let f = fixture();
    let plan = plan(f.path(), &ExportSelection::default());
    assert_eq!(plan.excluded.secret_values, 2);
    assert_eq!(
        plan.excluded.withheld_names,
        vec!["prod.token", "staging.token"]
    );
    assert!(
        !plan.json().contains("\"value\":\"\""),
        "a secret declaration must not carry an empty value either"
    );

    let one = plan_for_env(f.path(), "prod");
    assert_eq!(one.excluded.secret_values, 1);
    assert_eq!(one.excluded.withheld_names, vec!["prod.token"]);
}

fn plan_for_env(ws: &Path, name: &str) -> bundle::ExportPlan {
    plan(
        ws,
        &ExportSelection {
            collections: None,
            environments: Some(vec![name.to_string()]),
        },
    )
}

#[test]
fn a_selection_that_names_something_absent_fails_loud() {
    let f = fixture();
    for selection in [
        ExportSelection {
            collections: Some(vec![CollectionSelection::whole("ghost")]),
            environments: None,
        },
        ExportSelection {
            collections: Some(vec![CollectionSelection {
                slug: "acme-api".to_string(),
                folders: vec!["ghosts".to_string()],
                requests: Vec::new(),
            }]),
            environments: None,
        },
        ExportSelection {
            collections: None,
            environments: Some(vec!["ghost".to_string()]),
        },
    ] {
        let err = bundle::plan_export(f.path(), &selection).unwrap_err();
        assert_eq!(err.code(), "E_NOT_FOUND", "{err}");
    }
}

#[test]
fn a_finding_blocks_the_export_and_nothing_reaches_the_disk() {
    let f = fixture();
    let mut leaky = request("Leaky");
    leaky.headers = vec![(
        "Authorization".to_string(),
        format!("Bearer {}", "ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
    )];
    collection::save_request(f.path(), "beta", None, None, &leaky).unwrap();

    let selection = ExportSelection::default();
    let plan = plan(f.path(), &selection);
    assert!(plan.blocked);
    assert!(plan.findings.iter().any(|f| f.rule == "github-token"));
    assert!(
        plan.findings
            .iter()
            .all(|f| !f.excerpt.contains("347Ae178B4a")),
        "an excerpt must not reproduce the credential"
    );

    let dest = f.path().join("bundle.json");
    let err = bundle::run_export(f.path(), &selection, &plan.token, &dest, false).unwrap_err();
    assert_eq!(err.code(), "E_SECRET");
    assert!(
        !dest.exists(),
        "a blocked export must not leave a file behind"
    );

    let receipt =
        bundle::run_export(f.path(), &selection, &plan.token, &dest, true).expect("force");
    assert!(dest.exists());
    assert!(receipt.forced);
}

#[test]
fn a_credential_only_in_an_unselected_collection_does_not_block() {
    let f = fixture();
    let mut leaky = request("Leaky");
    leaky.url = format!("https://x.test/?k={}", "AKIAIOSFODNN7EXAMPLE");
    collection::save_request(f.path(), "beta", None, None, &leaky).unwrap();

    let selection = ExportSelection {
        collections: Some(vec![CollectionSelection::whole("acme-api")]),
        environments: None,
    };
    let plan = plan(f.path(), &selection);
    assert!(!plan.blocked, "{:?}", plan.findings);

    let dest = f.path().join("acme.json");
    bundle::run_export(f.path(), &selection, &plan.token, &dest, false).expect("write");
    assert!(!std::fs::read_to_string(&dest).unwrap().contains("AKIA"));
}

#[test]
fn an_export_cannot_run_without_the_token_of_its_own_plan() {
    let f = fixture();
    let selection = ExportSelection::default();
    let plan = plan(f.path(), &selection);
    let dest = f.path().join("bundle.json");

    let err = bundle::run_export(f.path(), &selection, "not-a-token", &dest, false).unwrap_err();
    assert_eq!(err.code(), "E_CONFLICT");
    assert!(!dest.exists());

    collection::save_request(f.path(), "beta", None, None, &request("Added later")).unwrap();
    let err = bundle::run_export(f.path(), &selection, &plan.token, &dest, false).unwrap_err();
    assert_eq!(err.code(), "E_CONFLICT");
    assert!(err.to_string().contains("reviewed"), "{err}");
    assert!(!dest.exists());

    let fresh = plan_again(f.path(), &selection);
    bundle::run_export(f.path(), &selection, &fresh.token, &dest, false).expect("write");
    assert!(dest.exists());
}

fn plan_again(ws: &Path, selection: &ExportSelection) -> bundle::ExportPlan {
    bundle::plan_export(ws, selection).expect("plan")
}

#[test]
fn a_narrowed_export_reimports_as_exactly_what_was_chosen() {
    let f = fixture();
    let selection = ExportSelection {
        collections: Some(vec![CollectionSelection {
            slug: "acme-api".to_string(),
            folders: vec!["users".to_string()],
            requests: Vec::new(),
        }]),
        environments: Some(vec!["prod".to_string()]),
    };
    let plan = plan(f.path(), &selection);
    let dest = f.path().join("users.json");
    bundle::run_export(f.path(), &selection, &plan.token, &dest, false).expect("write");

    let target = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(collection::collections_dir(target.path())).unwrap();
    let json = std::fs::read_to_string(&dest).unwrap();
    let report = bundle::import(target.path(), &json).expect("import");
    assert_eq!(report.imported, 2);
    assert_eq!(report.collections, 1);
    assert_eq!(report.environments, 1);
    let tree = collection::list_tree(target.path()).unwrap();
    assert_eq!(tree.collections.len(), 1);
    assert_eq!(tree.collections[0].slug, "acme-api");
}
