use crate::collection::{self, SavedRequest};
use crate::request::{Auth, GraphqlBody};
use crate::workspace::{self, Environment};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const PASSTHROUGH_NOTE: &str =
    "Postman {{variable}} syntax is identical to Mándalo's; variables were passed through untouched.";

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub environments: usize,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: String,
}

pub fn import(workspace: &Path, json: &str) -> Result<ImportReport, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
    if let Some(schema) = root.pointer("/info/schema").and_then(Value::as_str) {
        if !schema.contains("v2.1") {
            return Err(format!(
                "unsupported Postman schema: {schema} (only v2.1 collections are supported)"
            ));
        }
        return import_collection(workspace, &root);
    }
    if root.get("values").is_some_and(Value::is_array) && root.get("name").is_some() {
        return import_environment(workspace, &root);
    }
    Err(
        "unrecognized Postman export: expected a v2.1 collection (info.schema) or an environment (name + values)"
            .to_string(),
    )
}

fn import_collection(workspace: &Path, root: &Value) -> Result<ImportReport, String> {
    let collection_name = root
        .pointer("/info/name")
        .and_then(Value::as_str)
        .unwrap_or("postman-import");
    let mut report = ImportReport {
        imported: 0,
        environments: 0,
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: PASSTHROUGH_NOTE.to_string(),
    };
    let default_auth = match root.get("auth") {
        None => Auth::None,
        Some(a) => convert_auth(a, "collection", &mut report),
    };
    let items = root
        .get("item")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    walk(workspace, items, &mut Vec::new(), &default_auth, &mut report)?;

    if let Some(vars) = root.get("variable").and_then(Value::as_array) {
        let vars = collect_vars(vars, "key");
        if !vars.is_empty() {
            let env = Environment {
                name: workspace::sanitize_env_name(workspace, &format!("{collection_name}-vars")),
                vars,
            };
            workspace::save_environment(workspace, &env)?;
            report.environments += 1;
        }
    }
    Ok(report)
}

fn walk(
    workspace: &Path,
    items: &[Value],
    prefix: &mut Vec<String>,
    default_auth: &Auth,
    report: &mut ImportReport,
) -> Result<(), String> {
    for item in items {
        let item_name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unnamed")
            .to_string();
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            prefix.push(item_name);
            walk(workspace, children, prefix, default_auth, report)?;
            prefix.pop();
            continue;
        }
        let Some(request) = item.get("request") else {
            continue;
        };
        let full_name = if prefix.is_empty() {
            item_name
        } else {
            format!("{} / {item_name}", prefix.join(" / "))
        };
        if let Some(saved) = convert_request(request, &full_name, default_auth, report)? {
            collection::save_request(workspace, &saved)?;
            report.imported += 1;
        }
    }
    Ok(())
}

fn convert_request(
    request: &Value,
    name: &str,
    default_auth: &Auth,
    report: &mut ImportReport,
) -> Result<Option<SavedRequest>, String> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let url = convert_url(request.get("url"), name)?;

    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(list) = request.get("header").and_then(Value::as_array) {
        for h in list {
            if h.get("disabled").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let (Some(k), Some(v)) = (
                h.get("key").and_then(Value::as_str),
                h.get("value").and_then(Value::as_str),
            ) {
                headers.push((k.to_string(), v.to_string()));
            }
        }
    }

    let mut kind = "http".to_string();
    let mut body = None;
    let mut graphql = None;
    if let Some(b) = request.get("body").filter(|b| !b.is_null()) {
        match b.get("mode").and_then(Value::as_str).unwrap_or("raw") {
            "raw" => body = b.get("raw").and_then(Value::as_str).map(String::from),
            "graphql" => {
                kind = "graphql".to_string();
                let g = b
                    .get("graphql")
                    .ok_or_else(|| format!("{name}: graphql body mode without graphql payload"))?;
                graphql = Some(GraphqlBody {
                    query: g
                        .get("query")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    variables: g
                        .get("variables")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                });
            }
            "urlencoded" => {
                let pairs: Vec<String> = b
                    .get("urlencoded")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|e| e.get("disabled").and_then(Value::as_bool) != Some(true))
                    .filter_map(|e| {
                        Some(format!(
                            "{}={}",
                            e.get("key").and_then(Value::as_str)?,
                            e.get("value").and_then(Value::as_str)?
                        ))
                    })
                    .collect();
                body = Some(pairs.join("&"));
                if !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                {
                    headers.push((
                        "Content-Type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    ));
                }
            }
            mode => {
                report
                    .skipped
                    .push(format!("{name}: unsupported body mode {mode}"));
                return Ok(None);
            }
        }
    }

    let auth = match request.get("auth") {
        None => default_auth.clone(),
        Some(a) => convert_auth(a, name, report),
    };

    Ok(Some(SavedRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        kind,
        method,
        url,
        headers,
        body,
        auth,
        graphql,
        grpc: None,
    }))
}

