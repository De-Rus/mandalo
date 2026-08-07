use mandalo_core::capability::{EnvVarStore, LayeredSecrets, RefuseLocalWrites};
use mandalo_core::request::Auth;
use mandalo_core::runner::{env_frame, Runner};
use mandalo_core::secrets::LocalStore;
use mandalo_core::workspace::{self, EnvDoc, Environment, VarDef};
use mandalo_core::{bundle, git_sync, scan, AllowAll, SecretStore, SecretWriter};
use mandalo_testkit::fixtures;
use mandalo_testkit::MockApi;
use std::path::Path;

const WS: &str = "550e8400-e29b-41d4-a716-446655440000";
const SECRET: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

fn store(home: &Path) -> LocalStore {
    LocalStore::at(home.join("secrets.toml"), WS)
}

fn env_file(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join("environments").join(format!("{name}.toml"))).unwrap()
}

fn write_env_file(dir: &Path, name: &str, raw: &str) {
    std::fs::create_dir_all(dir.join("environments")).unwrap();
    std::fs::write(dir.join("environments").join(format!("{name}.toml")), raw).unwrap();
}

// ---------------------------------------------------------------- the format

#[test]
fn the_committed_file_carries_declarations_and_never_an_unshared_value() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let store = store(home.path());

    let mut doc = EnvDoc::new("prod");
    doc.vars.insert(
        "baseUrl".to_string(),
        VarDef::shared("https://api.acme.com"),
    );
    doc.vars.insert("devUrl".to_string(), VarDef::local(&[]));
    doc.vars.insert(
        "access_token".to_string(),
        VarDef::secret(&["api.acme.com"]),
    );
    workspace::save_env_doc(dir.path(), &doc).unwrap();

    store
        .set("prod", "devUrl", "http://localhost:3000")
        .unwrap();
    store.set("prod", "access_token", SECRET).unwrap();

    let raw = env_file(dir.path(), "prod");
    assert_eq!(
        raw,
        "schema_version = 1\nname = \"prod\"\n\n\
         [vars.access_token]\nsecret = true\nhosts = [\"api.acme.com\"]\n\n\
         [vars.baseUrl]\nvalue = \"https://api.acme.com\"\n\n\
         [vars.devUrl]\nshared = false\nhosts = []\n"
    );
    assert!(!raw.contains(SECRET));
    assert!(!raw.contains("localhost:3000"));
    assert_eq!(
        workspace::read_env_doc(dir.path(), "prod").unwrap(),
        Some(doc)
    );
}

#[test]
fn a_shared_secret_is_impossible_to_declare() {
    let dir = tempfile::tempdir().unwrap();
    write_env_file(
        dir.path(),
        "prod",
        "name = \"prod\"\n\n[vars.token]\nsecret = true\nshared = true\n",
    );
    let err = workspace::read_env_doc(dir.path(), "prod").unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
    assert!(
        err.to_string().contains("a secret is never shared"),
        "{err}"
    );
}

