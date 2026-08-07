use mandalo_core::capability::{Decision, HostPolicy};
use mandalo_core::error::CoreError;
use mandalo_core::{collection, remote, workspace};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SHA: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";

/// A stand-in for GitHub, on loopback. Every remote test drives this: nothing in
/// the suite reaches the real network, and the shapes it answers with are the
/// two GitHub endpoints Mándalo reads plus raw file contents.
struct Fixture {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

async fn serve(routes: BTreeMap<String, (u16, String)>) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                let target = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                seen.lock().expect("hits").push(target.clone());
                let (status, body) = routes
                    .get(&target)
                    .cloned()
                    .unwrap_or((404, "not found".to_string()));
                let response = if (300..400).contains(&status) {
                    format!(
                        "HTTP/1.1 {status} Found\r\nLocation: {body}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    format!(
                        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Fixture {
        base: format!("http://127.0.0.1:{port}"),
        hits,
    }
}

impl Fixture {
    fn endpoints(&self) -> remote::Endpoints {
        remote::Endpoints::at(format!("{}/api", self.base), format!("{}/raw", self.base))
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("hits").clone()
    }
}

fn tree_json(files: &[(&str, usize)]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|(path, size)| {
            format!(r#"{{"path":"{path}","type":"blob","size":{size},"mode":"100644"}}"#)
        })
        .collect();
    format!(
        r#"{{"sha":"{SHA}","truncated":false,"tree":[{}]}}"#,
        entries.join(",")
    )
}

/// A whole small workspace: manifest, one collection, two requests, one
/// environment. The second request carries a script and a second host, so the
/// review has something to report on both counts.
fn sample_repo() -> Vec<(&'static str, String)> {
    vec![
        (
            "mandalo.toml",
            "schema_version = 1\nid = \"shared-apis\"\nname = \"Shared APIs\"\n".to_string(),
        ),
        (
            "collections/billing/collection.toml",
            "schema_version = 1\nid = \"billing\"\nname = \"Billing\"\n".to_string(),
        ),
        (
            "collections/billing/invoices.http",
            concat!(
                "### GET invoices\n",
                "GET https://api.billing.example/invoices\n",
                "Accept: application/json\n",
                "\n",
                "### POST charge\n",
                "POST https://payments.example.dev/charges\n",
                "Content-Type: application/json\n",
                "\n",
                "{\"amount\": 100}\n",
                "\n",
                "> {%\n",
                "pm.environment.set(\"last\", pm.response.json().id);\n",
                "%}\n",
            )
            .to_string(),
        ),
        (
            "collections/billing/lookup.http",
            "### GET lookup\nGET {{baseUrl}}/lookup\n".to_string(),
        ),
        (
            "environments/staging.toml",
            concat!(
                "name = \"staging\"\n",
                "\n",
                "[vars]\n",
                "baseUrl = \"https://staging.billing.example\"\n",
                "\n",
                "[vars.apiToken]\n",
                "secret = true\n",
                "hosts = [\"api.billing.example\"]\n",
            )
            .to_string(),
        ),
    ]
}

fn routes_for(files: &[(&'static str, String)]) -> BTreeMap<String, (u16, String)> {
    let mut routes = BTreeMap::new();
    routes.insert(
        "/api/repos/acme/apis/commits/HEAD".to_string(),
        (200, format!(r#"{{"sha":"{SHA}"}}"#)),
    );
    let listing: Vec<(&str, usize)> = files.iter().map(|(p, t)| (*p, t.len())).collect();
    routes.insert(
        format!("/api/repos/acme/apis/git/trees/{SHA}?recursive=1"),
        (200, tree_json(&listing)),
    );
    for (path, text) in files {
        routes.insert(format!("/raw/acme/apis/{SHA}/{path}"), (200, text.clone()));
    }
    routes
}

async fn fetch_sample(fixture: &Fixture) -> remote::RemoteFetch {
    let source = remote::parse_source("acme/apis").expect("source");
    remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .expect("fetch")
}

/// The tests talk to loopback on purpose, so the policy has to allow it — the
/// blocked-redirect test brings its own.
#[derive(Debug, Default, Clone, Copy)]
struct Open;

impl HostPolicy for Open {
    fn allow(&self, _host: &str, _ip: &IpAddr) -> Decision {
        Decision::Allow
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct NoMetadata;

impl HostPolicy for NoMetadata {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision {
        match ip {
            IpAddr::V4(v4) if v4.is_link_local() => {
                Decision::Deny(format!("{host} is a metadata endpoint"))
            }
            _ => Decision::Allow,
        }
    }
}

#[tokio::test]
async fn a_public_repository_is_read_reviewed_and_adopted_read_only() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");

    assert_eq!(review.collections, 1);
    assert_eq!(review.requests, 3);
    assert_eq!(
        review.hosts,
        vec![
            "api.billing.example".to_string(),
            "payments.example.dev".to_string()
        ],
        "every host the collection would contact, deduped"
    );
    assert_eq!(
        review.templated_hosts,
        vec!["{{baseUrl}}/lookup".to_string()]
    );
    assert_eq!(review.scripts.len(), 1);
    assert_eq!(review.scripts[0].request, "POST charge");
    assert_eq!(review.environments.len(), 1);
    assert_eq!(review.environments[0].name, "staging");
    assert_eq!(review.environments[0].shared_values, 1);
    assert_eq!(review.environments[0].awaiting_values, 1);
    assert_eq!(review.origin.commit.as_deref(), Some(SHA));

    let home = tempfile::tempdir().expect("home");
    let registry = home.path().join("workspaces.toml");
    let dest = home.path().join("Shared APIs");
    let info = remote::adopt(&fetched, &review.token, &registry, &dest).expect("adopt");

    let origin = remote::origin(&dest).expect("origin").expect("is remote");
    assert_eq!(origin.commit.as_deref(), Some(SHA));
    assert!(origin.label.contains("github.com/acme/apis"));
    assert_eq!(info.path, dest.display().to_string());
    assert_eq!(
        collection::list_tree(&dest)
            .expect("tree")
            .collections
            .len(),
        1
    );
}

#[tokio::test]
async fn nothing_is_fetched_or_written_beyond_what_the_review_described() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");
    let before = fixture.hits().len();

    let home = tempfile::tempdir().expect("home");
    remote::adopt(
        &fetched,
        &review.token,
        &home.path().join("registry.toml"),
        &home.path().join("ws"),
    )
    .expect("adopt");

    assert_eq!(
        fixture.hits().len(),
        before,
        "adopting must write the bytes already in hand, not fetch again"
    );
}

#[tokio::test]
async fn a_review_token_from_a_different_fetch_is_refused() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let home = tempfile::tempdir().expect("home");
    let error = remote::adopt(
        &fetched,
        "not-the-token",
        &home.path().join("registry.toml"),
        &home.path().join("ws"),
    )
    .unwrap_err();
    assert_eq!(error.code(), "E_CONFLICT");
}

#[tokio::test]
async fn a_remote_workspace_refuses_every_write_until_it_is_copied_out() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");
    let home = tempfile::tempdir().expect("home");
    let registry = home.path().join("workspaces.toml");
    let dest = home.path().join("remote");
    remote::adopt(&fetched, &review.token, &registry, &dest).expect("adopt");

    let request = collection::load_request(&dest, "billing", "invoices.http#0").expect("request");
    let denied = collection::save_request(&dest, "billing", None, None, &request).unwrap_err();
    assert_eq!(denied.code(), "E_READ_ONLY");
    assert!(
        denied.to_string().contains("Save a copy") || denied.to_string().contains("save a copy")
    );

    assert_eq!(
        collection::create_collection(&dest, "mine")
            .unwrap_err()
            .code(),
        "E_READ_ONLY"
    );
    assert_eq!(
        collection::delete_collection(&dest, "billing")
            .unwrap_err()
            .code(),
        "E_READ_ONLY"
    );
    assert_eq!(
        workspace::delete_environment(&dest, "staging")
            .unwrap_err()
            .code(),
        "E_READ_ONLY"
    );
    assert_eq!(
        mandalo_core::sample::add_sample_collection(&dest)
            .unwrap_err()
            .code(),
        "E_READ_ONLY"
    );

    let mine = home.path().join("mine");
    remote::save_copy(&dest, &registry, &mine, "My copy").expect("save a copy");
    assert!(remote::origin(&mine).expect("origin").is_none());
    collection::create_collection(&mine, "scratch").expect("the copy is an ordinary workspace");
    assert!(
        remote::origin(&dest).expect("origin").is_some(),
        "the original stays read-only"
    );
}

#[tokio::test]
async fn opening_a_remote_workspace_again_does_not_lose_the_read_only_stamp() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");
    let home = tempfile::tempdir().expect("home");
    let registry = home.path().join("workspaces.toml");
    let dest = home.path().join("remote");
    remote::adopt(&fetched, &review.token, &registry, &dest).expect("adopt");

    workspace::open_workspace(&registry, &dest).expect("reopen");
    assert!(remote::origin(&dest).expect("origin").is_some());
}

#[tokio::test]
async fn a_credential_in_the_repository_is_named_in_the_review() {
    let mut files = sample_repo();
    files.push((
        "collections/billing/leaky.http",
        concat!(
            "### GET leaky\n",
            "GET https://api.billing.example/me\n",
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk\n",
        )
        .to_string(),
    ));
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");

    assert!(
        review.findings.iter().any(|f| f.rule == "jwt"),
        "the scanner runs over what was fetched: {:?}",
        review.findings
    );
    assert!(
        review
            .findings
            .iter()
            .all(|f| f.path.is_relative() && !f.path.starts_with("/")),
        "a finding names the file inside the collection, not a scratch path"
    );
}

#[tokio::test]
async fn a_redirect_to_a_blocked_host_is_refused_and_nothing_lands() {
    let mut routes = routes_for(&sample_repo());
    routes.insert(
        "/api/repos/acme/apis/commits/HEAD".to_string(),
        (302, "http://169.254.169.254/latest/meta-data".to_string()),
    );
    let fixture = serve(routes).await;
    let source = remote::parse_source("acme/apis").expect("source");
    let error = remote::fetch(&source, &fixture.endpoints(), &NoMetadata)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_HOST_DENIED", "{error}");
}

#[tokio::test]
async fn a_file_over_the_size_cap_is_skipped_rather_than_read() {
    let mut files = sample_repo();
    let mut routes = routes_for(&files);
    files.push(("collections/billing/huge.http", String::new()));
    let mut listing: Vec<(&str, usize)> = files.iter().map(|(p, t)| (*p, t.len())).collect();
    listing.last_mut().expect("huge").1 = remote::MAX_FILE_BYTES + 1;
    routes.insert(
        format!("/api/repos/acme/apis/git/trees/{SHA}?recursive=1"),
        (200, tree_json(&listing)),
    );
    let fixture = serve(routes).await;
    let fetched = fetch_sample(&fixture).await;

    assert!(fetched.skipped.iter().any(|s| s.contains("huge.http")));
    assert!(
        !fixture.hits().iter().any(|hit| hit.contains("huge.http")),
        "an over-size file is never even requested"
    );
}

#[tokio::test]
async fn too_many_files_and_too_many_bytes_are_both_refused() {
    let files = sample_repo();
    let mut routes = routes_for(&files);
    let names: Vec<String> = (0..remote::MAX_FILES + 5)
        .map(|n| format!("collections/billing/r{n}.http"))
        .collect();
    let listing: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), 10usize)).collect();
    routes.insert(
        format!("/api/repos/acme/apis/git/trees/{SHA}?recursive=1"),
        (200, tree_json(&listing)),
    );
    let fixture = serve(routes).await;
    let source = remote::parse_source("acme/apis").expect("source");
    let error = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("more than"), "{error}");

    let big: Vec<(&str, usize)> = vec![
        ("collections/billing/a.http", remote::MAX_FILE_BYTES),
        ("collections/billing/b.http", remote::MAX_FILE_BYTES),
        ("collections/billing/c.http", remote::MAX_FILE_BYTES),
        ("collections/billing/d.http", remote::MAX_FILE_BYTES),
        ("collections/billing/e.http", remote::MAX_FILE_BYTES),
        ("collections/billing/f.http", remote::MAX_FILE_BYTES),
        ("collections/billing/g.http", remote::MAX_FILE_BYTES),
        ("collections/billing/h.http", remote::MAX_FILE_BYTES),
        ("collections/billing/i.http", remote::MAX_FILE_BYTES),
    ];
    let mut routes = routes_for(&files);
    routes.insert(
        format!("/api/repos/acme/apis/git/trees/{SHA}?recursive=1"),
        (200, tree_json(&big)),
    );
    let fixture = serve(routes).await;
    let error = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bytes"), "{error}");
}

