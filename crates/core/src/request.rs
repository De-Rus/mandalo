use crate::body::{self, Body};
use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Body::is_none")]
    pub body: Body,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Root that `formdata` and `binary` file paths resolve against. Bodies read
    /// files, so the send path carries the workspace explicitly instead of
    /// reaching for a global — a spec with no root cannot read any file at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    #[default]
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    Apikey {
        key: String,
        value: String,
        placement: String,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub binary: bool,
    pub duration_ms: u128,
    pub size_bytes: usize,
}

pub fn client() -> CoreResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let built = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| CoreError::Network(e.to_string()))?;
    Ok(CLIENT.get_or_init(|| built))
}

fn interpolate_json(
    value: &serde_json::Value,
    vars: &BTreeMap<String, String>,
) -> CoreResult<serde_json::Value> {
    use serde_json::Value;
    match value {
        Value::String(s) => Ok(Value::String(interpolate::apply(s, vars)?)),
        Value::Array(items) => items
            .iter()
            .map(|v| interpolate_json(v, vars))
            .collect::<CoreResult<Vec<_>>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(interpolate::apply(k, vars)?, interpolate_json(v, vars)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

enum Payload {
    Empty,
    Bytes(Vec<u8>),
    Multipart(reqwest::multipart::Form),
}

fn graphql_payload(
    query: &str,
    variables: &str,
    vars: &BTreeMap<String, String>,
) -> CoreResult<Vec<u8>> {
    let query = interpolate::apply(query, vars)?;
    let variables: serde_json::Value = if variables.trim().is_empty() {
        serde_json::json!({})
    } else {
        let parsed = serde_json::from_str(variables)
            .map_err(|e| CoreError::Parse(format!("invalid graphql variables JSON: {e}")))?;
        interpolate_json(&parsed, vars)?
    };
    Ok(
        serde_json::json!({ "query": query, "variables": variables })
            .to_string()
            .into_bytes(),
    )
}

fn multipart_form(
    rows: &[body::FormDataRow],
    spec: &RequestSpec,
) -> CoreResult<reqwest::multipart::Form> {
    let vars = &spec.vars;
    let mut form = reqwest::multipart::Form::new();
    for row in rows.iter().filter(|r| r.enabled) {
        let key = interpolate::apply(&row.key, vars)?;
        if !row.is_file() {
            form = form.text(key, interpolate::apply(&row.value, vars)?);
            continue;
        }
        for path in &row.files {
            let file = body::read_file(
                spec.workspace.as_deref(),
                path,
                row.content_type.as_deref(),
                vars,
            )
            .map_err(|e| CoreError::Request(format!("form-data field {key:?}: {}", e.message())))?;
            let part = reqwest::multipart::Part::bytes(file.bytes)
                .file_name(file.file_name)
                .mime_str(&file.content_type)
                .map_err(|e| {
                    CoreError::Request(format!(
                        "form-data field {key:?}: invalid content type {:?}: {e}",
                        file.content_type
                    ))
                })?;
            form = form.part(key.clone(), part);
        }
    }
    Ok(form)
}

fn payload(spec: &RequestSpec) -> CoreResult<(Payload, Option<String>)> {
    let vars = &spec.vars;
    Ok(match &spec.body {
        Body::None => (Payload::Empty, None),
        Body::Raw { language, text } => (
            Payload::Bytes(interpolate::apply(text, vars)?.into_bytes()),
            Some(language.content_type().to_string()),
        ),
        Body::Urlencoded { rows } => (
            Payload::Bytes(body::encode_urlencoded(rows, vars)?),
            Some("application/x-www-form-urlencoded".to_string()),
        ),
        Body::Formdata { rows } => (Payload::Multipart(multipart_form(rows, spec)?), None),
        Body::Binary { file, content_type } => {
            let file = body::read_file(
                spec.workspace.as_deref(),
                file,
                content_type.as_deref(),
                vars,
            )?;
            (Payload::Bytes(file.bytes), Some(file.content_type))
        }
        Body::Graphql { query, variables } => (
            Payload::Bytes(graphql_payload(query, variables, vars)?),
            Some("application/json".to_string()),
        ),
    })
}

pub fn build(client: &reqwest::Client, spec: &RequestSpec) -> CoreResult<reqwest::Request> {
    let vars = &spec.vars;
    let url = interpolate::apply(&spec.url, vars)?;

    let mut headers = Vec::with_capacity(spec.headers.len());
    for (k, v) in &spec.headers {
        headers.push((interpolate::apply(k, vars)?, interpolate::apply(v, vars)?));
    }

    match spec.kind.as_str() {
        "http" => {}
        "graphql" => {
            if !matches!(spec.body, Body::Graphql { .. }) {
                return Err(CoreError::Request(
                    "graphql request is missing the graphql body".to_string(),
                ));
            }
        }
        other => {
            return Err(CoreError::Unsupported(format!(
                "unsupported request kind: {other}"
            )))
        }
    }

    let method = match spec.body {
        Body::Graphql { .. } => reqwest::Method::POST,
        _ => spec
            .method
            .to_uppercase()
            .parse()
            .map_err(|_| CoreError::Request(format!("invalid method: {}", spec.method)))?,
    };
    let (payload, content_type) = payload(spec)?;

    if matches!(spec.auth, Auth::Bearer { .. } | Auth::Basic { .. }) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
    }
    // A user Content-Type cannot survive multipart: it would arrive without the
    // boundary reqwest generates, and the server could not parse a single part.
    if matches!(payload, Payload::Multipart(_)) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
    }
    let user_set_content_type = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));

    let mut req = client.request(method, &url);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    if let Some(ct) = content_type {
        if !user_set_content_type {
            req = req.header("Content-Type", ct);
        }
    }
    match payload {
        Payload::Empty => {}
        Payload::Bytes(bytes) => req = req.body(bytes),
        Payload::Multipart(form) => req = req.multipart(form),
    }

    req = match &spec.auth {
        Auth::None => req,
        Auth::Bearer { token } => req.header(
            "Authorization",
            format!("Bearer {}", interpolate::apply(token, vars)?),
        ),
        Auth::Basic { username, password } => req.basic_auth(
            interpolate::apply(username, vars)?,
            Some(interpolate::apply(password, vars)?),
        ),
        Auth::Apikey {
            key,
            value,
            placement,
        } => {
            let k = interpolate::apply(key, vars)?;
            let v = interpolate::apply(value, vars)?;
            match placement.as_str() {
                "header" => req.header(&k, &v),
                "query" => req.query(&[(k, v)]),
                other => {
                    return Err(CoreError::Unsupported(format!(
                        "unsupported apikey placement: {other}"
                    )))
                }
            }
        }
    };

    req.build().map_err(|e| CoreError::Request(e.to_string()))
}