fn convert_auth(a: &Value, name: &str, report: &mut ImportReport) -> Auth {
    match a.get("type").and_then(Value::as_str) {
        None | Some("noauth") => Auth::None,
        Some("bearer") => Auth::Bearer {
            token: auth_param(a, "bearer", "token").unwrap_or_default(),
        },
        Some("basic") => Auth::Basic {
            username: auth_param(a, "basic", "username").unwrap_or_default(),
            password: auth_param(a, "basic", "password").unwrap_or_default(),
        },
        Some("apikey") => Auth::Apikey {
            key: auth_param(a, "apikey", "key").unwrap_or_default(),
            value: auth_param(a, "apikey", "value").unwrap_or_default(),
            placement: match auth_param(a, "apikey", "in").as_deref() {
                Some("query") => "query".to_string(),
                _ => "header".to_string(),
            },
        },
        Some(other) => {
            report.warnings.push(format!(
                "{name}: unsupported auth type {other}, imported without auth"
            ));
            Auth::None
        }
    }
}

fn auth_param(auth: &Value, section: &str, key: &str) -> Option<String> {
    let sec = auth.get(section)?;
    if let Some(arr) = sec.as_array() {
        return arr
            .iter()
            .find(|e| e.get("key").and_then(Value::as_str) == Some(key))
            .and_then(|e| e.get("value"))
            .and_then(Value::as_str)
            .map(String::from);
    }
    sec.get(key).and_then(Value::as_str).map(String::from)
}

fn convert_url(url: Option<&Value>, name: &str) -> Result<String, String> {
    let url = url.ok_or_else(|| format!("{name}: request has no url"))?;
    if let Some(s) = url.as_str() {
        return Ok(s.to_string());
    }
    if !url.is_object() {
        return Err(format!("{name}: unsupported url shape: {url}"));
    }
    if let Some(raw) = url.get("raw").and_then(Value::as_str) {
        return Ok(raw.to_string());
    }
    let host = url
        .get("host")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(".")
        })
        .unwrap_or_default();
    if host.is_empty() {
        return Err(format!("{name}: url object has neither raw nor host"));
    }
    let mut out = match url.get("protocol").and_then(Value::as_str) {
        Some(p) => format!("{p}://{host}"),
        None => host,
    };
    match url.get("port") {
        None | Some(Value::Null) => {}
        Some(Value::String(p)) => {
            out.push(':');
            out.push_str(p);
        }
        Some(Value::Number(p)) => {
            out.push(':');
            out.push_str(&p.to_string());
        }
        Some(other) => return Err(format!("{name}: unsupported url port shape: {other}")),
    }
    if let Some(segments) = url.get("path").and_then(Value::as_array) {
        for segment in segments {
            let text = segment
                .as_str()
                .or_else(|| segment.get("value").and_then(Value::as_str))
                .ok_or_else(|| format!("{name}: unsupported url path segment: {segment}"))?;
            out.push('/');
            out.push_str(text);
        }
    }
    let query: Vec<String> = url
        .get("query")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|q| q.get("disabled").and_then(Value::as_bool) != Some(true))
        .filter_map(|q| {
            let key = encode_query_component(q.get("key").and_then(Value::as_str)?);
            Some(match q.get("value").and_then(Value::as_str) {
                Some(value) => format!("{key}={}", encode_query_component(value)),
                None => key,
            })
        })
        .collect();
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query.join("&"));
    }
    Ok(out)
}

// Braces stay literal so {{variable}} templates survive the import.
const QUERY_ENCODE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'=')
    .add(b'+');

fn encode_query_component(raw: &str) -> String {
    percent_encoding::utf8_percent_encode(raw, QUERY_ENCODE).to_string()
}

fn import_environment(workspace: &Path, root: &Value) -> Result<ImportReport, String> {
    let name = root
        .get("name")
        .and_then(Value::as_str)
        .ok_or("environment export has no name")?;
    let values = root
        .get("values")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let env = Environment {
        name: workspace::sanitize_env_name(workspace, name),
        vars: collect_vars(values, "key"),
    };
    workspace::save_environment(workspace, &env)?;
    Ok(ImportReport {
        imported: 0,
        environments: 1,
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: PASSTHROUGH_NOTE.to_string(),
    })
}

