use assert_cmd::Command;
use std::path::Path;

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("environments")).unwrap();
    std::fs::create_dir_all(dir.path().join("collections/api")).unwrap();
    std::fs::write(
        dir.path().join("mandalo.toml"),
        "schema_version = 1\nid = \"curl-test\"\nname = \"Curl Test\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("collections/api/collection.toml"),
        "schema_version = 1\nid = \"api\"\nname = \"API\"\n",
    )
    .unwrap();
    dir
}

fn write_request(ws: &Path, path: &str, source: &str) {
    let target = ws.join("collections/api").join(path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, source).unwrap();
}

fn mandalo(ws: &Path) -> Command {
    let mut cmd = Command::cargo_bin("mandalo").unwrap();
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("NO_COLOR", "1")
        .arg("--workspace")
        .arg(ws);
    cmd
}

fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
}

#[test]
fn curl_prints_a_saved_request_with_its_variables_resolved() {
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/staging.toml"),
        "name = \"staging\"\n[vars]\nbase = \"https://staging.x.dev\"\n",
    )
    .unwrap();
    write_request(
        ws.path(),
        "auth/login.http",
        "### Login\nPOST {{base}}/login\nContent-Type: application/json\n\n{\"user\": \"ada\"}\n",
    );

    let out = mandalo(ws.path())
        .args(["curl", "api", "auth/login.http#0", "--env", "staging"])
        .assert()
        .success();
    let text = stdout_of(&out);
    assert!(
        text.contains("curl -X POST 'https://staging.x.dev/login'"),
        "{text}"
    );
    assert!(
        text.contains("-H 'Content-Type: application/json'"),
        "{text}"
    );
    assert!(text.contains("-d '{\"user\": \"ada\"}'"), "{text}");
}

#[test]
fn curl_refuses_a_request_whose_variables_are_not_set() {
    let ws = workspace();
    write_request(ws.path(), "login.http", "### Login\nGET {{base}}/login\n");

    let out = mandalo(ws.path())
        .args(["curl", "api", "login.http#0"])
        .assert()
        .failure()
        .code(1);
    let text = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(text.contains("E_UNRESOLVED_VAR"), "{text}");
}

#[test]
fn from_curl_reads_an_argument_and_prints_the_request() {
    let ws = workspace();
    let out = mandalo(ws.path())
        .args([
            "from-curl",
            "curl -X POST https://x.dev/users -H 'X-Trace: abc' -u ada:l0ve -d '{\"a\": 1}'",
        ])
        .assert()
        .success();
    let text = stdout_of(&out);
    assert!(text.contains("POST https://x.dev/users"), "{text}");
    assert!(text.contains("X-Trace: abc"), "{text}");
    assert!(text.contains("basic auth as ada"), "{text}");
    assert!(text.contains("{\"a\": 1}"), "{text}");
}

#[test]
fn from_curl_reads_stdin_and_emits_json() {
    let ws = workspace();
    let out = mandalo(ws.path())
        .args(["from-curl", "--reporter", "json"])
        .write_stdin("curl 'https://x.dev/a' \\\n  -H 'Accept: application/json'\n")
        .assert()
        .success();
    let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(parsed["kind"], "http");
    assert_eq!(parsed["method"], "GET");
    assert_eq!(parsed["url"], "https://x.dev/a");
    assert_eq!(parsed["headers"][0][0], "Accept");
    assert_eq!(parsed["headers"][0][1], "application/json");
}

#[test]
fn from_curl_refuses_a_flag_it_does_not_implement() {
    let ws = workspace();
    let out = mandalo(ws.path())
        .args(["from-curl", "curl --compressed https://x.dev"])
        .assert()
        .failure()
        .code(1);
    let text = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(text.contains("--compressed"), "{text}");
    assert!(text.contains("E_UNSUPPORTED"), "{text}");
}

/// What a user actually does: copy a command out of Mándalo, paste it back in.
#[test]
fn a_rendered_request_can_be_pasted_straight_back() {
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        "### Orders\nPOST https://x.dev/orders\nContent-Type: application/json\n\n{\"id\": 7}\n",
    );

    let rendered = stdout_of(
        &mandalo(ws.path())
            .args(["curl", "api", "orders.http#0"])
            .assert()
            .success(),
    );
    let out = mandalo(ws.path())
        .args(["from-curl", "--reporter", "json", rendered.trim()])
        .assert()
        .success();
    let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(parsed["method"], "POST");
    assert_eq!(parsed["url"], "https://x.dev/orders");
    assert_eq!(parsed["body"]["mode"], "raw");
    assert_eq!(parsed["body"]["language"], "json");
    assert_eq!(parsed["body"]["text"], "{\"id\": 7}");
}
