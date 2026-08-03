use crate::collection::{self, CollectionNode, FolderNode, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::postman::ImportReport;
use crate::scan::{self, Finding};
use crate::workspace::{self, EnvDoc, RawEnvDoc};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const BUNDLE_VERSION: u32 = 2;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    mandalo_bundle: u32,
    collections: Vec<BundleCollection>,
    requests: Vec<SavedRequest>,
    environments: Vec<EnvDoc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingBundle {
    mandalo_bundle: u32,
    #[serde(default)]
    collections: Vec<BundleCollection>,
    #[serde(default)]
    requests: Vec<SavedRequest>,
    #[serde(default)]
    environments: Vec<RawEnvDoc>,
}

/// The payload plus what the scanner found in it. A caller shows the findings
/// before the bundle leaves the machine; secret **values** are never in `json`
/// because the environment files never held them.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportBundle {
    pub json: String,
    pub findings: Vec<Finding>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleCollection {
    id: String,
    slug: String,
    name: String,
    #[serde(default)]
    requests: Vec<BundleRequest>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleRequest {
    path: String,
    request: SavedRequest,
}

fn flatten(folders: &[FolderNode], out: &mut Vec<String>) {
    for folder in folders {
        for request in &folder.requests {
            out.push(request.path.clone());
        }
        flatten(&folder.folders, out);
    }
}

fn paths(node: &CollectionNode) -> Vec<String> {
    let mut out: Vec<String> = node.requests.iter().map(|r| r.path.clone()).collect();
    flatten(&node.folders, &mut out);
    out.sort();
    out
}

pub fn export(workspace: &Path) -> CoreResult<ExportBundle> {
    let tree = collection::list_tree(workspace)?;
    let environments = workspace::list_env_docs(workspace)?;
    let corrupt: Vec<String> = tree
        .skipped
        .into_iter()
        .chain(environments.skipped)
        .collect();
    if !corrupt.is_empty() {
        return Err(CoreError::Io(format!(
            "cannot export: unreadable workspace files: {}",
            corrupt.join("; ")
        )));
    }
    let mut collections = Vec::with_capacity(tree.collections.len());
    for node in &tree.collections {
        let mut requests = Vec::new();
        for path in paths(node) {
            // The source view, not the resolved one: a bundle travels to another
            // machine, where a baked-in absolute proto path resolves to nothing.
            let request = collection::load_request_source(workspace, &node.slug, &path)?;
            requests.push(BundleRequest { path, request });
        }
        collections.push(BundleCollection {
            id: node.id.clone(),
            slug: node.slug.clone(),
            name: node.name.clone(),
            requests,
        });
    }
    let bundle = Bundle {
        mandalo_bundle: BUNDLE_VERSION,
        collections,
        requests: Vec::new(),
        environments: environments.items,
    };
    let json =
        serde_json::to_string_pretty(&bundle).map_err(|e| CoreError::Parse(e.to_string()))?;
    let findings = scan::scan_text(Path::new("<bundle>"), &json);
    Ok(ExportBundle { json, findings })
}

pub fn import(workspace: &Path, json: &str) -> CoreResult<ImportReport> {
    let bundle: IncomingBundle = serde_json::from_str(json)
        .map_err(|e| CoreError::Parse(format!("invalid bundle JSON: {e}")))?;
    let mut environments = Vec::with_capacity(bundle.environments.len());
    for raw in bundle.environments {
        environments.push(raw.validate("bundle environment")?);
    }
    let mut report = ImportReport {
        imported: 0,
        collections: 0,
        environments: 0,
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: String::new(),
    };
    match bundle.mandalo_bundle {
        1 => {
            let target = collection::ensure_collection(workspace, "default", "Default", None)?;
            report.collections = 1;
            for request in &bundle.requests {
                collection::save_request(workspace, &target.slug, None, None, request)?;
                report.imported += 1;
            }
            report.summary =
                "Version 1 bundle imported into the Default collection and upgraded to version 2."
                    .to_string();
        }
        2 => {
            for c in &bundle.collections {
                let target =
                    collection::ensure_collection(workspace, &c.slug, &c.name, Some(&c.id))?;
                report.collections += 1;
                for entry in &c.requests {
                    collection::put_request(workspace, &target.slug, &entry.path, &entry.request)?;
                    report.imported += 1;
                }
            }
            report.summary =
                "Mándalo bundle imported; requests keep their original ids and folders."
                    .to_string();
        }
        other => {
            return Err(CoreError::Unsupported(format!(
                "unsupported bundle version: {other} (expected 1 or {BUNDLE_VERSION})"
            )))
        }
    }
    for env in &environments {
        workspace::save_env_doc(workspace, env)?;
    }
    report.environments = environments.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::Scripts;
    use crate::body::Body;
    use crate::collection::GrpcRequest;
    use crate::request::Auth;
    use crate::workspace::Environment;
    use std::collections::BTreeMap;

    fn request(name: &str) -> SavedRequest {
        SavedRequest {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: "http".to_string(),
            method: "POST".to_string(),
            url: "{{base}}/users".to_string(),
            description: None,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            auth: Auth::Basic {
                username: "u".to_string(),
                password: "p".to_string(),
            },
            body: Body::json("{\"a\": 1}"),
            grpc: None,
            scripts: Scripts {
                pre: Some("setup()".to_string()),
                post: Some("pm.environment.set(\"userId\", pm.response.json().id);".to_string()),
            },
            tests: Vec::new(),
            captures: Vec::new(),
        }
    }

    fn workspace_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(collection::collections_dir(dir.path())).unwrap();
        std::fs::create_dir_all(dir.path().join("protos")).unwrap();
        std::fs::write(dir.path().join("protos/echo.proto"), "syntax = \"proto3\";").unwrap();
        dir
    }

    fn seed(workspace: &Path) {
        let acme = collection::create_collection(workspace, "Acme API").unwrap();
        collection::create_folder(workspace, &acme.slug, "users").unwrap();
        collection::create_folder(workspace, &acme.slug, "users/admin").unwrap();
        collection::save_request(workspace, &acme.slug, None, None, &request("Root")).unwrap();
        collection::save_request(workspace, &acme.slug, None, Some("users"), &request("List"))
            .unwrap();
        collection::save_request(
            workspace,
            &acme.slug,
            None,
            Some("users/admin"),
            &request("Purge"),
        )
        .unwrap();

        let other = collection::create_collection(workspace, "Beta").unwrap();
        let mut grpc = request("Echo");
        grpc.kind = "grpc".to_string();
        grpc.body = Body::None;
        grpc.auth = Auth::None;
        grpc.headers = Vec::new();
        grpc.grpc = Some(GrpcRequest {
            proto_paths: vec!["protos/echo.proto".to_string()],
            service: "test.v1.Echo".to_string(),
            method: "Say".to_string(),
            message: "{}".to_string(),
            metadata: Vec::new(),
        });
        collection::save_request(workspace, &other.slug, None, None, &grpc).unwrap();

        workspace::save_environment(
            workspace,
            &Environment {
                name: "staging".to_string(),
                vars: BTreeMap::from([("base".to_string(), "https://staging.x.dev".to_string())]),
            },
        )
        .unwrap();
    }

    #[test]
    fn export_import_roundtrip_is_lossless() {
        let a = workspace_dir();
        let b = workspace_dir();
        seed(a.path());
        let json = export(a.path()).unwrap().json;
        let report = import(b.path(), &json).unwrap();
        assert_eq!(report.imported, 4);
        assert_eq!(report.collections, 2);
        assert_eq!(report.environments, 1);
        assert!(report.skipped.is_empty());
        assert_eq!(
            collection::list_tree(b.path()).unwrap(),
            collection::list_tree(a.path()).unwrap()
        );
        assert_eq!(
            workspace::list_environments(b.path()).unwrap().items,
            workspace::list_environments(a.path()).unwrap().items
        );
        assert_eq!(
            collection::load_request(b.path(), "acme-api", "users/admin/purge.http#0").unwrap(),
            collection::load_request(a.path(), "acme-api", "users/admin/purge.http#0").unwrap()
        );
        assert_eq!(
            collection::load_request_source(b.path(), "beta", "echo.grpc#0")
                .unwrap()
                .grpc
                .unwrap()
                .proto_paths,
            vec!["protos/echo.proto".to_string()],
            "a bundle carries the workspace-relative proto path, not this machine's"
        );
    }

    #[test]
    fn export_declares_version_2_with_folder_paths() {
        let dir = workspace_dir();
        seed(dir.path());
        let json: serde_json::Value =
            serde_json::from_str(&export(dir.path()).unwrap().json).unwrap();
        assert_eq!(json["mandaloBundle"], 2);
        assert_eq!(json["collections"][0]["slug"], "acme-api");
        let paths: Vec<&str> = json["collections"][0]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec![
                "root.http#0",
                "users/admin/purge.http#0",
                "users/list.http#0"
            ]
        );
        assert_eq!(
            json["collections"][0]["requests"][0]["request"]["scripts"]["pre"],
            "setup()"
        );
        assert_eq!(
            json["collections"][0]["requests"][0]["request"]["url"],
            "{{base}}/users"
        );
    }

    #[test]
    fn export_fails_loud_on_corrupt_workspace_file() {
        let dir = workspace_dir();
        seed(dir.path());
        std::fs::write(
            collection::collections_dir(dir.path())
                .join("acme-api")
                .join("bad.http"),
            "### Bad\nGETT https://x.dev/a\n",
        )
        .unwrap();
        let err = export(dir.path()).unwrap_err().to_string();
        assert!(err.contains("cannot export"), "{err}");
        assert!(err.contains("bad.http"), "{err}");
    }

    #[test]
    fn version_1_bundle_imports_into_default_collection() {
        let dir = workspace_dir();
        let legacy = serde_json::json!({
            "mandaloBundle": 1,
            "requests": [request("Create User"), request("Delete User")],
            "environments": [{"name": "prod", "vars": {"base": "https://x.dev"}}]
        })
        .to_string();
        let report = import(dir.path(), &legacy).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.collections, 1);
        assert_eq!(report.environments, 1);
        assert!(report.summary.contains("version 2"));
        let tree = collection::list_tree(dir.path()).unwrap();
        assert_eq!(tree.collections.len(), 1);
        assert_eq!(tree.collections[0].slug, "default");
        assert_eq!(tree.collections[0].name, "Default");
        let paths: Vec<&str> = tree.collections[0]
            .requests
            .iter()
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(paths, vec!["create-user.http#0", "delete-user.http#0"]);
        let reexported: serde_json::Value =
            serde_json::from_str(&export(dir.path()).unwrap().json).unwrap();
        assert_eq!(reexported["mandaloBundle"], 2);
    }

    #[test]
    fn version_1_bundle_without_scripts_fields_still_imports() {
        let dir = workspace_dir();
        let legacy = serde_json::json!({
            "mandaloBundle": 1,
            "requests": [{
                "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "name": "Ping",
                "kind": "http",
                "method": "GET",
                "url": "https://x.dev/ping",
                "auth": {"type": "none"}
            }],
            "environments": []
        })
        .to_string();
        import(dir.path(), &legacy).unwrap();
        let saved = collection::load_request(dir.path(), "default", "ping.http#0").unwrap();
        assert_eq!(saved.description, None);
        assert_eq!(saved.scripts, Scripts::default());
        assert!(saved.tests.is_empty());
        assert!(saved.captures.is_empty());
    }

    #[test]
    fn import_into_existing_collection_keeps_both_requests() {
        let dir = workspace_dir();
        seed(dir.path());
        let json = export(dir.path()).unwrap().json;
        let mut bundle: serde_json::Value = serde_json::from_str(&json).unwrap();
        bundle["collections"][0]["requests"][0]["path"] = serde_json::json!("fresh.http");
        import(dir.path(), &bundle.to_string()).unwrap();
        let tree = collection::list_tree(dir.path()).unwrap();
        let acme = &tree.collections[0];
        assert_eq!(acme.slug, "acme-api");
        assert_eq!(acme.requests.len(), 2);
    }

    #[test]
    fn unsupported_version_fails_loud() {
        let dir = workspace_dir();
        let err = import(dir.path(), "{\"mandaloBundle\": 9}")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported bundle version: 9"));
        assert!(err.contains("expected 1 or 2"));
    }

    #[test]
    fn malformed_bundle_paths_are_rejected() {
        let dir = workspace_dir();
        let json = serde_json::json!({
            "mandaloBundle": 2,
            "collections": [{
                "id": "11111111-1111-4111-8111-111111111111",
                "slug": "acme",
                "name": "Acme",
                "requests": [{"path": "../../evil.http", "request": request("Evil")}]
            }],
            "environments": []
        })
        .to_string();
        assert!(import(dir.path(), &json).is_err());
        assert!(!dir.path().parent().unwrap().join("evil.http").exists());
    }

    #[test]
    fn bundle_slug_is_validated() {
        let dir = workspace_dir();
        let json = serde_json::json!({
            "mandaloBundle": 2,
            "collections": [{
                "id": "11111111-1111-4111-8111-111111111111",
                "slug": "../escape",
                "name": "Acme",
                "requests": []
            }],
            "environments": []
        })
        .to_string();
        assert!(import(dir.path(), &json)
            .unwrap_err()
            .to_string()
            .contains("invalid collection slug"));
    }
}
