use assert_cmd::Command;
use mandalo_testkit::fixtures::install_example_workspace;
use mandalo_testkit::{MockApi, TOKEN};
use std::path::Path;

struct Fixture {
    _api: MockApi,
    dir: tempfile::TempDir,
}

impl Fixture {
    async fn start() -> Fixture {
        let api = MockApi::start().await;
        let dir = tempfile::tempdir().expect("tempdir");
        install_example_workspace(
            dir.path(),
            &api.base_url(),
            &api.grpc_url(),
            &api.proto_path(),
        );
        Fixture { _api: api, dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn cmd(&self) -> Command {
        mandalo(self.dir.path())
    }
}

fn mandalo(ws: &Path) -> Command {
    let mut cmd = Command::cargo_bin("mandalo").expect("the mandalo binary is built");
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("NO_COLOR", "1")
        .arg("--workspace")
        .arg(ws);
    cmd
}

fn stdout(output: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&output.get_output().stdout).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shipped_example_workspace_lists_every_request_file() {
    let fixture = Fixture::start().await;

    let out = fixture.cmd().arg("ls").assert().success();
    let text = stdout(&out);

    for file in [
        "http/echo.http#",
        "auth/methods.http#",
        "graphql/queries.http#",
        "uploads/bodies.http#",
        "chaining/login-flow.http#",
        "grpc/service.grpc#",
    ] {
        assert!(text.contains(file), "{file} missing from:\n{text}");
    }
    assert!(text.contains("mock"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_exits_zero_when_every_request_passes() {
    let fixture = Fixture::start().await;

    let out = fixture
        .cmd()
        .args(["run", "mock", "--env", "local"])
        .assert()
        .success();

    let text = stdout(&out);
    assert!(text.contains("requests passed"), "{text}");
    assert!(!text.contains("FAIL"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_exits_one_when_a_request_fails() {
    let fixture = Fixture::start().await;
    std::fs::write(
        fixture.path().join("collections/mock/broken.http"),
        "### Broken\nGET {{baseUrl}}/status/500\n\n> {%\npm.test(\"status is 200\", function () { pm.response.to.have.status(200); });\n%}\n",
    )
    .expect("write request");

    fixture
        .cmd()
        .args(["run", "mock", "--env", "local"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("FAIL  Broken"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_json_reporter_carries_the_real_results() {
    let fixture = Fixture::start().await;

    let out = fixture
        .cmd()
        .args(["run", "mock", "--env", "local", "--reporter", "json"])
        .assert()
        .success();

    let report: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("the json reporter emits JSON");
    assert_eq!(report["collection"], "mock");
    assert_eq!(report["env"], "local");
    assert_eq!(report["failed"], 0);
    assert_eq!(report["total"], report["passed"]);

    let statuses: Vec<u64> = report["requests"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|s| {
            s["path"]
                .as_str()
                .unwrap_or_default()
                .starts_with("auth/methods.http#")
        })
        .map(|s| s["response"]["status"].as_u64().expect("a status"))
        .collect();
    assert_eq!(statuses, vec![200, 200, 200]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_junit_reporter_carries_the_real_results() {
    let fixture = Fixture::start().await;

    let out = fixture
        .cmd()
        .args(["run", "mock", "--env", "local", "--reporter", "junit"])
        .assert()
        .success();

    let xml = stdout(&out);
    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{xml}"
    );
    assert!(xml.contains("failures=\"0\""), "{xml}");
    assert!(
        xml.contains("<testcase classname=\"mock.graphql/queries.http#0\""),
        "{xml}"
    );
    assert!(xml.contains("</testsuites>"), "{xml}");
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_fast_stops_at_the_first_failure() {
    let fixture = Fixture::start().await;
    let folder = fixture.path().join("collections/mock/fastfail");
    std::fs::create_dir_all(&folder).expect("folder");
    for (name, path, url) in [
        ("1 Broken", "1-broken.http", "{{baseUrl}}/status/500"),
        ("2 Fine", "2-fine.http", "{{baseUrl}}/get"),
    ] {
        std::fs::write(
            folder.join(path),
            format!(
                "### {name}\nGET {url}\n\n> {{%\npm.test(\"status is 200\", function () {{ pm.response.to.have.status(200); }});\n%}}\n"
            ),
        )
        .expect("write request");
    }

    let out = fixture
        .cmd()
        .args([
            "run",
            "mock",
            "--folder",
            "fastfail",
            "--env",
            "local",
            "--fail-fast",
            "--reporter",
            "json",
        ])
        .assert()
        .failure()
        .code(1);

    let report: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("the json reporter emits JSON");
    assert_eq!(report["total"], 1);
    assert_eq!(report["requests"][0]["name"], "1 Broken");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_environment_fails_loud() {
    let fixture = Fixture::start().await;

    fixture
        .cmd()
        .args(["run", "mock", "--env", "ghost"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("unknown environment: ghost"))
        .stderr(predicates::str::contains("E_NOT_FOUND"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_prints_one_response() {
    let fixture = Fixture::start().await;

    let out = fixture
        .cmd()
        .args(["send", "mock", "http/echo.http#0", "--env", "local"])
        .assert()
        .success();

    let text = stdout(&out);
    assert!(text.contains("200 OK"), "{text}");
    assert!(
        text.contains("\"method\": \"GET\"") || text.contains("\"method\":\"GET\""),
        "{text}"
    );
    assert!(text.contains("PASS"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_runs_the_login_chain_and_captures_the_token() {
    let fixture = Fixture::start().await;

    let out = fixture
        .cmd()
        .args(["run", "mock", "--env", "local"])
        .assert()
        .success();

    let text = stdout(&out);
    assert!(text.contains("PASS  1 Login"), "{text}");
    assert!(text.contains("PASS  2 Authenticated call"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_echoed_back_by_the_server_never_reaches_stdout() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    install_example_workspace(
        dir.path(),
        &api.base_url(),
        &api.grpc_url(),
        &api.proto_path(),
    );
    std::fs::write(
        dir.path().join("collections/mock/echo-secret.http"),
        "### Echo secret\nGET {{baseUrl}}/headers/echo\nAuthorization: Bearer {{apiToken}}\n",
    )
    .expect("write request");

    let out = mandalo(dir.path())
        .env("MANDALO_SECRET__LOCAL__APITOKEN", TOKEN)
        .args(["send", "mock", "echo-secret.http", "--env", "local"])
        .assert()
        .success();

    let text = stdout(&out);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(text.contains("[redacted:local.apiToken]"), "{text}");
    assert!(
        !text.contains(TOKEN),
        "the secret leaked to stdout:\n{text}"
    );
    assert!(
        !stderr.contains(TOKEN),
        "the secret leaked to stderr:\n{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn strict_network_refuses_the_loopback_mock() {
    let fixture = Fixture::start().await;

    fixture
        .cmd()
        .args([
            "run",
            "mock",
            "--env",
            "local",
            "--strict-network",
            "--reporter",
            "json",
        ])
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("E_HOST_DENIED"));
}