pub async fn send_request(spec: RequestSpec) -> CoreResult<ResponseData> {
    let client = client()?;
    let req = build(client, &spec)?;
    send_built(client, req).await
}

pub async fn send_built(
    client: &reqwest::Client,
    req: reqwest::Request,
) -> CoreResult<ResponseData> {
    let started = Instant::now();
    let resp = client
        .execute(req)
        .await
        .map_err(|e| CoreError::Network(e.to_string()))?;
    let status = resp.status();
    let status_text = resp
        .extensions()
        .get::<hyper::ext::ReasonPhrase>()
        .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned())
        .or_else(|| status.canonical_reason().map(str::to_string))
        .unwrap_or_default();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| CoreError::Network(e.to_string()))?;
    let size_bytes = bytes.len();
    let (body, binary) = match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (text, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    };
    Ok(ResponseData {
        status: status.as_u16(),
        status_text,
        headers,
        body,
        binary,
        size_bytes,
        duration_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: serde_json::Value) -> RequestSpec {
        serde_json::from_value(json).unwrap()
    }

    fn build_ok(json: serde_json::Value) -> reqwest::Request {
        build(client().unwrap(), &spec(json)).unwrap()
    }

    fn build_err(json: serde_json::Value) -> String {
        build(client().unwrap(), &spec(json))
            .unwrap_err()
            .to_string()
    }

    fn header<'a>(req: &'a reqwest::Request, name: &str) -> &'a str {
        req.headers().get(name).unwrap().to_str().unwrap()
    }

    fn body_str(req: &reqwest::Request) -> String {
        String::from_utf8(req.body().unwrap().as_bytes().unwrap().to_vec()).unwrap()
    }

    #[test]
    fn bearer_auth_sets_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "get", "url": "https://x.dev/a",
            "auth": {"type": "bearer", "token": "{{t}}"},
            "vars": {"t": "secret"}
        }));
        assert_eq!(header(&req, "authorization"), "Bearer secret");
    }

    #[test]
    fn basic_auth_sets_base64_credentials() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "basic", "username": "user", "password": "pass"}
        }));
        assert_eq!(header(&req, "authorization"), "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn apikey_header_placement() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "apikey", "key": "X-Api-Key", "value": "{{k}}", "placement": "header"},
            "vars": {"k": "abc"}
        }));
        assert_eq!(header(&req, "x-api-key"), "abc");
    }

    #[test]
    fn apikey_query_placement_appends_param() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev/search?q=1",
            "auth": {"type": "apikey", "key": "api_key", "value": "abc", "placement": "query"}
        }));
        assert_eq!(req.url().as_str(), "https://x.dev/search?q=1&api_key=abc");
    }

    #[test]
    fn apikey_unknown_placement_fails() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "apikey", "key": "k", "value": "v", "placement": "cookie"}
        }));
        assert!(err.contains("unsupported apikey placement: cookie"));
    }

    #[test]
    fn graphql_forces_post_json_body() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "GET", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "query { user(id: {{id}}) { name } }", "variables": "{\"limit\": 5}"},
            "vars": {"id": "7"}
        }));
        assert_eq!(req.method(), reqwest::Method::POST);
        assert_eq!(header(&req, "content-type"), "application/json");
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(body["query"], "query { user(id: 7) { name } }");
        assert_eq!(body["variables"], serde_json::json!({"limit": 5}));
    }

    #[test]
    fn graphql_empty_variables_becomes_empty_object() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(body["variables"], serde_json::json!({}));
    }

    #[test]
    fn graphql_invalid_variables_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": "not json"}
        }));
        assert!(err.contains("invalid graphql variables JSON"));
    }

    #[test]
    fn graphql_without_body_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql"
        }));
        assert!(err.contains("missing the graphql body"));
    }

    #[test]
    fn interpolates_url_headers_and_body() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "POST", "url": "{{base}}/users",
            "headers": [["X-{{suffix}}", "v-{{suffix}}"]],
            "body": {"mode": "raw", "language": "text", "text": "hello {{name}}"},
            "vars": {"base": "https://x.dev", "suffix": "trace", "name": "nova"}
        }));
        assert_eq!(req.url().as_str(), "https://x.dev/users");
        assert_eq!(header(&req, "x-trace"), "v-trace");
        assert_eq!(body_str(&req), "hello nova");
    }

    #[test]
    fn unresolved_var_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GET", "url": "{{base}}/users"
        }));
        assert!(err.contains("unresolved variable: base"));
    }

    #[test]
    fn unsupported_kind_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "soap", "method": "GET", "url": "https://x.dev"
        }));
        assert!(err.contains("unsupported request kind: soap"));
    }

    #[test]
    fn graphql_variable_value_cannot_inject_json_keys() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": "{\"q\":\"{{term}}\"}"},
            "vars": {"term": "x\", \"isAdmin\": true, \"z\": \""}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        let variables = body["variables"].as_object().unwrap();
        assert_eq!(variables.len(), 1);
        assert_eq!(variables["q"], "x\", \"isAdmin\": true, \"z\": \"");
    }

    #[test]
    fn graphql_interpolates_nested_keys_and_values() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {
                "mode": "graphql",
                "query": "{ ping }",
                "variables": "{\"filter\": {\"{{field}}\": [\"{{v}}\", 3]}, \"n\": 7}"
            },
            "vars": {"field": "status", "v": "open"}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(
            body["variables"],
            serde_json::json!({"filter": {"status": ["open", 3]}, "n": 7})
        );
    }

    #[test]
    fn user_content_type_wins_over_graphql_default() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "headers": [["content-type", "application/graphql-response+json"]],
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        let values: Vec<_> = req.headers().get_all("content-type").iter().collect();
        assert_eq!(values, vec!["application/graphql-response+json"]);
    }

    #[test]
    fn bearer_auth_replaces_user_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["Authorization", "stale"]],
            "auth": {"type": "bearer", "token": "fresh"}
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["Bearer fresh"]);
    }

    #[test]
    fn basic_auth_replaces_user_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["authorization", "stale"]],
            "auth": {"type": "basic", "username": "u", "password": "p"}
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["Basic dTpw"]);
    }

    #[test]
    fn user_authorization_header_kept_without_bearer_or_basic() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["Authorization", "custom scheme"]]
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["custom scheme"]);
    }

    #[test]
    fn each_mode_sets_its_own_content_type() {
        for (body, expected) in [
            (
                serde_json::json!({"mode": "raw", "language": "json", "text": "{}"}),
                "application/json",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "xml", "text": "<a/>"}),
                "application/xml",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "html", "text": "<p>"}),
                "text/html",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "javascript", "text": "1"}),
                "application/javascript",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "text", "text": "hola"}),
                "text/plain",
            ),
            (
                serde_json::json!({"mode": "urlencoded", "rows": [{"key": "a", "value": "1"}]}),
                "application/x-www-form-urlencoded",
            ),
            (
                serde_json::json!({"mode": "graphql", "query": "{ ping }"}),
                "application/json",
            ),
        ] {
            let req = build_ok(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev", "body": body
            }));
            assert_eq!(header(&req, "content-type"), expected, "{body}");
        }
    }

    #[test]
    fn a_user_content_type_wins_in_every_mode_and_is_never_duplicated() {
        for body in [
            serde_json::json!({"mode": "raw", "language": "json", "text": "{}"}),
            serde_json::json!({"mode": "urlencoded", "rows": [{"key": "a", "value": "1"}]}),
            serde_json::json!({"mode": "graphql", "query": "{ ping }"}),
        ] {
            let req = build_ok(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev",
                "headers": [["Content-Type", "application/vnd.acme+json"]],
                "body": body
            }));
            let values: Vec<_> = req.headers().get_all("content-type").iter().collect();
            assert_eq!(values, vec!["application/vnd.acme+json"], "{body}");
        }
    }

    #[test]
    fn an_empty_body_sends_no_content_type() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev"
        }));
        assert!(req.headers().get("content-type").is_none());
        assert!(req.body().is_none());
    }

    #[test]
    fn urlencoded_body_is_form_encoded_and_skips_disabled_rows() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev",
            "body": {"mode": "urlencoded", "rows": [
                {"key": "user", "value": "{{who}} lovelace"},
                {"key": "debug", "value": "1", "enabled": false},
                {"key": "q", "value": "a&b"}
            ]},
            "vars": {"who": "ada"}
        }));
        assert_eq!(body_str(&req), "user=ada+lovelace&q=a%26b");
    }

    #[test]
    fn a_graphql_body_forces_post_whatever_the_method_says() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        assert_eq!(req.method(), reqwest::Method::POST);
    }

    #[test]
    fn a_file_body_without_a_workspace_cannot_read_anything() {
        for body in [
            serde_json::json!({"mode": "binary", "file": "files/a.png"}),
            serde_json::json!({"mode": "formdata", "rows": [{"key": "a", "files": ["files/a.png"]}]}),
        ] {
            let err = build_err(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev", "body": body
            }));
            assert!(err.contains("no workspace root"), "{err}");
        }
    }

    #[test]
    fn shared_client_is_reused() {
        assert!(std::ptr::eq(client().unwrap(), client().unwrap()));
    }

    #[test]
    fn invalid_method_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GE T", "url": "https://x.dev"
        }));
        assert!(err.contains("invalid method"));
    }
}
