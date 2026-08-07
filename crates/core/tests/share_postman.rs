use mandalo_core::collection;
use mandalo_core::git_sync::{self, Auth, SyncOutcome, SyncSelection};
use mandalo_core::postman;
use mandalo_core::share;
use mandalo_core::workspace::{self, ShareConfig, ShareFormat, SCHEMA_VERSION};
use std::sync::OnceLock;

fn isolate() {
    static ONCE: OnceLock<tempfile::TempDir> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("config sandbox");
        for level in [
            git2::ConfigLevel::System,
            git2::ConfigLevel::Global,
            git2::ConfigLevel::XDG,
            git2::ConfigLevel::ProgramData,
        ] {
            unsafe {
                let _ = git2::opts::set_search_path(level, dir.path());
            }
        }
        dir
    });
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    workspace::write_manifest(
        dir.path(),
        &workspace::Manifest {
            schema_version: SCHEMA_VERSION,
            id: "share-test".into(),
            name: "Share Test".into(),
            remote: None,
            share: None,
        },
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("environments")).unwrap();
    std::fs::create_dir_all(dir.path().join("collections")).unwrap();
    collection::ensure_collection(dir.path(), "api", "API", None).unwrap();
    let req = collection::SavedRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Ping".into(),
        kind: "http".into(),
        method: "GET".into(),
        url: "https://example.test/ping".into(),
        description: None,
        headers: vec![],
        auth: Default::default(),
        body: Default::default(),
        grpc: None,
        stream: None,
        scripts: Default::default(),
        tests: vec![],
        captures: vec![],
    };
    collection::save_request(dir.path(), "api", None, None, &req).unwrap();
    std::fs::write(
        dir.path().join("environments/local.toml"),
        "schema_version = 1\nname = \"local\"\n\n[vars]\nbaseUrl = { value = \"https://example.test\" }\n",
    )
    .unwrap();
    dir
}

#[test]
fn materialize_is_a_noop_without_share_config() {
    let dir = workspace();
    let report = share::materialize(dir.path()).unwrap();
    assert!(report.paths.is_empty());
    assert!(!dir.path().join("postman").exists());
}

#[test]
fn materialize_writes_postman_mirror_when_configured() {
    let dir = workspace();
    workspace::set_share(dir.path(), Some(ShareConfig::postman())).unwrap();
    let report = share::materialize(dir.path()).unwrap();
    assert_eq!(report.dir, "postman");
    assert!(report.paths.iter().any(|p| p == "postman/api.json"));
    assert!(report
        .paths
        .iter()
        .any(|p| p == "postman/environments/local.json"));
    let json = std::fs::read_to_string(dir.path().join("postman/api.json")).unwrap();
    assert!(json.contains("schema.getpostman.com/json/collection/v2.1.0"));
    assert!(json.contains("Ping"));
}

#[test]
fn postman_export_round_trips_a_rest_request() {
    let dir = workspace();
    let tree = collection::list_tree(dir.path()).unwrap();
    let node = &tree.collections[0];
    let mut warnings = postman::ExportWarnings::default();
    let json = postman::collection_json(dir.path(), node, &mut warnings).unwrap();

    let dest = tempfile::tempdir().unwrap();
    workspace::write_manifest(
        dest.path(),
        &workspace::Manifest {
            schema_version: SCHEMA_VERSION,
            id: "roundtrip".into(),
            name: "Roundtrip".into(),
            remote: None,
            share: None,
        },
    )
    .unwrap();
    std::fs::create_dir_all(dest.path().join("collections")).unwrap();
    std::fs::create_dir_all(dest.path().join("environments")).unwrap();
    let report = postman::import(dest.path(), &json).unwrap();
    assert!(report.imported >= 1, "{report:?}");
    let back = collection::list_tree(dest.path()).unwrap();
    assert!(!back.collections.is_empty());
}

#[test]
fn plan_sync_includes_generated_postman_files() {
    isolate();
    let root = tempfile::tempdir().unwrap();
    let ws = root.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();

    // Seed a real Mandalo workspace, then turn it into a git repo.
    workspace::write_manifest(
        &ws,
        &workspace::Manifest {
            schema_version: SCHEMA_VERSION,
            id: "sync-share".into(),
            name: "Sync Share".into(),
            remote: None,
            share: Some(ShareConfig {
                format: ShareFormat::Postman,
                dir: None,
            }),
        },
    )
    .unwrap();
    std::fs::create_dir_all(ws.join("environments")).unwrap();
    std::fs::create_dir_all(ws.join("collections")).unwrap();
    collection::ensure_collection(&ws, "api", "API", None).unwrap();
    let req = collection::SavedRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Ping".into(),
        kind: "http".into(),
        method: "GET".into(),
        url: "https://example.test/ping".into(),
        description: None,
        headers: vec![],
        auth: Default::default(),
        body: Default::default(),
        grpc: None,
        stream: None,
        scripts: Default::default(),
        tests: vec![],
        captures: vec![],
    };
    collection::save_request(&ws, "api", None, None, &req).unwrap();

    git_sync::init(&ws, None).unwrap();
    let repo = git2::Repository::open(&ws).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "tester").unwrap();
    config.set_str("user.email", "tester@example.test").unwrap();

    let plan = git_sync::plan_sync(&ws, &SyncSelection::default(), None).unwrap();
    assert_eq!(plan.share_dir.as_deref(), Some("postman"));
    assert!(
        plan.files.iter().any(|f| f.path.starts_with("postman/")),
        "expected postman/* in plan, got {:?}",
        plan.files
    );

    let outcome = git_sync::run_sync(
        &ws,
        &SyncSelection::default(),
        &plan.token,
        "add postman mirror",
        &Auth::None,
        false,
    )
    .unwrap();
    assert!(
        matches!(outcome, SyncOutcome::Committed { .. }),
        "{outcome:?}"
    );
    assert!(ws.join("postman/api.json").exists());
}

#[test]
fn rematerialize_is_stable_when_nothing_changed() {
    let dir = workspace();
    workspace::set_share(dir.path(), Some(ShareConfig::postman())).unwrap();
    let first = share::materialize(dir.path()).unwrap();
    let a = std::fs::read_to_string(dir.path().join("postman/api.json")).unwrap();
    let second = share::materialize(dir.path()).unwrap();
    let b = std::fs::read_to_string(dir.path().join("postman/api.json")).unwrap();
    assert_eq!(a, b);
    assert_eq!(first.paths, second.paths);
}