fn collect_vars(values: &[Value], key_field: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    for v in values {
        let enabled = v.get("enabled").and_then(Value::as_bool) != Some(false)
            && v.get("disabled").and_then(Value::as_bool) != Some(true);
        if !enabled {
            continue;
        }
        if let (Some(k), Some(val)) = (
            v.get(key_field).and_then(Value::as_str),
            v.get("value").and_then(Value::as_str),
        ) {
            vars.insert(k.to_string(), val.to_string());
        }
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection_fixture() -> String {
        serde_json::json!({
            "info": {
                "name": "Demo API",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [
                {
                    "name": "Users",
                    "item": [
                        {
                            "name": "Admin",
                            "item": [
                                {
                                    "name": "List",
                                    "request": {
                                        "method": "GET",
                                        "url": {"raw": "{{baseUrl}}/users"},
                                        "header": [
                                            {"key": "Accept", "value": "application/json"},
                                            {"key": "X-Debug", "value": "1", "disabled": true}
                                        ]
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "name": "Create",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/users",
                        "body": {"mode": "raw", "raw": "{\"name\": \"nova\"}"},
                        "auth": {
                            "type": "bearer",
                            "bearer": [{"key": "token", "value": "{{token}}", "type": "string"}]
                        }
                    }
                },
                {
                    "name": "Gql",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/graphql",
                        "body": {
                            "mode": "graphql",
                            "graphql": {"query": "{ ping }", "variables": "{\"limit\": 5}"}
                        }
                    }
                },
                {
                    "name": "Login",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/login",
                        "body": {
                            "mode": "urlencoded",
                            "urlencoded": [
                                {"key": "user", "value": "a"},
                                {"key": "pass", "value": "b"},
                                {"key": "debug", "value": "1", "disabled": true}
                            ]
                        }
                    }
                },
                {
                    "name": "Upload",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/upload",
                        "body": {"mode": "formdata", "formdata": []}
                    }
                },
                {
                    "name": "Search",
                    "request": {
                        "method": "GET",
                        "url": {
                            "protocol": "https",
                            "host": ["api", "x", "dev"],
                            "path": ["search"],
                            "query": [{"key": "q", "value": "1"}]
                        },
                        "auth": {
                            "type": "apikey",
                            "apikey": {"key": "api_key", "value": "abc", "in": "query"}
                        }
                    }
                },
                {
                    "name": "Signed",
                    "request": {
                        "method": "GET",
                        "url": "{{baseUrl}}/signed",
                        "auth": {"type": "awsv4", "awsv4": []}
                    }
                }
            ],
            "variable": [
                {"key": "baseUrl", "value": "https://api.x.dev"},
                {"key": "old", "value": "x", "disabled": true}
            ]
        })
        .to_string()
    }

    fn by_name<'a>(requests: &'a [SavedRequest], name: &str) -> &'a SavedRequest {
        requests.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn imports_v21_collection_fully() {
        let dir = tempfile::tempdir().unwrap();
        let report = import(dir.path(), &collection_fixture()).unwrap();
        assert_eq!(report.imported, 6);
        assert_eq!(report.environments, 1);
        assert_eq!(report.skipped, vec!["Upload: unsupported body mode formdata"]);
        assert_eq!(
            report.warnings,
            vec!["Signed: unsupported auth type awsv4, imported without auth"]
        );
        assert!(report.summary.contains("untouched"));

        let requests = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(requests.len(), 6);

        let list = by_name(&requests, "Users / Admin / List");
        assert_eq!(list.method, "GET");
        assert_eq!(list.url, "{{baseUrl}}/users");
        assert_eq!(
            list.headers,
            vec![("Accept".to_string(), "application/json".to_string())]
        );
        assert!(uuid::Uuid::parse_str(&list.id).is_ok());

        let create = by_name(&requests, "Create");
        assert_eq!(create.body.as_deref(), Some("{\"name\": \"nova\"}"));
        assert_eq!(
            create.auth,
            Auth::Bearer {
                token: "{{token}}".to_string()
            }
        );

        let gql = by_name(&requests, "Gql");
        assert_eq!(gql.kind, "graphql");
        assert_eq!(
            gql.graphql,
            Some(GraphqlBody {
                query: "{ ping }".to_string(),
                variables: "{\"limit\": 5}".to_string()
            })
        );

        let login = by_name(&requests, "Login");
        assert_eq!(login.body.as_deref(), Some("user=a&pass=b"));
        assert_eq!(
            login.headers,
            vec![(
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string()
            )]
        );

        let search = by_name(&requests, "Search");
        assert_eq!(search.url, "https://api.x.dev/search?q=1");
        assert_eq!(
            search.auth,
            Auth::Apikey {
                key: "api_key".to_string(),
                value: "abc".to_string(),
                placement: "query".to_string()
            }
        );

        assert_eq!(by_name(&requests, "Signed").auth, Auth::None);

        let envs = workspace::list_environments(dir.path()).unwrap().items;
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "Demo-API-vars");
        assert_eq!(envs[0].vars.get("baseUrl").unwrap(), "https://api.x.dev");
        assert!(!envs[0].vars.contains_key("old"));
    }

    #[test]
    fn v20_schema_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Old",
                "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json"
            },
            "item": []
        })
        .to_string();
        let err = import(dir.path(), &json).unwrap_err();
        assert!(err.contains("v2.0.0"));
        assert!(err.contains("only v2.1"));
    }

    #[test]
    fn imports_environment_export_excluding_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "name": "Staging (EU)",
            "values": [
                {"key": "baseUrl", "value": "https://staging.x.dev", "enabled": true},
                {"key": "secret", "value": "s", "enabled": false},
                {"key": "token", "value": "t"}
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.environments, 1);
        let envs = workspace::list_environments(dir.path()).unwrap().items;
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "Staging--EU-");
        assert_eq!(envs[0].vars.len(), 2);
        assert_eq!(envs[0].vars.get("token").unwrap(), "t");
        assert!(!envs[0].vars.contains_key("secret"));
    }

    #[test]
    fn sanitized_env_name_never_collides_with_existing_env() {
        let dir = tempfile::tempdir().unwrap();
        let existing = Environment {
            name: "Staging--EU-".to_string(),
            vars: BTreeMap::from([("keep".to_string(), "me".to_string())]),
        };
        workspace::save_environment(dir.path(), &existing).unwrap();
        let json = serde_json::json!({
            "name": "Staging (EU)",
            "values": [{"key": "base", "value": "https://eu.x.dev"}]
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let envs = workspace::list_environments(dir.path()).unwrap().items;
        let names: Vec<_> = envs.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Staging--EU-", "Staging--EU--2"]);
        assert_eq!(envs[0].vars.get("keep").unwrap(), "me");
    }

    #[test]
    fn url_object_keeps_port_path_variables_and_valueless_query() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Urls",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Get",
                "request": {
                    "method": "GET",
                    "url": {
                        "protocol": "http",
                        "host": ["localhost"],
                        "port": "8080",
                        "path": ["users", {"value": ":id"}, "posts"],
                        "query": [
                            {"key": "verbose", "value": null},
                            {"key": "q", "value": "a b&c=d"}
                        ]
                    }
                }
            }]
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let requests = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(
            requests[0].url,
            "http://localhost:8080/users/:id/posts?verbose&q=a%20b%26c%3Dd"
        );
    }

    #[test]
    fn numeric_port_and_template_query_survive() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Urls2",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Get",
                "request": {
                    "method": "GET",
                    "url": {
                        "protocol": "https",
                        "host": ["api", "x", "dev"],
                        "port": 8443,
                        "query": [{"key": "token", "value": "{{token}}"}]
                    }
                }
            }]
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let requests = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(requests[0].url, "https://api.x.dev:8443?token={{token}}");
    }

    #[test]
    fn collection_level_auth_is_default_for_requests_without_auth() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Authy",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "auth": {
                "type": "bearer",
                "bearer": [{"key": "token", "value": "{{token}}", "type": "string"}]
            },
            "item": [
                {"name": "Inherits", "request": {"method": "GET", "url": "https://x.dev/a"}},
                {
                    "name": "Overrides",
                    "request": {
                        "method": "GET",
                        "url": "https://x.dev/b",
                        "auth": {"type": "basic", "basic": [
                            {"key": "username", "value": "u"},
                            {"key": "password", "value": "p"}
                        ]}
                    }
                },
                {
                    "name": "OptsOut",
                    "request": {
                        "method": "GET",
                        "url": "https://x.dev/c",
                        "auth": {"type": "noauth"}
                    }
                }
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert!(report.warnings.is_empty());
        let requests = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(
            by_name(&requests, "Inherits").auth,
            Auth::Bearer {
                token: "{{token}}".to_string()
            }
        );
        assert_eq!(
            by_name(&requests, "Overrides").auth,
            Auth::Basic {
                username: "u".to_string(),
                password: "p".to_string()
            }
        );
        assert_eq!(by_name(&requests, "OptsOut").auth, Auth::None);
    }

    #[test]
    fn unsupported_collection_level_auth_warns_once() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Aws",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "auth": {"type": "awsv4", "awsv4": []},
            "item": [
                {"name": "A", "request": {"method": "GET", "url": "https://x.dev/a"}},
                {"name": "B", "request": {"method": "GET", "url": "https://x.dev/b"}}
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(
            report.warnings,
            vec!["collection: unsupported auth type awsv4, imported without auth"]
        );
        let requests = collection::list_requests(dir.path()).unwrap().items;
        assert_eq!(by_name(&requests, "A").auth, Auth::None);
        assert_eq!(by_name(&requests, "B").auth, Auth::None);
    }

    #[test]
    fn unrecognized_export_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let err = import(dir.path(), "{\"foo\": 1}").unwrap_err();
        assert!(err.contains("unrecognized Postman export"));
    }
}
