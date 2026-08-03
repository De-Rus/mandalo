use assert_cmd::Command;
use mandalo_core::github_auth::CLIENT_ID_PLACEHOLDER;

fn mandalo() -> Command {
    Command::cargo_bin("mandalo").unwrap()
}

#[test]
fn the_github_commands_are_listed() {
    let out = mandalo().arg("--help").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("login"));
    assert!(text.contains("logout"));
    assert!(text.contains("whoami"));
}

#[test]
fn a_token_is_read_from_stdin_and_never_from_an_argument() {
    let text = String::from_utf8_lossy(
        &mandalo()
            .arg("login")
            .arg("--help")
            .output()
            .unwrap()
            .stdout,
    )
    .to_string();
    assert!(text.contains("--with-token"));
    assert!(!text.contains("<TOKEN>"));
}

#[test]
fn an_empty_stdin_fails_before_anything_is_stored() {
    let out = mandalo()
        .args(["login", "--with-token"])
        .write_stdin("")
        .assert()
        .failure();
    let text = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(text.contains("stdin"), "{text}");
    assert!(text.contains("E_SECRET"), "{text}");
}

#[test]
fn a_build_without_an_oauth_app_refuses_to_start_a_device_flow() {
    let out = mandalo()
        .args([
            "login",
            "--client-id",
            CLIENT_ID_PLACEHOLDER,
            "--no-browser",
        ])
        .assert()
        .failure();
    let text = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(text.contains("E_UNSUPPORTED"), "{text}");
    assert!(text.contains("MANDALO_GITHUB_CLIENT_ID"), "{text}");
}