#[test]
fn a_value_on_an_unshared_declaration_is_a_hard_error() {
    for (declaration, expected) in [
        ("secret = true", "set-secret"),
        ("shared = false", "set-local"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_env_file(
            dir.path(),
            "prod",
            &format!("name = \"prod\"\n\n[vars.token]\n{declaration}\nvalue = \"leaked-in-git\"\n"),
        );
        let err = workspace::read_env_doc(dir.path(), "prod").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(err.to_string().contains(expected), "{err}");
        assert!(!err.to_string().contains("leaked-in-git"), "{err}");
        assert_eq!(
            workspace::list_env_docs(dir.path()).unwrap_err().code(),
            "E_SCHEMA",
            "a listing must not quietly skip the dangerous file"
        );
    }
}

#[test]
fn a_bare_value_means_shared() {
    let dir = tempfile::tempdir().unwrap();
    write_env_file(
        dir.path(),
        "prod",
        "name = \"prod\"\n[vars]\nbaseUrl = \"https://x.dev\"\n",
    );
    let doc = workspace::read_env_doc(dir.path(), "prod")
        .unwrap()
        .unwrap();
    assert_eq!(doc.vars["baseUrl"], VarDef::shared("https://x.dev"));
    assert!(doc.vars["baseUrl"].is_shared());
    assert!(!doc.vars["baseUrl"].is_secret());
}

#[test]
fn hosts_bind_either_unshared_kind_and_never_a_shared_one() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let store = store(home.path());

    workspace::set_secret(dir.path(), &store, "prod", "token", SECRET).unwrap();
    workspace::set_local(dir.path(), &store, "prod", "devUrl", "http://box").unwrap();
    assert_eq!(
        workspace::bind_secret_host(dir.path(), "prod", "token", "API.Acme.com").unwrap(),
        vec!["api.acme.com".to_string()]
    );
    assert_eq!(
        workspace::bind_secret_host(dir.path(), "prod", "devUrl", "box").unwrap(),
        vec!["box".to_string()]
    );

    write_env_file(
        dir.path(),
        "other",
        "name = \"other\"\n\n[vars.baseUrl]\nvalue = \"v\"\nhosts = [\"x.dev\"]\n",
    );
    let err = workspace::read_env_doc(dir.path(), "other").unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
    assert!(err.to_string().contains("nothing left to protect"), "{err}");
}

// ------------------------------------------------------------- the precedence

#[test]
fn an_exported_variable_beats_the_local_file_which_beats_the_committed_value() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let local = store(home.path());
    fixtures::workspace(dir.path());

    let mut doc = EnvDoc::new("prod");
    doc.vars.insert(
        "baseUrl".to_string(),
        VarDef::shared("https://team.example"),
    );
    workspace::save_env_doc(dir.path(), &doc).unwrap();

    let chain = LayeredSecrets::over(store(home.path()));
    let frame = env_frame(dir.path(), &chain, Some("prod")).unwrap();
    assert_eq!(frame.get("baseUrl"), Some("https://team.example"));

    local
        .set("prod", "baseUrl", "http://localhost:3000")
        .unwrap();
    let frame = env_frame(dir.path(), &chain, Some("prod")).unwrap();
    assert_eq!(
        frame.get("baseUrl"),
        Some("http://localhost:3000"),
        "this machine may override a shared value without touching the file"
    );

    let name = EnvVarStore::variable_name("prod", "baseUrl");
    std::env::set_var(&name, "https://ci.example");
    let frame = env_frame(dir.path(), &chain, Some("prod")).unwrap();
    assert_eq!(
        frame.get("baseUrl"),
        Some("https://ci.example"),
        "CI injects the environment variable and it must win"
    );
    std::env::remove_var(&name);

    assert_eq!(
        env_file(dir.path(), "prod"),
        "schema_version = 1\nname = \"prod\"\n\n[vars.baseUrl]\nvalue = \"https://team.example\"\n",
        "no override may ever reach the committed file"
    );
}

#[tokio::test]
async fn a_declared_secret_with_no_value_anywhere_fails_loud_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mock = MockApi::start().await;
    let slug = fixtures::workspace(dir.path());

    let mut doc = EnvDoc::new("prod");
    doc.vars.insert("token".to_string(), VarDef::secret(&[]));
    workspace::save_env_doc(dir.path(), &doc).unwrap();

    let mut request = fixtures::request("Whoami", "GET", &mock.url("/headers/echo"));
    request.auth = Auth::Bearer {
        token: "{{token}}".to_string(),
    };
    mandalo_core::collection::save_request(dir.path(), &slug, None, None, &request).unwrap();

    let runner = Runner::new(LayeredSecrets::over(store(home.path())), AllowAll);
    let error = runner
        .run_one(dir.path(), &request, Some("prod"))
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_SECRET");
    let text = error.to_string();
    assert!(text.contains("prod.token"), "{text}");
    assert!(text.contains("mandalo env set-secret prod token"), "{text}");
    assert!(text.contains("MANDALO_SECRET__PROD__TOKEN"), "{text}");
    assert!(
        mock.last_request().is_none(),
        "an empty value must never be sent"
    );
}

