use assert_cmd::Command;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

fn serve(bodies: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for body in bodies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 8192];
            let mut received = Vec::new();
            loop {
                let Ok(n) = stream.read(&mut buf) else { return };
                received.extend_from_slice(&buf[..n]);
                if n == 0 || received.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("environments")).unwrap();
    std::fs::create_dir_all(dir.path().join("collections/api")).unwrap();
    std::fs::write(
        dir.path().join("mandalo.toml"),
        "schema_version = 1\nid = \"cli-test\"\nname = \"CLI Test\"\n",
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

fn status_test(code: u16) -> String {
    format!(
        "\n> {{%\npm.test(\"status is {code}\", function () {{ pm.response.to.have.status({code}); }});\n%}}\n"
    )
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

#[test]
fn help_and_version_work() {
    let mut help = Command::cargo_bin("mandalo").unwrap();
    let out = help.arg("--help").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for command in ["run", "send", "env", "ls", "import", "export", "scan"] {
        assert!(text.contains(command), "--help does not mention {command}");
    }

    let mut version = Command::cargo_bin("mandalo").unwrap();
    version
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains("mandalo"));
}

#[test]
fn ls_prints_the_collection_tree() {
    let ws = workspace();
    write_request(
        ws.path(),
        "login.http",
        "### Login\nPOST https://x.dev/login\n",
    );
    write_request(
        ws.path(),
        "admin/purge.http",
        "### Purge\nDELETE https://x.dev/purge\n",
    );

    let out = mandalo(ws.path()).arg("ls").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("api"), "{text}");
    assert!(text.contains("POST    Login"), "{text}");
    assert!(text.contains("admin/"), "{text}");
    assert!(text.contains("DELETE  Purge"), "{text}");
    assert!(text.contains("admin/purge.http#0"), "{text}");
}

