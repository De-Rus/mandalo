use assert_cmd::Command;
use std::path::{Path, PathBuf};

const SECRET: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

struct Fixture {
    dir: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Fixture {
    fn start() -> Fixture {
        let fixture = Fixture {
            dir: tempfile::tempdir().expect("tempdir"),
            home: tempfile::tempdir().expect("tempdir"),
        };
        let path = fixture.dir.path().to_path_buf();
        fixture
            .cmd()
            .arg("init")
            .arg(&path)
            .args(["--name", "Test"])
            .assert()
            .success();
        fixture
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Deliberately outside the workspace: that is the whole point.
    fn secrets_file(&self) -> PathBuf {
        self.home
            .path()
            .join(".config")
            .join("mandalo")
            .join("secrets.toml")
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("mandalo").expect("the mandalo binary is built");
        cmd.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("NO_COLOR", "1")
            .env("HOME", self.home.path())
            .env("MANDALO_SECRETS_FILE", self.secrets_file())
            .arg("--workspace")
            .arg(self.dir.path());
        cmd
    }

    fn env_file(&self, name: &str) -> String {
        std::fs::read_to_string(
            self.dir
                .path()
                .join("environments")
                .join(format!("{name}.toml")),
        )
        .unwrap()
    }
}

#[test]
fn set_secret_reads_the_value_from_stdin_and_keeps_it_out_of_the_committed_file() {
    let f = Fixture::start();

    f.cmd()
        .args(["env", "set-secret", "prod", "token"])
        .write_stdin(format!("{SECRET}\n"))
        .assert()
        .success();

    let committed = f.env_file("prod");
    assert!(committed.contains("secret = true"), "{committed}");
    assert!(!committed.contains(SECRET), "{committed}");
    assert!(!committed.contains("value"), "{committed}");

    let held = std::fs::read_to_string(f.secrets_file()).unwrap();
    assert!(held.contains(SECRET), "{held}");
    assert!(
        !f.secrets_file().starts_with(f.path()),
        "the values file lives outside the workspace entirely"
    );
}

/// A value passed as an argument is a value every process on the box can read
/// out of `ps`. The command must not offer the option at all.
#[test]
fn set_secret_has_no_way_to_pass_the_value_on_the_command_line() {
    let f = Fixture::start();
    let help = f
        .cmd()
        .args(["env", "set-secret", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("<NAME> <KEY>"), "{text}");
    assert!(!text.to_lowercase().contains("<value>"), "{text}");

    f.cmd()
        .args(["env", "set-secret", "prod", "token", SECRET])
        .assert()
        .failure();
}

#[test]
fn an_empty_stdin_is_refused_rather_than_stored() {
    let f = Fixture::start();
    let out = f
        .cmd()
        .args(["env", "set-secret", "prod", "token"])
        .write_stdin("   \n")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no value arrived on stdin"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!f.secrets_file().exists());
}

#[test]
fn env_get_names_the_layer_every_value_comes_from() {
    let f = Fixture::start();
    f.cmd()
        .args(["env", "set", "prod", "baseUrl", "https://team.example"])
        .assert()
        .success();
    f.cmd()
        .args(["env", "set-secret", "prod", "token"])
        .write_stdin(SECRET)
        .assert()
        .success();

    let out = f.cmd().args(["env", "get", "prod"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("baseUrl=https://team.example"), "{text}");
    assert!(
        text.contains("secret · not bound to a host · from this machine"),
        "{text}"
    );
    assert!(!text.contains(SECRET), "{text}");

    let out = f
        .cmd()
        .env("MANDALO_SECRET__PROD__BASEURL", "https://ci.example")
        .args(["env", "get", "prod"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("overridden from the environment variable"),
        "{text}"
    );
}

#[test]
fn set_local_stores_a_value_without_masking_it() {
    let f = Fixture::start();
    f.cmd()
        .args(["env", "set-local", "prod", "devUrl"])
        .write_stdin("http://localhost:3000")
        .assert()
        .success();

    let committed = f.env_file("prod");
    assert!(committed.contains("shared = false"), "{committed}");
    assert!(!committed.contains("localhost:3000"), "{committed}");

    let out = f.cmd().args(["env", "get", "prod"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("local · not bound to a host · from this machine"),
        "{text}"
    );
}

#[test]
fn clear_secret_forgets_the_value_and_keeps_the_declaration() {
    let f = Fixture::start();
    f.cmd()
        .args(["env", "set-secret", "prod", "token"])
        .write_stdin(SECRET)
        .assert()
        .success();

    f.cmd()
        .args(["env", "clear-secret", "prod", "token"])
        .assert()
        .success();

    assert!(!std::fs::read_to_string(f.secrets_file())
        .unwrap()
        .contains(SECRET));
    assert!(f.env_file("prod").contains("secret = true"));

    let out = f.cmd().args(["env", "get", "prod"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("no value on this machine"));
}

#[cfg(unix)]
#[test]
fn the_values_file_is_created_private_to_this_user() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::start();
    f.cmd()
        .args(["env", "set-secret", "prod", "token"])
        .write_stdin(SECRET)
        .assert()
        .success();
    let mode = std::fs::metadata(f.secrets_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}