#[test]
fn a_workspace_from_a_colleague_names_what_it_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut doc = EnvDoc::new("prod");
    doc.vars.insert("token".to_string(), VarDef::secret(&[]));
    doc.vars.insert("devUrl".to_string(), VarDef::local(&[]));
    doc.vars
        .insert("baseUrl".to_string(), VarDef::shared("https://x.dev"));
    workspace::save_env_doc(dir.path(), &doc).unwrap();

    let chain = LayeredSecrets::over(store(home.path()));
    assert_eq!(
        workspace::missing_values(dir.path(), &chain, "prod").unwrap(),
        vec!["devUrl".to_string(), "token".to_string()]
    );

    store(home.path()).set("prod", "token", SECRET).unwrap();
    assert_eq!(
        workspace::missing_values(dir.path(), &chain, "prod").unwrap(),
        vec!["devUrl".to_string()]
    );
}

// --------------------------------------------------------- the writer routing

#[test]
fn saving_an_environment_routes_every_value_by_its_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let local = store(home.path());

    workspace::set_secret(dir.path(), &local, "prod", "token", SECRET).unwrap();
    workspace::set_local(dir.path(), &local, "prod", "devUrl", "http://old").unwrap();

    workspace::save_environment(
        dir.path(),
        &local,
        &Environment {
            name: "prod".to_string(),
            vars: [
                ("baseUrl", "https://api.acme.com"),
                ("token", "typed-into-the-editor"),
                ("devUrl", "http://localhost:3000"),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        },
    )
    .unwrap();

    let raw = env_file(dir.path(), "prod");
    assert!(raw.contains("value = \"https://api.acme.com\""), "{raw}");
    assert!(!raw.contains("typed-into-the-editor"), "{raw}");
    assert!(!raw.contains("localhost:3000"), "{raw}");
    assert!(raw.contains("secret = true"), "{raw}");
    assert!(raw.contains("shared = false"), "{raw}");

    assert_eq!(
        local.get("prod", "token").unwrap(),
        Some("typed-into-the-editor".to_string())
    );
    assert_eq!(
        local.get("prod", "devUrl").unwrap(),
        Some("http://localhost:3000".to_string())
    );
}

#[test]
fn emptying_an_unshared_field_forgets_the_value_and_keeps_the_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let local = store(home.path());
    workspace::set_secret(dir.path(), &local, "prod", "token", SECRET).unwrap();

    workspace::save_environment(
        dir.path(),
        &local,
        &Environment {
            name: "prod".to_string(),
            vars: [("token".to_string(), String::new())].into_iter().collect(),
        },
    )
    .unwrap();

    assert_eq!(local.get("prod", "token").unwrap(), None);
    assert!(workspace::read_env_doc(dir.path(), "prod")
        .unwrap()
        .unwrap()
        .vars["token"]
        .is_secret());
}

#[test]
fn an_import_cannot_give_a_value_to_something_that_is_not_shared() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    workspace::set_secret(dir.path(), &store(home.path()), "prod", "token", SECRET).unwrap();

    let error = workspace::save_environment(
        dir.path(),
        &RefuseLocalWrites,
        &Environment {
            name: "prod".to_string(),
            vars: [("token".to_string(), "from-an-import".to_string())]
                .into_iter()
                .collect(),
        },
    )
    .unwrap_err();

    assert_eq!(error.code(), "E_SECRET");
    assert!(!env_file(dir.path(), "prod").contains("from-an-import"));
}

// --------------------------------------------------------- never leaving here

#[test]
fn an_export_carries_declarations_and_no_local_value() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let local = store(home.path());
    fixtures::workspace(dir.path());

    workspace::set_secret(dir.path(), &local, "prod", "token", SECRET).unwrap();
    workspace::set_local(
        dir.path(),
        &local,
        "prod",
        "devUrl",
        "http://localhost:3000",
    )
    .unwrap();

    let exported = bundle::export(dir.path()).unwrap();
    assert!(!exported.json.contains(SECRET), "{}", exported.json);
    assert!(
        !exported.json.contains("localhost:3000"),
        "{}",
        exported.json
    );

    let parsed: serde_json::Value = serde_json::from_str(&exported.json).unwrap();
    let vars = &parsed["environments"][0]["vars"];
    assert_eq!(vars["token"]["secret"], true);
    assert!(vars["token"]["value"].is_null());
    assert_eq!(vars["devUrl"]["shared"], false);
    assert!(vars["devUrl"]["value"].is_null());
}

