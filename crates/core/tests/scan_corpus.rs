//! The scanner blocks an export and a sync, so a false positive is not noise —
//! it is a workspace nobody can commit, and a habit of reaching for `--force`.
//! These two tests are the contract: nothing on a clean corpus, everything on a
//! credential.

use mandalo_core::scan;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn clean(dir: &Path) {
    let findings = scan::scan_workspace(dir).expect("scan");
    assert!(
        findings.is_empty(),
        "{} is known clean, so every one of these is a false positive:\n{findings:#?}",
        dir.display()
    );
}

#[test]
fn the_example_workspace_produces_nothing() {
    clean(&repo_root().join("examples/mock-workspace"));
}

#[test]
fn the_documentation_produces_nothing() {
    clean(&repo_root().join("docs"));
}

#[test]
fn a_workspace_imported_from_the_postman_fixtures_produces_nothing() {
    let dir = tempfile::tempdir().unwrap();
    mandalo_testkit::fixtures::workspace(dir.path());
    let fixtures = repo_root().join("crates/core/tests/fixtures/postman");
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let path = entry.unwrap().path();
        let json = std::fs::read_to_string(&path).unwrap();
        mandalo_core::postman::import(dir.path(), &json)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    clean(dir.path());
}

#[test]
fn the_shapes_a_workspace_is_made_of_are_never_findings() {
    let ordinary = [
        "orders/list-orders.http#0",
        "collections/acme-api/users/create-user.http",
        "path = \"orders/list-orders.http#0\"",
        "url = \"{{base}}/v1/organisations/{{org}}/members?limit=100\"",
        "### List every order placed in the last thirty days",
        "# see https://docs.acme.example/api/reference/orders#pagination",
        "id = \"550e8400-e29b-41d4-a716-446655440000\"",
        "description = \"returns the most recent orders for the signed in user\"",
        "content-type: application/vnd.acme.orders.v2+json",
        "user-agent: mandalo/0.1.0 (macOS; arm64)",
        "@base = https://api.staging.acme-internal.example.com",
        "x-request-id: {{$guid}}",
        "accept-language: en-GB,en;q=0.9,es-ES;q=0.8",
        "proto: protos/order-service/v1/orders.proto",
        "file = \"files/very-long-attachment-name-here.pdf\"",
        "authorization: Bearer {{token}}",
        "password = \"{{account_password}}\"",
        "api_key = \"$ACME_API_KEY\"",
        "secret = \"changeme-before-you-deploy\"",
        "token = \"your-token-goes-here\"",
        "2024-11-05T14:32:07.123456789+01:00",
        "sha = \"see the lockfile\"",
    ];
    for line in ordinary {
        let found = scan::scan_text(Path::new("collections/api/list.http"), line);
        assert!(found.is_empty(), "false positive on {line:?}: {found:#?}");
    }
}

#[test]
fn every_credential_shape_the_review_demonstrated_is_caught() {
    let aws = format!("AKIA{}", "IOSFODNN7EXAMPLE");
    let base64_aws = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(aws.as_bytes())
    };
    let homoglyph = format!("AKI\u{0410}{}", "IOSFODNN7EXAMPLE");

    let credentials = vec![
        format!("token = \"{aws}\""),
        format!("blob = \"{base64_aws}\""),
        format!("aws = \"{homoglyph}\""),
        format!("token = \"AKIA\" \\\n  \"{}\"", "IOSFODNN7EXAMPLE"),
        format!("token = \"AKIA\" +\n  \"{}\"", "IOSFODNN7EXAMPLE"),
        "gitlab_token = 4f2a1b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a".to_string(),
        "totp = \"MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43UOJUW4ZY\"".to_string(),
        "url = \"https://admin:hunter2hunter2@internal.acme.io/\"".to_string(),
        "auth = \"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\"".to_string(),
        format!("stripe = \"sk_{}_4eC39HqLyjWDarjtT1zdp7dc\"", "live"),
        format!("gh = \"gh{}_16C7e42F292c6912E7710c838347Ae178B4a\"", "p"),
        "secret = \"Xq7Lm2Pv9Zt4Rn8Wb3Kd6Yh1\"".to_string(),
        "-----BEGIN RSA PRIVATE KEY-----".to_string(),
        "api_key = \"7f3a2b1c.9d8e7f6a5b4c3d2e1f0a9b8c\"".to_string(),
    ];
    for line in &credentials {
        let found = scan::scan_text(Path::new("environments/prod.toml"), line);
        assert!(!found.is_empty(), "missed the credential in {line:?}");
    }
}