#[tokio::test]
async fn a_repository_that_is_not_a_workspace_fails_loud_and_writes_nothing() {
    let files: Vec<(&'static str, String)> = vec![
        ("README.md", "# just a repo\n".to_string()),
        ("docs/notes.txt", "nothing to see\n".to_string()),
    ];
    let fixture = serve(routes_for(&files)).await;
    let source = remote::parse_source("acme/apis").expect("source");
    let error = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_NOT_FOUND");
    assert!(error.to_string().contains("collections/"), "{error}");
}

#[tokio::test]
async fn a_repository_with_a_corrupt_request_file_fails_before_anything_is_adopted() {
    let mut files = sample_repo();
    files.push((
        "collections/billing/collection.toml",
        "this is not toml [[[".to_string(),
    ));
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let home = tempfile::tempdir().expect("home");
    let dest = home.path().join("ws");
    let outcome = remote::review(&fetched).and_then(|review| {
        remote::adopt(
            &fetched,
            &review.token,
            &home.path().join("registry.toml"),
            &dest,
        )
    });
    assert!(
        outcome.is_err(),
        "a malformed collection must not half-load"
    );
    assert!(!dest.join("collections").exists(), "nothing was written");
}

#[tokio::test]
async fn a_private_or_missing_repository_says_both_and_points_at_cloning() {
    let mut routes = routes_for(&sample_repo());
    routes.insert(
        "/api/repos/acme/apis/commits/HEAD".to_string(),
        (404, r#"{"message":"Not Found"}"#.to_string()),
    );
    let fixture = serve(routes).await;
    let source = remote::parse_source("acme/apis").expect("source");
    let error = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_PRIVATE");
    assert!(error.to_string().contains("may not exist"), "{error}");
    assert!(error.to_string().contains("private"), "{error}");
    assert!(error.to_string().contains("clone"), "{error}");
    assert_eq!(
        remote::clone_url(&source).as_deref(),
        Some("https://github.com/acme/apis.git")
    );
}

#[tokio::test]
async fn a_single_bundle_url_opens_the_same_way() {
    let bundle = serde_json::json!({
        "mandaloBundle": 2,
        "collections": [{
            "id": "b1", "slug": "team", "name": "Team",
            "requests": [{
                "path": "api.http#0",
                "request": {
                    "id": "r1", "name": "Ping", "kind": "http",
                    "method": "GET", "url": "https://ping.example.dev/health",
                    "headers": [], "auth": {"type": "none"}
                }
            }]
        }],
        "requests": [],
        "environments": []
    })
    .to_string();
    let mut routes = BTreeMap::new();
    routes.insert("/raw/team.json".to_string(), (200, bundle));
    let fixture = serve(routes).await;

    let url = format!("{}/raw/team.json", fixture.base);
    let source = remote::parse_source(&url).expect("source");
    assert!(matches!(source, remote::RemoteSource::Document { .. }));
    let fetched = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .expect("fetch");
    let review = remote::review(&fetched).expect("review");
    assert_eq!(review.requests, 1);
    assert_eq!(review.hosts, vec!["ping.example.dev".to_string()]);

    let home = tempfile::tempdir().expect("home");
    let dest = home.path().join("team");
    remote::adopt(
        &fetched,
        &review.token,
        &home.path().join("registry.toml"),
        &dest,
    )
    .expect("adopt");
    assert!(remote::origin(&dest).expect("origin").is_some());
}

#[tokio::test]
async fn a_url_that_is_not_a_bundle_is_refused_by_name() {
    let mut routes = BTreeMap::new();
    routes.insert(
        "/raw/openapi.json".to_string(),
        (200, r#"{"openapi":"3.0.0"}"#.to_string()),
    );
    let fixture = serve(routes).await;
    let url = format!("{}/raw/openapi.json", fixture.base);
    let source = remote::parse_source(&url).expect("source");
    let error = remote::fetch(&source, &fixture.endpoints(), &Open)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bundle"), "{error}");
}

#[tokio::test]
async fn a_remote_environment_carries_declarations_and_never_a_stored_value() {
    let files = sample_repo();
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;
    let review = remote::review(&fetched).expect("review");
    let home = tempfile::tempdir().expect("home");
    let dest = home.path().join("ws");
    remote::adopt(
        &fetched,
        &review.token,
        &home.path().join("registry.toml"),
        &dest,
    )
    .expect("adopt");

    let doc = workspace::read_env_doc(&dest, "staging")
        .expect("read")
        .expect("staging");
    let secret = doc.vars.get("apiToken").expect("apiToken");
    assert!(secret.is_secret());
    assert!(
        !secret.is_shared(),
        "a secret declaration has no value field to carry one in"
    );
    let raw = std::fs::read_to_string(dest.join("environments/staging.toml")).expect("raw");
    assert!(!raw.contains("value"), "{raw}");
}

#[tokio::test]
async fn a_path_that_climbs_out_of_the_workspace_is_never_written() {
    let files: Vec<(&'static str, String)> = vec![
        (
            "mandalo.toml",
            "schema_version = 1\nid = \"x\"\nname = \"X\"\n".to_string(),
        ),
        (
            "collections/a/one.http",
            "### GET one\nGET https://a.example.dev/one\n".to_string(),
        ),
        (
            "../../escaped.http",
            "### GET x\nGET https://x/\n".to_string(),
        ),
        (".env", "TOKEN=hunter2hunter2\n".to_string()),
    ];
    let fixture = serve(routes_for(&files)).await;
    let fetched = fetch_sample(&fixture).await;

    match &fetched.payload {
        remote::RemotePayload::Tree(files) => {
            assert!(files
                .iter()
                .all(|(p, _)| p == "mandalo.toml" || p.starts_with("collections/")));
        }
        other => panic!("expected a tree, got {other:?}"),
    }
    assert!(fetched.skipped.iter().any(|s| s.contains(".env")));
}

#[test]
fn the_sample_collection_lands_beside_whatever_is_already_there() {
    let home = tempfile::tempdir().expect("home");
    let registry = home.path().join("workspaces.toml");
    let dir = home.path().join("ws");
    workspace::create_workspace(&registry, &dir, "Mine").expect("workspace");

    assert_eq!(
        mandalo_core::sample::add_sample_collection(&dir).expect("first"),
        "mock"
    );
    assert_eq!(
        mandalo_core::sample::add_sample_collection(&dir).expect("second"),
        "mock-2"
    );
    assert_eq!(
        mandalo_core::sample::add_sample_collection(&dir).expect("third"),
        "mock-3"
    );

    let tree = collection::list_tree(&dir).expect("tree");
    let slugs: Vec<&str> = tree.collections.iter().map(|c| c.slug.as_str()).collect();
    assert!(slugs.contains(&"mock"));
    assert!(slugs.contains(&"mock-2"));
    assert!(slugs.contains(&"mock-3"));
}

#[test]
fn adding_the_sample_never_touches_a_collection_the_user_already_has() {
    let home = tempfile::tempdir().expect("home");
    let registry = home.path().join("workspaces.toml");
    let dir = home.path().join("ws");
    workspace::create_workspace(&registry, &dir, "Mine").expect("workspace");
    collection::create_collection(&dir, "Mine own").expect("collection");
    let file = collection::collections_dir(&dir).join("mine-own/api.http");
    std::fs::write(&file, "### GET mine\nGET https://mine.example.dev/\n").expect("write");
    let before = std::fs::read_to_string(&file).expect("read");

    mandalo_core::sample::add_sample_collection(&dir).expect("sample");

    assert_eq!(std::fs::read_to_string(&file).expect("read"), before);
    assert!(collection::collections_dir(&dir).join("mock").is_dir());
}

#[test]
fn a_workspace_that_was_never_opened_says_so_instead_of_scaffolding_itself() {
    let home = tempfile::tempdir().expect("home");
    let dir = home.path().join("not-a-workspace");
    std::fs::create_dir_all(&dir).expect("dir");
    let error = mandalo_core::sample::add_sample_collection(&dir).unwrap_err();
    assert_eq!(error.code(), "E_NOT_FOUND");
}

#[test]
fn every_sample_file_the_browser_inlines_is_in_the_desktop_table() {
    let table: BTreeMap<&str, usize> = mandalo_core::sample::files()
        .iter()
        .map(|(path, text)| (*path, text.len()))
        .collect();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mock-workspace");
    for (path, size) in &table {
        let on_disk = std::fs::read_to_string(root.join(path)).expect(path);
        assert_eq!(on_disk.len(), *size, "{path} drifted from the fixture");
    }
    assert!(table.contains_key("mandalo.toml"));
    assert!(table.keys().any(|p| p.starts_with("collections/mock/")));
    assert!(table.keys().any(|p| p.starts_with("environments/")));
}

#[test]
fn the_read_only_error_is_the_one_the_shell_can_act_on() {
    let error = CoreError::ReadOnly("x".to_string());
    assert_eq!(error.code(), "E_READ_ONLY");
}