#[test]
fn the_scanner_refuses_a_local_values_file_that_turns_up_inside_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    fixtures::workspace(dir.path());
    std::fs::write(
        dir.path().join("secrets.toml"),
        format!("[workspaces.\"{WS}\".prod]\ntoken = \"{SECRET}\"\n"),
    )
    .unwrap();

    let findings = scan::scan_workspace(dir.path()).unwrap();
    let found = findings
        .iter()
        .find(|f| f.rule == "local-values-file")
        .expect("the scanner must name the file");
    assert!(found.path.ends_with("secrets.toml"));
    assert!(!found.excerpt.contains(SECRET), "{found:?}");

    assert!(scan::is_local_values_file(Path::new("prod.local.toml")));
    assert!(scan::is_local_values_file(Path::new("/a/b/secrets.toml")));
    assert!(!scan::is_local_values_file(Path::new("prod.toml")));
}

#[test]
fn a_local_values_file_is_never_staged_by_the_sync_path() {
    let dir = tempfile::tempdir().unwrap();
    fixtures::workspace(dir.path());
    let doomed = dir.path().join("secrets.toml");
    std::fs::write(
        &doomed,
        format!("[workspaces.\"{WS}\".prod]\ntoken = \"x\"\n"),
    )
    .unwrap();

    let repo = git2::Repository::init(dir.path()).unwrap();
    // No .gitignore at all, and named explicitly: the rule is gone and the
    // reviewer picked the file anyway.
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("secrets.toml")).unwrap();
    index.write().unwrap();
    assert!(index.get_path(Path::new("secrets.toml"), 0).is_some());

    let selection = git_sync::SyncSelection::default();
    let plan = git_sync::plan_sync(dir.path(), &selection, None).unwrap();
    git_sync::run_sync(
        dir.path(),
        &selection,
        &plan.token,
        "Update workspace",
        &git_sync::Auth::None,
        true,
    )
    .unwrap();

    let repo = git2::Repository::open(dir.path()).unwrap();
    let staged = repo.index().unwrap();
    assert!(
        staged.get_path(Path::new("secrets.toml"), 0).is_none(),
        "the one file whose commit is catastrophic must never reach the index"
    );
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    assert!(
        tree.get_name("secrets.toml").is_none(),
        "nor the commit it produces"
    );
    assert!(
        doomed.exists(),
        "it stays on disk; only the commit is refused"
    );
}

#[test]
fn the_managed_gitignore_block_covers_every_name_the_scanner_refuses() {
    let dir = tempfile::tempdir().unwrap();
    mandalo_core::git::ensure_git_hygiene(dir.path()).unwrap();
    let raw = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    for pattern in ["secrets.toml", "*.local.toml"] {
        assert!(raw.contains(pattern), "{raw}");
        assert!(
            scan::is_local_values_file(Path::new(&pattern.replace('*', "prod"))),
            "the ignore rule and the scanner must agree on {pattern}"
        );
    }
}

#[test]
fn the_environments_directory_ignores_a_stray_local_file() {
    let dir = tempfile::tempdir().unwrap();
    write_env_file(
        dir.path(),
        "prod",
        "name = \"prod\"\n[vars]\nbase = \"b\"\n",
    );
    std::fs::write(
        dir.path().join("environments").join("prod.local.toml"),
        "token = \"x\"\n",
    )
    .unwrap();

    let listed = workspace::list_env_docs(dir.path()).unwrap();
    assert_eq!(listed.items.len(), 1);
    assert!(listed.skipped.is_empty(), "{:?}", listed.skipped);
}
