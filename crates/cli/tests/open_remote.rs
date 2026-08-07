use assert_cmd::Command;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;

const SHA: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";

/// A stand-in for GitHub on loopback. `mandalo open` is pointed at it with the
/// hidden endpoint flags, so the suite never reaches the network.
fn serve(routes: BTreeMap<String, (u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let mut received = Vec::new();
            loop {
                let Ok(n) = stream.read(&mut buf) else { return };
                received.extend_from_slice(&buf[..n]);
                if n == 0 || received.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&received).to_string();
            let target = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let (status, body) = routes
                .get(&target)
                .cloned()
                .unwrap_or((404, "not found".to_string()));
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn files() -> Vec<(&'static str, String)> {
    vec![
        (
            "mandalo.toml",
            "schema_version = 1\nid = \"shared\"\nname = \"Shared APIs\"\n".to_string(),
        ),
        (
            "collections/billing/collection.toml",
            "schema_version = 1\nid = \"billing\"\nname = \"Billing\"\n".to_string(),
        ),
        (
            "collections/billing/invoices.http",
            "### GET invoices\nGET https://api.billing.example/invoices\n".to_string(),
        ),
    ]
}

fn routes() -> BTreeMap<String, (u16, String)> {
    let mut routes = BTreeMap::new();
    routes.insert(
        "/repos/acme/apis/commits/HEAD".to_string(),
        (200, format!(r#"{{"sha":"{SHA}"}}"#)),
    );
    let tree: Vec<String> = files()
        .iter()
        .map(|(path, text)| format!(r#"{{"path":"{path}","type":"blob","size":{}}}"#, text.len()))
        .collect();
    routes.insert(
        format!("/repos/acme/apis/git/trees/{SHA}?recursive=1"),
        (
            200,
            format!(r#"{{"truncated":false,"tree":[{}]}}"#, tree.join(",")),
        ),
    );
    for (path, text) in files() {
        routes.insert(format!("/acme/apis/{SHA}/{path}"), (200, text));
    }
    routes
}

fn mandalo() -> Command {
    Command::cargo_bin("mandalo").unwrap()
}

#[test]
fn open_reads_a_public_repository_and_says_what_it_is_before_opening_it() {
    let base = serve(routes());
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join("shared");

    let out = mandalo()
        .args([
            "open",
            "acme/apis",
            dest.to_str().unwrap(),
            "--yes",
            "--github-api",
            &base,
            "--github-raw",
            &base,
        ])
        .env("HOME", home.path())
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    assert!(text.contains("github.com/acme/apis"), "{text}");
    assert!(text.contains("api.billing.example"), "{text}");
    assert!(text.contains("carries no scripts"), "{text}");
    assert!(text.contains("read-only"), "{text}");
    assert!(dest.join("collections/billing/invoices.http").exists());
    assert!(std::fs::read_to_string(dest.join("mandalo.toml"))
        .unwrap()
        .contains("[remote]"));
}

#[test]
fn review_only_prints_the_review_and_opens_nothing() {
    let base = serve(routes());
    let home = tempfile::tempdir().unwrap();

    let out = mandalo()
        .args([
            "open",
            "acme/apis",
            "--review-only",
            "--github-api",
            &base,
            "--github-raw",
            &base,
        ])
        .env("HOME", home.path())
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let review: serde_json::Value = serde_json::from_str(text.trim()).unwrap();

    assert_eq!(review["requests"], 1);
    assert_eq!(review["hosts"][0], "api.billing.example");
    assert!(review["token"].as_str().is_some());
    assert!(!home.path().join("apis").exists());
}

#[test]
fn a_remote_workspace_refuses_a_write_and_save_copy_is_the_way_out() {
    let base = serve(routes());
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join("shared");

    mandalo()
        .args([
            "open",
            "acme/apis",
            dest.to_str().unwrap(),
            "--yes",
            "--github-api",
            &base,
            "--github-raw",
            &base,
        ])
        .env("HOME", home.path())
        .assert()
        .success();

    mandalo()
        .args([
            "--workspace",
            dest.to_str().unwrap(),
            "env",
            "set",
            "staging",
            "baseUrl",
            "https://x.dev",
        ])
        .env("HOME", home.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("E_READ_ONLY"));

    let copy = home.path().join("mine");
    mandalo()
        .args([
            "--workspace",
            dest.to_str().unwrap(),
            "save-copy",
            copy.to_str().unwrap(),
        ])
        .env("HOME", home.path())
        .assert()
        .success();

    assert!(!std::fs::read_to_string(copy.join("mandalo.toml"))
        .unwrap()
        .contains("[remote]"));
    mandalo()
        .args([
            "--workspace",
            copy.to_str().unwrap(),
            "env",
            "set",
            "staging",
            "baseUrl",
            "https://x.dev",
        ])
        .env("HOME", home.path())
        .assert()
        .success();
}

#[test]
fn a_private_or_missing_repository_names_both_possibilities_and_points_at_cloning() {
    let mut routes = routes();
    routes.insert(
        "/repos/acme/apis/commits/HEAD".to_string(),
        (404, r#"{"message":"Not Found"}"#.to_string()),
    );
    let base = serve(routes);
    let home = tempfile::tempdir().unwrap();

    let out = mandalo()
        .args([
            "open",
            "acme/apis",
            "--yes",
            "--github-api",
            &base,
            "--github-raw",
            &base,
        ])
        .env("HOME", home.path())
        .assert()
        .failure();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    assert!(text.contains("may not exist"), "{text}");
    assert!(text.contains("private"), "{text}");
    assert!(text.contains("mandalo login && mandalo clone"), "{text}");
}

#[test]
fn a_url_with_a_credential_in_it_is_refused_without_echoing_it() {
    let home = tempfile::tempdir().unwrap();

    let out = mandalo()
        .args([
            "open",
            "https://ghp_abcdefghijklmnopqrstu@github.com/acme/apis",
        ])
        .env("HOME", home.path())
        .assert()
        .failure();
    let text = String::from_utf8(out.get_output().stderr.clone()).unwrap();

    assert!(text.contains("E_SECRET"), "{text}");
    assert!(
        !text.contains("ghp_abcdefghij"),
        "the credential was echoed"
    );
}

#[test]
fn the_sample_collection_can_always_be_added_and_never_overwrites() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("ws");

    mandalo()
        .args(["init", dir.to_str().unwrap()])
        .env("HOME", home.path())
        .assert()
        .success();

    for expected in ["mock", "mock-2"] {
        assert!(mandalo_core::sample::add_sample_collection(&dir).unwrap() == expected);
    }
    assert!(dir.join("collections/mock").is_dir());
    assert!(dir.join("collections/mock-2").is_dir());
}
