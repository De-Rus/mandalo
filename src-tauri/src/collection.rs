use crate::request::{Auth, GraphqlBody};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SavedRequest {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub graphql: Option<GraphqlBody>,
    #[serde(default)]
    pub grpc: Option<GrpcRequest>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrpcRequest {
    pub proto_paths: Vec<String>,
    pub service: String,
    pub method: String,
    pub message: String,
    #[serde(default)]
    pub metadata: Vec<(String, String)>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestList {
    pub items: Vec<SavedRequest>,
    pub skipped: Vec<String>,
}

fn requests_dir(workspace: &Path) -> PathBuf {
    workspace.join("requests")
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid request id: {id:?}"));
    }
    Ok(())
}

pub fn list_requests(workspace: &Path) -> Result<RequestList, String> {
    let dir = requests_dir(workspace);
    let mut list = RequestList {
        items: Vec::new(),
        skipped: Vec::new(),
    };
    if !dir.exists() {
        return Ok(list);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|raw| toml::from_str::<SavedRequest>(&raw).map_err(|e| e.to_string()));
        match parsed {
            Ok(request) => list.items.push(request),
            Err(e) => list.skipped.push(format!("{}: {e}", path.display())),
        }
    }
    list.items.sort_by(|a, b| a.name.cmp(&b.name));
    list.skipped.sort();
    Ok(list)
}

pub fn save_request(workspace: &Path, request: &SavedRequest) -> Result<PathBuf, String> {
    validate_id(&request.id)?;
    let dir = requests_dir(workspace);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.toml", request.id));
    let raw = toml::to_string_pretty(request).map_err(|e| e.to_string())?;
    crate::workspace::atomic_write(&path, &raw)?;
    Ok(path)
}

pub fn delete_request(workspace: &Path, id: &str) -> Result<(), String> {
    validate_id(id)?;
    let path = requests_dir(workspace).join(format!("{id}.toml"));
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_request(id: &str, name: &str) -> SavedRequest {
        SavedRequest {
            id: id.to_string(),
            name: name.to_string(),
            kind: "http".to_string(),
            method: "POST".to_string(),
            url: "https://x.dev/users".to_string(),
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: Some("{\"a\": 1}".to_string()),
            auth: Auth::Bearer {
                token: "{{token}}".to_string(),
            },
            graphql: None,
            grpc: None,
        }
    }

    #[test]
    fn save_list_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let b = http_request("11111111-1111-4111-8111-111111111111", "Beta");
        let a = http_request("22222222-2222-4222-8222-222222222222", "Alpha");
        save_request(dir.path(), &b).unwrap();
        save_request(dir.path(), &a).unwrap();
        let list = list_requests(dir.path()).unwrap();
        assert_eq!(list.items, vec![a.clone(), b.clone()]);
        assert!(list.skipped.is_empty());
        delete_request(dir.path(), &b.id).unwrap();
        assert_eq!(list_requests(dir.path()).unwrap().items, vec![a]);
    }

    #[test]
    fn corrupt_request_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let good = http_request("11111111-1111-4111-8111-111111111111", "Good");
        save_request(dir.path(), &good).unwrap();
        let bad = dir.path().join("requests").join("bad.toml");
        std::fs::write(&bad, "kind = [ broken").unwrap();
        let list = list_requests(dir.path()).unwrap();
        assert_eq!(list.items, vec![good]);
        assert_eq!(list.skipped.len(), 1);
        assert!(list.skipped[0].contains("bad.toml"));
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        save_request(dir.path(), &http_request("abc", "A")).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path().join("requests"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["abc.toml"]);
    }

    #[test]
    fn path_traversal_id_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut req = http_request("ok", "Evil");
        req.id = "../evil".to_string();
        assert!(save_request(dir.path(), &req).unwrap_err().contains("invalid request id"));
        req.id = String::new();
        assert!(save_request(dir.path(), &req).is_err());
        assert!(delete_request(dir.path(), "../../etc/passwd").is_err());
    }

    #[test]
    fn grpc_request_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let req = SavedRequest {
            id: "33333333-3333-4333-8333-333333333333".to_string(),
            name: "Echo Say".to_string(),
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
                message: "{\"text\": \"hola\"}".to_string(),
                metadata: vec![("x-trace".to_string(), "1".to_string())],
            }),
        };
        save_request(dir.path(), &req).unwrap();
        assert_eq!(list_requests(dir.path()).unwrap().items, vec![req]);
    }

    #[test]
    fn graphql_request_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let req = SavedRequest {
            id: "44444444-4444-4444-8444-444444444444".to_string(),
            name: "Gql".to_string(),
            kind: "graphql".to_string(),
            method: "POST".to_string(),
            url: "https://x.dev/graphql".to_string(),
            headers: Vec::new(),
            body: None,
            auth: Auth::Apikey {
                key: "X-Api-Key".to_string(),
                value: "k".to_string(),
                placement: "header".to_string(),
            },
            graphql: Some(GraphqlBody {
                query: "{ ping }".to_string(),
                variables: "{}".to_string(),
            }),
            grpc: None,
        };
        save_request(dir.path(), &req).unwrap();
        assert_eq!(list_requests(dir.path()).unwrap().items, vec![req]);
    }
}
