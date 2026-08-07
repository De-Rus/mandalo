use assert_cmd::Command;
use std::path::Path;

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::create_dir_all(ws.join("environments")).unwrap();
    std::fs::create_dir_all(ws.join("collections/api/users")).unwrap();
    std::fs::write(
        ws.join("mandalo.toml"),
        "schema_version = 1\nid = \"review-test\"\nname = \"Review Test\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("collections/api/collection.toml"),
        "schema_version = 1\nid = \"api\"\nname = \"API\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("collections/api/ping.http"),
        "### Ping\nGET https://example.test/ping\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("collections/api/users/list.http"),
        "### List users\nGET https://example.test/users\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("environments/prod.toml"),
        "schema_version = 1\nname = \"prod\"\n\n[vars]\nbase = { value = \"https://example.test\" }\ntoken = { secret = true, hosts = [\"example.test\"] }\n",
    )
    .unwrap();
    dir
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

fn leak(ws: &Path) {
    std::fs::write(
        ws.join("collections/api/leak.http"),
        format!(
            "### Leak\nGET https://example.test/x\nAuthorization: Bearer gh{}_16C7e42F292c6912E7710c838347Ae178B4a\n",
            "p"
        ),
    )
    .unwrap();
}

#[test]
fn export_without_a_terminal_and_without_yes_refuses_to_assume() {
    let dir = workspace();
    let out = dir.path().join("bundle.json");
    let assert = mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(!out.exists(), "nothing may be written without an answer");
}

#[test]
fn export_prints_the_plan_before_it_writes() {
    let dir = workspace();
    let out = dir.path().join("bundle.json");
    let assert = mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .arg("--yes")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("2 requests"), "{stdout}");
    assert!(stdout.contains("1 environment"), "{stdout}");
    assert!(
        stdout.contains("1 secret value are not included") || stdout.contains("1 secret value"),
        "{stdout}"
    );
    assert!(out.exists());
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("\"mandaloBundle\""));
}

#[test]
fn export_narrows_to_a_folder_and_an_environment() {
    let dir = workspace();
    let out = dir.path().join("users.json");
    mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .args(["--collection", "api", "--folder", "users", "--env", "prod"])
        .arg("--yes")
        .assert()
        .success();
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("List users"));
    assert!(!written.contains("\"Ping\""), "{written}");
}

#[test]
fn export_refuses_to_mix_all_with_a_filter() {
    let dir = workspace();
    mandalo(dir.path())
        .arg("export")
        .arg(dir.path().join("x.json"))
        .args(["--all", "--collection", "api", "--yes"])
        .assert()
        .failure();
}

#[test]
fn a_finding_stops_the_export_and_leaves_no_file() {
    let dir = workspace();
    leak(dir.path());
    let out = dir.path().join("bundle.json");
    let assert = mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .arg("--yes")
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("credential"), "{stdout}");
    assert!(stdout.contains("--force"), "{stdout}");
    assert!(
        !out.exists(),
        "the file must not exist when the scanner blocked the export"
    );

    mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .args(["--yes", "--force"])
        .assert()
        .success();
    assert!(out.exists(), "--force is the one way through");
}

#[test]
fn a_finding_outside_the_selection_does_not_block() {
    let dir = workspace();
    leak(dir.path());
    let out = dir.path().join("users.json");
    mandalo(dir.path())
        .arg("export")
        .arg(&out)
        .args(["--collection", "api", "--folder", "users", "--yes"])
        .assert()
        .success();
    assert!(out.exists());
}

#[test]
fn sync_without_a_terminal_and_without_yes_refuses_to_assume() {
    let dir = workspace();
    mandalo(dir.path()).args(["git-hygiene"]).assert().success();
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .assert()
        .success();

    let assert = mandalo(dir.path()).arg("sync").assert().failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(
        !dir.path().join(".git/refs/heads/main").exists(),
        "nothing may be committed without an answer"
    );
}

#[test]
fn sync_prints_the_files_and_the_remote_before_committing() {
    let dir = workspace();
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .assert()
        .success();
    for (key, value) in [("user.name", "Tester"), ("user.email", "t@example.test")] {
        Command::new("git")
            .args(["config", key, value])
            .current_dir(dir.path())
            .assert()
            .success();
    }
    let assert = mandalo(dir.path())
        .args(["sync", "-m", "first", "--yes"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("branch main"), "{stdout}");
    assert!(stdout.contains("no remote"), "{stdout}");
    assert!(stdout.contains("collections/api/ping.http"), "{stdout}");
    assert!(stdout.contains("committing as Tester"), "{stdout}");
}

#[test]
fn sync_leaves_an_excepted_file_out_and_says_so() {
    let dir = workspace();
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .assert()
        .success();
    for (key, value) in [("user.name", "Tester"), ("user.email", "t@example.test")] {
        Command::new("git")
            .args(["config", key, value])
            .current_dir(dir.path())
            .assert()
            .success();
    }
    let assert = mandalo(dir.path())
        .args([
            "sync",
            "-m",
            "most of it",
            "--except",
            "collections/api/users",
            "--yes",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(stdout.contains("left out"), "{stdout}");

    let tracked = Command::new("git")
        .args(["ls-files"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let files = String::from_utf8_lossy(&tracked.stdout).to_string();
    assert!(files.contains("collections/api/ping.http"), "{files}");
    assert!(!files.contains("users/list.http"), "{files}");
    assert!(
        dir.path().join("collections/api/users/list.http").exists(),
        "the file left out stays in the working tree"
    );
}