#[test]
fn run_reports_a_passing_suite_and_exits_zero() {
    let url = serve(vec![r#"{"total":3}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        &format!("### Orders\nGET {url}/orders\n{}", status_test(200)),
    );

    mandalo(ws.path())
        .args(["run", "api"])
        .assert()
        .success()
        .stdout(predicates::str::contains("PASS  Orders"))
        .stdout(predicates::str::contains("all 1 requests passed"));
}

#[test]
fn run_exits_one_when_an_assertion_fails() {
    let url = serve(vec![r#"{"total":0}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        &format!("### Orders\nGET {url}/orders\n{}", status_test(404)),
    );

    mandalo(ws.path())
        .args(["run", "api"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("FAIL  Orders"))
        .stdout(predicates::str::contains("1 of 1 requests failed"));
}

#[test]
fn run_emits_junit_xml() {
    let url = serve(vec![r#"{"total":3}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        &format!("### Orders\nGET {url}/orders\n{}", status_test(200)),
    );

    let out = mandalo(ws.path())
        .args(["run", "api", "--reporter", "junit"])
        .assert()
        .success();
    let xml = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{xml}"
    );
    assert!(
        xml.contains("<testsuites name=\"mandalo\" tests=\"1\" failures=\"0\""),
        "{xml}"
    );
    assert!(
        xml.contains("<testcase classname=\"api.orders.http#0\""),
        "{xml}"
    );
    assert!(xml.contains("</testsuites>"), "{xml}");
}

#[test]
fn run_emits_json() {
    let url = serve(vec![r#"{"total":3}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        &format!("### Orders\nGET {url}/orders\n"),
    );

    let out = mandalo(ws.path())
        .args(["run", "api", "--reporter", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("json reporter emits valid JSON");
    assert_eq!(parsed["collection"], "api");
    assert_eq!(parsed["env"], serde_json::Value::Null);
    assert_eq!(parsed["total"], 1);
    assert_eq!(parsed["passed"], 1);
    assert_eq!(parsed["failed"], 0);

    let request = &parsed["requests"][0];
    assert_eq!(request["path"], "orders.http#0");
    assert_eq!(request["name"], "Orders");
    assert_eq!(request["method"], "GET");
    assert_eq!(request["url"], format!("{url}/orders"));
    assert_eq!(request["response"]["status"], 200);
    assert_eq!(request["passed"], true);
    assert_eq!(request["error"], serde_json::Value::Null);
}

fn json_stdout(out: &assert_cmd::assert::Assert) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.starts_with('{'),
        "--reporter json must put nothing but the document on stdout, got: {stdout}"
    );
    serde_json::from_str(&stdout).expect("--reporter json emits a valid JSON document")
}

#[test]
fn run_json_reports_a_failing_assertion_by_id_and_still_exits_one() {
    let url = serve(vec![r#"{"total":0}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "auth/login.http",
        &format!(
            r#"### Login
POST {url}/login

> {{%
pm.test("status is 200", function () {{ pm.response.to.have.status(200); }});
pm.test("total is above zero", function () {{ pm.expect(pm.response.json().total).to.be.above(0); }});
pm.environment.set("total", String(pm.response.json().total));
%}}
"#
        ),
    );

    let out = mandalo(ws.path())
        .args(["run", "api", "--reporter", "json"])
        .assert()
        .failure()
        .code(1);
    let parsed = json_stdout(&out);

    assert_eq!(parsed["total"], 1);
    assert_eq!(parsed["passed"], 0);
    assert_eq!(parsed["failed"], 1);

    let request = &parsed["requests"][0];
    assert_eq!(
        request["path"], "auth/login.http#0",
        "the path is collection-relative POSIX and addresses the block inside the file"
    );
    assert_eq!(request["passed"], false);

    let tests = request["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["id"], "script:0");
    assert_eq!(tests[0]["name"], "status is 200");
    assert_eq!(tests[0]["kind"], "script");
    assert_eq!(tests[0]["passed"], true);
    assert_eq!(tests[1]["id"], "script:1");
    assert_eq!(tests[1]["name"], "total is above zero");
    assert_eq!(tests[1]["passed"], false);
    assert!(tests[1]["detail"].as_str().unwrap().contains('0'));

    // A text request has no declarative captures, and the JSON reporter carries no
    // field for what a script wrote, so this list is empty by construction.
    assert_eq!(request["captures"].as_array().unwrap().len(), 0);
}

#[test]
fn run_json_reports_a_transport_failure_as_data() {
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        "### Orders\nGET http://127.0.0.1:1/orders\n",
    );

    let out = mandalo(ws.path())
        .args(["run", "api", "--reporter", "json"])
        .assert()
        .failure()
        .code(1);
    let request = &json_stdout(&out)["requests"][0];

    assert_eq!(request["response"], serde_json::Value::Null);
    assert_eq!(request["passed"], false);
    assert_eq!(request["errorCode"], "E_NETWORK");
    assert!(request["error"].as_str().unwrap().contains("127.0.0.1:1"));
    assert_eq!(request["tests"].as_array().unwrap().len(), 0);
}

#[test]
fn run_json_on_an_unknown_collection_writes_nothing_to_stdout() {
    let ws = workspace();
    let out = mandalo(ws.path())
        .args(["run", "nope", "--reporter", "json"])
        .assert()
        .failure()
        .code(1);
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout).trim(),
        "",
        "a hard failure must leave stdout empty so a parser fails loud on the exit code"
    );
    assert!(String::from_utf8_lossy(&out.get_output().stderr).contains("unknown collection: nope"));
}

#[test]
fn send_emits_json() {
    let url = serve(vec![r#"{"token":"abc123"}"#.to_string()]);
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/staging.toml"),
        "name = \"staging\"\n[vars]\n",
    )
    .unwrap();
    write_request(
        ws.path(),
        "auth/login.http",
        &format!(
            r#"### Login
POST {url}/login

> {{%
pm.test("status is 200", function () {{ pm.response.to.have.status(200); }});
pm.environment.set("token", pm.response.json().token);
%}}
"#
        ),
    );

    let out = mandalo(ws.path())
        .args([
            "send",
            "api",
            "auth/login.http",
            "--reporter",
            "json",
            "--env",
            "staging",
        ])
        .assert()
        .success();
    let parsed = json_stdout(&out);

    assert_eq!(parsed["collection"], "api");
    assert_eq!(parsed["env"], "staging");
    assert_eq!(parsed["path"], "auth/login.http");
    assert_eq!(parsed["name"], "Login");
    assert_eq!(parsed["method"], "POST");
    assert_eq!(parsed["url"], format!("{url}/login"));
    assert_eq!(parsed["response"]["status"], 200);
    assert_eq!(parsed["response"]["statusText"], "OK");
    assert_eq!(parsed["response"]["binary"], false);
    assert_eq!(parsed["tests"][0]["id"], "script:0");
    assert_eq!(parsed["tests"][0]["name"], "status is 200");
    assert_eq!(parsed["passed"], true);
    assert_eq!(parsed["error"], serde_json::Value::Null);
}

#[test]
fn send_json_reports_a_transport_failure_in_the_payload() {
    let ws = workspace();
    write_request(
        ws.path(),
        "dead.http",
        "### Dead\nGET http://127.0.0.1:1/dead\n",
    );

    let out = mandalo(ws.path())
        .args(["send", "api", "dead.http", "--reporter", "json"])
        .assert()
        .failure()
        .code(1);
    let parsed = json_stdout(&out);

    assert_eq!(parsed["collection"], "api");
    assert_eq!(parsed["env"], serde_json::Value::Null);
    assert_eq!(parsed["path"], "dead.http");
    assert_eq!(parsed["response"], serde_json::Value::Null);
    assert_eq!(parsed["errorCode"], "E_NETWORK");
    assert_eq!(parsed["passed"], false);
}

#[test]
fn send_json_reports_every_script_test_under_its_own_id() {
    let url = serve(vec![r#"{"total":3}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "orders.http",
        &format!(
            r#"### Orders
GET {url}/orders

> {{%
pm.test("total is three", function () {{ pm.expect(pm.response.json().total).to.eql(3); }});
pm.test("total is four", function () {{ pm.expect(pm.response.json().total).to.eql(4); }});
console.log("checked");
%}}
"#
        ),
    );

    let out = mandalo(ws.path())
        .args(["send", "api", "orders.http", "--reporter", "json"])
        .assert()
        .failure()
        .code(1);
    let parsed = json_stdout(&out);

    let tests = parsed["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2, "{tests:?}");
    assert_eq!(tests[0]["id"], "script:0");
    assert_eq!(tests[0]["kind"], "script");
    assert_eq!(tests[0]["name"], "total is three");
    assert_eq!(tests[0]["passed"], true);
    assert_eq!(tests[1]["id"], "script:1");
    assert_eq!(tests[1]["passed"], false);
    assert!(tests[1]["detail"].as_str().unwrap().contains("4"));
    assert_eq!(parsed["logs"][0], "checked");
    assert_eq!(parsed["passed"], false);
    assert_eq!(parsed["error"], serde_json::Value::Null);
}

#[test]
fn ls_emits_json() {
    let ws = workspace();
    write_request(ws.path(), "ping.http", "### Ping\nGET https://x.dev/ping\n");
    write_request(
        ws.path(),
        "auth/login.http",
        "### Login\nPOST https://x.dev/login\n",
    );

    let out = mandalo(ws.path())
        .args(["ls", "--reporter", "json"])
        .assert()
        .success();
    let parsed = json_stdout(&out);

    let collection = &parsed["collections"][0];
    assert_eq!(collection["id"], "api");
    assert_eq!(collection["slug"], "api");
    assert_eq!(collection["name"], "API");
    assert_eq!(collection["requests"][0]["name"], "Ping");
    assert_eq!(collection["requests"][0]["path"], "ping.http#0");
    assert_eq!(collection["requests"][0]["kind"], "http");
    assert_eq!(collection["requests"][0]["method"], "GET");

    let folder = &collection["folders"][0];
    assert_eq!(folder["name"], "auth");
    assert_eq!(folder["path"], "auth", "a folder path carries no extension");
    assert_eq!(folder["requests"][0]["path"], "auth/login.http#0");
    assert_eq!(parsed["skipped"].as_array().unwrap().len(), 0);
}

#[test]
fn ls_json_stays_a_bare_document_on_an_empty_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mandalo.toml"),
        "schema_version = 1\nid = \"empty\"\nname = \"Empty\"\n",
    )
    .unwrap();

    let out = mandalo(dir.path())
        .args(["ls", "--reporter", "json"])
        .assert()
        .success();
    let parsed = json_stdout(&out);
    assert_eq!(parsed["collections"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["skipped"].as_array().unwrap().len(), 0);
}

#[test]
fn env_list_emits_json() {
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/staging.toml"),
        "name = \"staging\"\n[vars]\nbase_url = \"https://staging.x.dev\"\n",
    )
    .unwrap();

    let out = mandalo(ws.path())
        .args(["env", "list", "--reporter", "json"])
        .assert()
        .success();
    let parsed = json_stdout(&out);

    assert_eq!(parsed["items"][0]["name"], "staging");
    assert_eq!(
        parsed["items"][0]["vars"]["base_url"],
        "https://staging.x.dev"
    );
    assert_eq!(parsed["skipped"].as_array().unwrap().len(), 0);
}

#[test]
fn env_list_json_stays_a_bare_document_when_there_are_no_environments() {
    let ws = workspace();
    let out = mandalo(ws.path())
        .args(["env", "list", "--reporter", "json"])
        .assert()
        .success();
    let parsed = json_stdout(&out);
    assert_eq!(parsed["items"].as_array().unwrap().len(), 0);
}

#[test]
fn pretty_stays_the_default_for_every_command() {
    let ws = workspace();
    write_request(ws.path(), "ping.http", "### Ping\nGET https://x.dev/ping\n");
    std::fs::write(
        ws.path().join("environments/staging.toml"),
        "name = \"staging\"\n[vars]\nbase_url = \"https://staging.x.dev\"\n",
    )
    .unwrap();

    for args in [vec!["ls"], vec!["env", "list"]] {
        let out = mandalo(ws.path()).args(&args).assert().success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        assert!(
            !stdout.trim_start().starts_with('{'),
            "{args:?} must still print human output by default, got: {stdout}"
        );
    }
}

#[test]
fn send_prints_the_response_and_redacts_resolved_secrets() {
    let url = serve(vec![r#"{"echo":"s3cr3t-token-value"}"#.to_string()]);
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/local.toml"),
        "name = \"local\"\n[vars]\n",
    )
    .unwrap();
    write_request(
        ws.path(),
        "whoami.http",
        &format!("### Who am I\nGET {url}/me\nAuthorization: Bearer {{{{token}}}}\n"),
    );

    let out = mandalo(ws.path())
        .env("MANDALO_SECRET__LOCAL__TOKEN", "s3cr3t-token-value")
        .args(["send", "api", "whoami.http", "--env", "local"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        !text.contains("s3cr3t-token-value"),
        "secret leaked: {text}"
    );
    assert!(text.contains("[redacted:local.token]"), "{text}");
    assert!(text.contains("200 OK"), "{text}");
}

#[test]
fn github_actions_masks_every_resolved_secret() {
    let url = serve(vec![r#"{"ok":true}"#.to_string()]);
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/local.toml"),
        "name = \"local\"\n[vars]\n",
    )
    .unwrap();
    write_request(
        ws.path(),
        "whoami.http",
        &format!("### Who am I\nGET {url}/me\nAuthorization: Bearer {{{{token}}}}\n"),
    );

    let out = mandalo(ws.path())
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_RUN_ID", "1")
        .env("GITHUB_WORKFLOW", "ci")
        .env("GITHUB_ACTION", "run")
        .env("CI", "true")
        .env("MANDALO_SECRET__LOCAL__TOKEN", "masked-value")
        .args(["send", "api", "whoami.http", "--env", "local"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("::add-mask::masked-value"), "{text}");
}

/// `::add-mask::` prints the secret on purpose. One exported variable anybody
/// can set must not be enough to turn that on.
#[test]
fn one_exported_variable_does_not_make_the_cli_print_a_secret() {
    let url = serve(vec![r#"{"ok":true}"#.to_string()]);
    let ws = workspace();
    std::fs::write(
        ws.path().join("environments/local.toml"),
        "name = \"local\"\n[vars]\n",
    )
    .unwrap();
    write_request(
        ws.path(),
        "whoami.http",
        &format!("### Who am I\nGET {url}/me\nAuthorization: Bearer {{{{token}}}}\n"),
    );

    let out = mandalo(ws.path())
        .env("GITHUB_ACTIONS", "true")
        .env_remove("GITHUB_RUN_ID")
        .env_remove("GITHUB_WORKFLOW")
        .env_remove("GITHUB_ACTION")
        .env_remove("CI")
        .env("MANDALO_SECRET__LOCAL__TOKEN", "masked-value")
        .args(["send", "api", "whoami.http", "--env", "local"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(!text.contains("masked-value"), "{text}");
}

#[test]
fn env_list_get_and_set_round_trip() {
    let ws = workspace();
    mandalo(ws.path())
        .args(["env", "set", "local", "base", "https://api.example.com"])
        .assert()
        .success()
        .stdout(predicates::str::contains("local.base saved"));

    mandalo(ws.path())
        .args(["env", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("local"))
        .stdout(predicates::str::contains("1 variables"));

    mandalo(ws.path())
        .args(["env", "get", "local"])
        .assert()
        .success()
        .stdout(predicates::str::contains("base=https://api.example.com"));
}

#[test]
fn env_set_refuses_a_credential_looking_value() {
    let ws = workspace();
    mandalo(ws.path())
        .args(["env", "set", "prod", "aws", "AKIAIOSFODNN7EXAMPLE"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("looks like a credential"))
        .stderr(predicates::str::contains("MANDALO_SECRET__PROD__AWS"));
    assert!(!ws.path().join("environments/prod.toml").exists());
}

#[test]
fn scan_exits_one_on_a_credential_and_zero_on_a_clean_workspace() {
    let ws = workspace();
    write_request(ws.path(), "clean.http", "### Clean\nGET {{base}}/ok\n");
    mandalo(ws.path())
        .arg("scan")
        .assert()
        .success()
        .stdout(predicates::str::contains("no credential-looking literals"));

    std::fs::write(
        ws.path().join("environments/prod.toml"),
        "name = \"prod\"\n[vars]\naws = \"AKIAIOSFODNN7EXAMPLE\"\n",
    )
    .unwrap();
    mandalo(ws.path())
        .arg("scan")
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("aws-access-key"))
        .stdout(predicates::str::contains("prod.toml"));
}

#[test]
fn strict_network_refuses_a_loopback_target() {
    let url = serve(vec![r#"{"ok":true}"#.to_string()]);
    let ws = workspace();
    write_request(
        ws.path(),
        "local.http",
        &format!("### Local\nGET {url}/x\n"),
    );

    mandalo(ws.path())
        .args(["run", "api", "--strict-network", "--reporter", "json"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("E_HOST_DENIED"));
}

#[test]
fn export_then_import_round_trips_a_workspace() {
    let ws = workspace();
    write_request(ws.path(), "ping.http", "### Ping\nGET https://x.dev/ping\n");
    let bundle = ws.path().join("bundle.json");

    mandalo(ws.path())
        .arg("export")
        .arg(&bundle)
        .arg("--yes")
        .assert()
        .success()
        .stdout(predicates::str::contains("exported 1 request"));

    let other = workspace();
    mandalo(other.path())
        .arg("import")
        .arg(&bundle)
        .assert()
        .success()
        .stdout(predicates::str::contains("1 requests"));

    mandalo(other.path())
        .arg("ls")
        .assert()
        .success()
        .stdout(predicates::str::contains("Ping"));
}

#[test]
fn an_unknown_collection_fails_with_its_error_code() {
    let ws = workspace();
    mandalo(ws.path())
        .args(["run", "ghost"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("unknown collection: ghost"))
        .stderr(predicates::str::contains("E_NOT_FOUND"));
}

/// An import that half-creates a workspace leaves a directory every later command
/// refuses to read, so it has to be refused before it writes anything.
#[test]
fn import_into_a_directory_that_is_not_a_workspace_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let spec = dir.path().join("openapi.json");
    std::fs::write(
        &spec,
        r#"{"openapi":"3.0.0","info":{"title":"T","version":"1"},"paths":{}}"#,
    )
    .unwrap();
    let target = tempfile::tempdir().unwrap();

    let assert = mandalo(target.path())
        .args(["import", spec.to_str().unwrap()])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("is not a Mándalo workspace"), "{stderr}");
    assert_eq!(
        std::fs::read_dir(target.path()).unwrap().count(),
        0,
        "the refused import left files behind"
    );
}

#[test]
fn ls_on_a_directory_that_is_not_a_workspace_fails_loud() {
    let dir = tempfile::tempdir().unwrap();
    let assert = mandalo(dir.path()).arg("ls").assert().failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("is not a Mándalo workspace"), "{stderr}");
    assert!(stderr.contains("mandalo init"), "{stderr}");
    assert!(
        String::from_utf8(assert.get_output().stdout.clone())
            .unwrap()
            .is_empty(),
        "an empty project and a typo must not look the same"
    );
}

#[test]
fn init_creates_a_workspace_a_relative_path_can_then_address() {
    let home = tempfile::tempdir().unwrap();
    let mut init = Command::cargo_bin("mandalo").unwrap();
    init.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("NO_COLOR", "1")
        .env("HOME", home.path())
        .current_dir(home.path())
        .args(["init", "ws-example", "--name", "Example"])
        .assert()
        .success();

    assert!(
        !home.path().join("Mandalo").exists(),
        "init must not create a default workspace on the side"
    );
    let created = home.path().join("ws-example");
    assert!(created.join("mandalo.toml").is_file());
    assert!(created.join("collections").is_dir());
    assert!(created.join("environments").is_dir());

    let mut ls = Command::cargo_bin("mandalo").unwrap();
    let assert = ls
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("NO_COLOR", "1")
        .env("HOME", home.path())
        .current_dir(home.path())
        .args(["--workspace", "./ws-example", "ls"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("no collections yet"), "{stdout}");
}

#[test]
fn scan_staged_outside_a_git_repository_says_so_in_one_line() {
    let ws = workspace();
    let assert = mandalo(ws.path())
        .args(["scan", "--staged"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("not inside a git repository"), "{stderr}");
    assert!(stderr.lines().count() <= 2, "{stderr}");
}

#[test]
fn the_send_help_addresses_a_request_the_way_the_product_does() {
    let ws = workspace();
    let assert = mandalo(ws.path())
        .args(["send", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("auth/login.http#0"), "{stdout}");
    assert!(!stdout.contains(".toml"), "{stdout}");
}
