use crate::collection::{self, SavedRequest};
use crate::postman::ImportReport;
use crate::workspace::{self, Environment};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    mandalo_bundle: u32,
    #[serde(default)]
    requests: Vec<SavedRequest>,
    #[serde(default)]
    environments: Vec<Environment>,
}

pub fn export(workspace: &Path) -> Result<String, String> {
    let requests = collection::list_requests(workspace)?;
    let environments = workspace::list_environments(workspace)?;
    let corrupt: Vec<String> = requests
        .skipped
        .into_iter()
        .chain(environments.skipped)
        .collect();
    if !corrupt.is_empty() {
        return Err(format!(
            "cannot export: unreadable workspace files: {}",
            corrupt.join("; ")
        ));
    }
    let bundle = Bundle {
        mandalo_bundle: 1,
        requests: requests.items,
        environments: environments.items,
    };
    serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())
}

pub fn import(workspace: &Path, json: &str) -> Result<ImportReport, String> {
    let bundle: Bundle =
        serde_json::from_str(json).map_err(|e| format!("invalid bundle JSON: {e}"))?;
    if bundle.mandalo_bundle != 1 {
        return Err(format!(
            "unsupported bundle version: {} (expected 1)",
            bundle.mandalo_bundle
        ));
    }
    for request in &bundle.requests {
        collection::save_request(workspace, request)?;
    }
    for env in &bundle.environments {
        workspace::save_environment(workspace, env)?;
    }
    Ok(ImportReport {
        imported: bundle.requests.len(),
        environments: bundle.environments.len(),
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: "Mándalo bundle imported; requests keep their original ids.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::GrpcRequest;
    use crate::request::Auth;
    use std::collections::BTreeMap;

    fn seed(workspace: &Path) -> (Vec<SavedRequest>, Vec<Environment>) {
        let http = SavedRequest {
            id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            name: "Create user".to_string(),
            kind: "http".to_string(),
            method: "POST".to_string(),
            url: "{{base}}/users".to_string(),
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: Some("{\"a\": 1}".to_string()),
            auth: Auth::Basic {
                username: "u".to_string(),
                password: "p".to_string(),
            },
            graphql: None,
            grpc: None,
        };
        let grpc = SavedRequest {
            id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            name: "Echo".to_string(),
            kind: "grpc".to_string(),
            method: "POST".to_string(),
            url: "http://localhost:50051".to_string(),
            headers: Vec::new(),
            body: None,
            auth: Auth::None,
            graphql: None,
            grpc: Some(GrpcRequest {
                proto_paths: vec!["/protos/echo.proto".to_string()],
                service: "test.v1.Echo".to_string(),
                method: "Say".to_string(),
                message: "{}".to_string(),
                metadata: Vec::new(),
            }),
        };
        let env = Environment {
            name: "staging".to_string(),
            vars: BTreeMap::from([("base".to_string(), "https://staging.x.dev".to_string())]),
        };
        collection::save_request(workspace, &http).unwrap();
        collection::save_request(workspace, &grpc).unwrap();
        workspace::save_environment(workspace, &env).unwrap();
        (vec![http, grpc], vec![env])
    }

    #[test]
    fn export_import_roundtrip_is_lossless() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        seed(a.path());
        let json = export(a.path()).unwrap();
        let report = import(b.path(), &json).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.environments, 1);
        assert!(report.skipped.is_empty());
        assert_eq!(
            collection::list_requests(b.path()).unwrap().items,
            collection::list_requests(a.path()).unwrap().items
        );
        assert_eq!(
            workspace::list_environments(b.path()).unwrap().items,
            workspace::list_environments(a.path()).unwrap().items
        );
    }

    #[test]
    fn export_fails_loud_on_corrupt_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        std::fs::write(dir.path().join("requests").join("bad.toml"), "broken [").unwrap();
        let err = export(dir.path()).unwrap_err();
        assert!(err.contains("cannot export"));
        assert!(err.contains("bad.toml"));
    }

    #[test]
    fn import_overwrites_same_id() {
        let dir = tempfile::tempdir().unwrap();
        let (requests, _) = seed(dir.path());
        let mut renamed = requests[0].clone();
        renamed.name = "Renamed".to_string();
        let json = serde_json::json!({
            "mandaloBundle": 1,
            "requests": [renamed.clone()],
            "environments": []
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let listed = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&renamed));
    }

    #[test]
    fn wrong_version_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let err = import(dir.path(), "{\"mandaloBundle\": 2}").unwrap_err();
        assert!(err.contains("unsupported bundle version: 2"));
    }
}
