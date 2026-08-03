use crate::assertions::Scripts;
use crate::body::{Body, FormDataRow, RawLanguage};
use crate::collection::{self, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::request::Auth;
use crate::workspace::{self, Environment};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const PASSTHROUGH_NOTE: &str =
    "Postman {{variable}} syntax is identical to Mándalo's; variables were passed through untouched.";

const UNSUPPORTED_SCRIPT_APIS: &[(&str, &str)] = &[
    (
        "pm.sendRequest",
        "scripts cannot make network requests; chain a real request instead",
    ),
    (
        "pm.execution",
        "run-flow control is not implemented; requests run in collection order",
    ),
    (
        "setNextRequest",
        "run-flow control is not implemented; requests run in collection order",
    ),
    (
        "pm.iterationData",
        "there is no data-file runner; pass the value as an environment variable",
    ),
    ("pm.cookies", "there is no cookie jar"),
    ("pm.vault", "the Postman vault is not implemented"),
    (
        "pm.visualizer",
        "response visualizations are not implemented",
    ),
    ("jsonSchema", "no JSON-schema validator is bundled"),
    (
        "require(",
        "there is no module loader; scripts cannot import code",
    ),
    (
        "CryptoJS",
        "crypto-js is not bundled; compute the value outside the script",
    ),
    ("moment(", "moment is not bundled; use the built-in Date"),
    ("xml2Json", "the XML helper is not bundled"),
];

struct Inherited {
    auth: Auth,
    pre: Vec<(String, String)>,
    post: Vec<(String, String)>,
}

impl Inherited {
    fn with(&self, auth: Auth, scope: &str, scripts: &Scripts) -> Inherited {
        let mut next = Inherited {
            auth,
            pre: self.pre.clone(),
            post: self.post.clone(),
        };
        if let Some(pre) = scripts.pre.clone() {
            next.pre.push((scope.to_string(), pre));
        }
        if let Some(post) = scripts.post.clone() {
            next.post.push((scope.to_string(), post));
        }
        next
    }
}

fn inline(inherited: &[(String, String)], own: Option<String>, kind: &str) -> Option<String> {
    if inherited.is_empty() {
        return own;
    }
    let mut parts: Vec<String> = inherited
        .iter()
        .map(|(scope, source)| {
            format!("// inlined by the Postman import: {scope} {kind} script\n{source}")
        })
        .collect();
    if let Some(own) = own {
        parts.push(own);
    }
    Some(parts.join("\n\n"))
}

fn note_inlined_scripts(
    scope: &str,
    scripts: &Scripts,
    affected: &[String],
    report: &mut ImportReport,
) {
    if scripts.is_empty() {
        return;
    }
    let kinds = match (scripts.pre.is_some(), scripts.post.is_some()) {
        (true, true) => "pre-request and test scripts",
        (true, false) => "pre-request script",
        (false, true) => "test script",
        (false, false) => return,
    };
    if affected.is_empty() {
        report.warnings.push(format!(
            "{scope}: {kinds} dropped — it contains no requests to run them against"
        ));
        return;
    }
    let shown: Vec<&str> = affected.iter().take(5).map(String::as_str).collect();
    let rest = affected.len().saturating_sub(shown.len());
    let list = if rest == 0 {
        shown.join(", ")
    } else {
        format!("{} and {rest} more", shown.join(", "))
    };
    report.warnings.push(format!(
        "{scope}: {kinds} copied into every request below it ({list}) — Mándalo has no shared scripts, so edit each copy"
    ));
}

fn note_unsupported_script_apis(name: &str, scripts: &Scripts, report: &mut ImportReport) {
    let mut seen: Vec<&str> = Vec::new();
    for source in [scripts.pre.as_deref(), scripts.post.as_deref()]
        .into_iter()
        .flatten()
    {
        for (api, why) in UNSUPPORTED_SCRIPT_APIS {
            if source.contains(api) && !seen.contains(api) {
                seen.push(api);
                report.warnings.push(format!(
                    "{name}: script uses {api}, which Mándalo does not support — {why}; the script stops there with that message when the request runs"
                ));
            }
        }
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub collections: usize,
    pub environments: usize,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: String,
}

pub fn import(workspace: &Path, json: &str) -> CoreResult<ImportReport> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| CoreError::Parse(format!("invalid JSON: {e}")))?;
    if let Some(schema) = root.pointer("/info/schema").and_then(Value::as_str) {
        if !schema.contains("v2.1") {
            return Err(CoreError::Unsupported(format!(
                "unsupported Postman schema: {schema} (only v2.1 collections are supported)"
            )));
        }
        return import_collection(workspace, &root);
    }
    if root.get("values").is_some_and(Value::is_array) && root.get("name").is_some() {
        return import_environment(workspace, &root);
    }
    Err(CoreError::Unsupported(
        "unrecognized Postman export: expected a v2.1 collection (info.schema) or an environment (name + values)"
            .to_string(),
    ))
}

fn import_collection(workspace: &Path, root: &Value) -> CoreResult<ImportReport> {
    let collection_name = root
        .pointer("/info/name")
        .and_then(Value::as_str)
        .unwrap_or("postman-import");
    let mut report = ImportReport {
        imported: 0,
        collections: 0,
        environments: 0,
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: PASSTHROUGH_NOTE.to_string(),
    };
    let default_auth = match root.get("auth") {
        None => Auth::None,
        Some(a) => convert_auth(a, "collection", &mut report),
    };
    let target = collection::create_collection(workspace, collection_name)?;
    report.collections = 1;
    let scripts = extract_scripts(root);
    let inherited = Inherited {
        auth: Auth::None,
        pre: Vec::new(),
        post: Vec::new(),
    }
    .with(
        default_auth,
        &format!("collection \"{collection_name}\""),
        &scripts,
    );
    let items = root
        .get("item")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let affected = walk(workspace, &target.slug, items, "", &inherited, &mut report)?;
    note_inlined_scripts(collection_name, &scripts, &affected, &mut report);

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
    slug: &str,
    items: &[Value],
    folder: &str,
    inherited: &Inherited,
    report: &mut ImportReport,
) -> CoreResult<Vec<String>> {
    let mut imported: Vec<String> = Vec::new();
    for item in items {
        let item_name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unnamed")
            .to_string();
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let created = collection::create_folder_named(workspace, slug, folder, &item_name)?;
            let scripts = extract_scripts(item);
            let auth = match item.get("auth") {
                None => inherited.auth.clone(),
                Some(a) => convert_auth(a, &item_name, report),
            };
            let nested = inherited.with(auth, &format!("folder \"{item_name}\""), &scripts);
            let affected = walk(workspace, slug, children, &created.path, &nested, report)?;
            note_inlined_scripts(&item_name, &scripts, &affected, report);
            imported.extend(affected);
            continue;
        }
        let Some(request) = item.get("request") else {
            continue;
        };
        let Some(mut saved) = convert_request(request, &item_name, inherited, report)? else {
            continue;
        };
        let own = extract_scripts(item);
        saved.scripts = Scripts {
            pre: inline(&inherited.pre, own.pre.clone(), "pre-request"),
            post: inline(&inherited.post, own.post.clone(), "test"),
        };
        note_unsupported_script_apis(&item_name, &saved.scripts, report);
        if item
            .get("description")
            .or_else(|| request.get("description"))
            .and_then(Value::as_str)
            .is_some_and(|d| !d.trim().is_empty())
        {
            report.warnings.push(format!(
                "{item_name}: the description was dropped — neither .http nor .grpc has a line for one; keep it in a `#` comment above the request"
            ));
        }
        collection::save_request(workspace, slug, None, Some(folder), &saved)?;
        report.imported += 1;
        imported.push(item_name);
    }
    Ok(imported)
}

fn extract_scripts(item: &Value) -> Scripts {
    let mut scripts = Scripts::default();
    let Some(events) = item.get("event").and_then(Value::as_array) else {
        return scripts;
    };
    for event in events {
        let Some(exec) = event.pointer("/script/exec") else {
            continue;
        };
        let text = match exec {
            Value::String(s) => s.clone(),
            Value::Array(lines) => lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        match event.get("listen").and_then(Value::as_str) {
            Some("prerequest") => scripts.pre = Some(text),
            Some("test") => scripts.post = Some(text),
            _ => {}
        }
    }
    scripts
}

/// Postman writes `src` as a string for one file and as an array for a
/// multi-file field.
fn sources(entry: &Value) -> Vec<String> {
    match entry.get("src") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

fn row_description(entry: &Value, unresolved: &[String]) -> Option<String> {
    let own = entry
        .get("description")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map(String::from);
    if unresolved.is_empty() {
        return own;
    }
    let note = format!(
        "The Postman export referenced {} — pick the file inside the workspace.",
        unresolved.join(", ")
    );
    Some(match own {
        Some(own) => format!("{own}\n{note}"),
        None => note,
    })
}

fn note_unresolved_file(name: &str, field: &str, sources: &[String], report: &mut ImportReport) {
    let referenced = match sources.len() {
        0 => "no path at all".to_string(),
        _ => sources.join(", "),
    };
    report.warnings.push(format!(
        "{name}: file field `{field}` needs a file inside the workspace — the export referenced {referenced}"
    ));
}

fn convert_body(
    b: &Value,
    name: &str,
    headers: &mut Vec<(String, String)>,
    report: &mut ImportReport,
) -> CoreResult<(String, Body)> {
    match b.get("mode").and_then(Value::as_str).unwrap_or("raw") {
        "raw" => {
            let text = b
                .get("raw")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let declared = b
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
                .and_then(RawLanguage::from_postman);
            // A .http file reads the language back off the Content-Type, so the
            // language Postman recorded has to be written as the header it always
            // became on the wire.
            let body = match declared {
                Some(language) => {
                    if !headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    {
                        headers.push((
                            "Content-Type".to_string(),
                            language.content_type().to_string(),
                        ));
                    }
                    Body::Raw { language, text }
                }
                None => Body::raw(text),
            };
            Ok(("http".to_string(), body))
        }
        "graphql" => {
            let g = b.get("graphql").ok_or_else(|| {
                CoreError::Parse(format!("{name}: graphql body mode without graphql payload"))
            })?;
            Ok((
                "graphql".to_string(),
                Body::graphql(
                    g.get("query").and_then(Value::as_str).unwrap_or(""),
                    g.get("variables").and_then(Value::as_str).unwrap_or(""),
                ),
            ))
        }
        // A .http file writes a form body the way the format itself prescribes: one
        // line of `a=1&b=2` text under an explicit Content-Type. Braces stay literal
        // so {{variable}} templates survive.
        "urlencoded" => {
            let mut pairs: Vec<String> = Vec::new();
            let mut dropped: Vec<&str> = Vec::new();
            for e in b
                .get("urlencoded")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let Some(key) = e.get("key").and_then(Value::as_str) else {
                    continue;
                };
                if e.get("disabled").and_then(Value::as_bool) == Some(true) {
                    dropped.push(key);
                    continue;
                }
                pairs.push(format!(
                    "{}={}",
                    encode_query_component(key),
                    encode_query_component(e.get("value").and_then(Value::as_str).unwrap_or(""))
                ));
            }
            if !dropped.is_empty() {
                report.warnings.push(format!(
                    "{name}: disabled form fields ({}) were dropped — a .http file writes the whole form body as one line of text",
                    dropped.join(", ")
                ));
            }
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                headers.push((
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ));
            }
            Ok((
                "http".to_string(),
                Body::Raw {
                    language: RawLanguage::Text,
                    text: pairs.join("&"),
                },
            ))
        }
        "formdata" => {
            let mut rows = Vec::new();
            for entry in b
                .get("formdata")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let Some(key) = entry.get("key").and_then(Value::as_str) else {
                    continue;
                };
                let enabled = entry.get("disabled").and_then(Value::as_bool) != Some(true);
                if entry.get("type").and_then(Value::as_str) == Some("file") {
                    let srcs = sources(entry);
                    note_unresolved_file(name, key, &srcs, report);
                    rows.push(FormDataRow {
                        key: key.to_string(),
                        value: String::new(),
                        files: Vec::new(),
                        content_type: entry
                            .get("contentType")
                            .and_then(Value::as_str)
                            .map(String::from),
                        enabled,
                        description: row_description(entry, &srcs),
                    });
                    continue;
                }
                rows.push(FormDataRow {
                    key: key.to_string(),
                    value: entry
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    files: Vec::new(),
                    content_type: entry
                        .get("contentType")
                        .and_then(Value::as_str)
                        .map(String::from),
                    enabled,
                    description: row_description(entry, &[]),
                });
            }
            report.skipped.push(format!(
                "{name}: {} form-data fields not imported — a .http file cannot express a multipart body, so the request was imported without one",
                rows.len()
            ));
            Ok(("http".to_string(), Body::None))
        }
        "file" => {
            let srcs = b.get("file").map(sources).unwrap_or_default();
            let referenced = match srcs.is_empty() {
                true => "no path at all".to_string(),
                false => srcs.join(", "),
            };
            report.warnings.push(format!(
                "{name}: the binary body needs a file inside the workspace — the export referenced {referenced}; imported without a body, so write `< ./your-file` under the request"
            ));
            Ok(("http".to_string(), Body::None))
        }
        other => {
            report.skipped.push(format!(
                "{name}: {other} body not imported — the request was imported without a body, so it will not send one"
            ));
            Ok(("http".to_string(), Body::None))
        }
    }
}

fn convert_request(
    request: &Value,
    name: &str,
    inherited: &Inherited,
    report: &mut ImportReport,
) -> CoreResult<Option<SavedRequest>> {
    if let Some(url) = request.as_str() {
        return convert_request(
            &serde_json::json!({ "method": "GET", "url": url }),
            name,
            inherited,
            report,
        );
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_string();
    let mut url = convert_url(request.get("url"), name, report)?;

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

    let (kind, body) = match request.get("body").filter(|b| !b.is_null()) {
        Some(b) => convert_body(b, name, &mut headers, report)?,
        None => ("http".to_string(), Body::None),
    };

    let auth = match request.get("auth") {
        None => inherited.auth.clone(),
        Some(a) => convert_auth(a, name, report),
    };
    // A .http file has no typed api-key-in-the-query auth. The key goes where it
    // was always going to go on the wire: into the URL.
    let auth = match auth {
        Auth::Apikey {
            key,
            value,
            placement,
        } if placement == "query" => {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&format!(
                "{}={}",
                encode_query_component(&key),
                encode_query_component(&value)
            ));
            report.warnings.push(format!(
                "{name}: the API key moved into the URL query — a .http file writes an api key as a header or a query parameter, not as typed auth"
            ));
            Auth::None
        }
        other => other,
    };

    Ok(Some(SavedRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        kind,
        method,
        url,
        description: None,
        headers,
        auth,
        body,
        grpc: None,
        scripts: Scripts::default(),
        tests: Vec::new(),
        captures: Vec::new(),
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
        Some("oauth2") => match auth_param(a, "oauth2", "accessToken").filter(|t| !t.is_empty()) {
            Some(token) => {
                report.warnings.push(format!(
                    "{name}: OAuth 2.0 imported as the access token Postman had stored — Mándalo does not run the OAuth flow, so refresh the token yourself when it expires"
                ));
                Auth::Bearer { token }
            }
            None => {
                report.warnings.push(format!(
                    "{name}: OAuth 2.0 has no stored access token — imported without auth; paste a token into a bearer variable to send it"
                ));
                Auth::None
            }
        },
        Some(other) => {
            report.warnings.push(format!(
                "{name}: {other} auth is not supported — imported without auth, so the request goes out unauthenticated"
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

fn apply_path_variables(url: &str, spec: &Value, name: &str, report: &mut ImportReport) -> String {
    let Some(vars) = spec.get("variable").and_then(Value::as_array) else {
        return url.to_string();
    };
    let mut out = url.to_string();
    for var in vars {
        let Some(key) = var.get("key").and_then(Value::as_str) else {
            continue;
        };
        let value = var.get("value").and_then(Value::as_str).unwrap_or("");
        if value.is_empty() {
            report.warnings.push(format!(
                "{name}: path variable :{key} has no value in the export — it stays literal in the URL, set it before sending"
            ));
            continue;
        }
        out = out
            .split('/')
            .map(|segment| {
                if segment == format!(":{key}") {
                    value
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/");
    }
    out
}

fn convert_url(url: Option<&Value>, name: &str, report: &mut ImportReport) -> CoreResult<String> {
    let url = url.ok_or_else(|| CoreError::Parse(format!("{name}: request has no url")))?;
    if let Some(s) = url.as_str() {
        return Ok(s.to_string());
    }
    if !url.is_object() {
        return Err(CoreError::Unsupported(format!(
            "{name}: unsupported url shape: {url}"
        )));
    }
    if let Some(raw) = url.get("raw").and_then(Value::as_str) {
        return Ok(apply_path_variables(raw, url, name, report));
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
        return Err(CoreError::Parse(format!(
            "{name}: url object has neither raw nor host"
        )));
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
        Some(other) => {
            return Err(CoreError::Unsupported(format!(
                "{name}: unsupported url port shape: {other}"
            )))
        }
    }
    if let Some(segments) = url.get("path").and_then(Value::as_array) {
        for segment in segments {
            let text = segment
                .as_str()
                .or_else(|| segment.get("value").and_then(Value::as_str))
                .ok_or_else(|| {
                    CoreError::Unsupported(format!(
                        "{name}: unsupported url path segment: {segment}"
                    ))
                })?;
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
    Ok(apply_path_variables(&out, url, name, report))
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

fn import_environment(workspace: &Path, root: &Value) -> CoreResult<ImportReport> {
    let name = root
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::Parse("environment export has no name".to_string()))?;
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
        collections: 0,
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
                    "name": "Upload avatar",
                    "request": {
                        "method": "POST",
                        "url": "{{baseUrl}}/upload",
                        "body": {"mode": "formdata", "formdata": [
                            {"key": "caption", "value": "hola", "type": "text", "description": "shown under the photo"},
                            {"key": "avatar", "type": "file", "src": "/Users/ada/pic.png"},
                            {"key": "attachments", "type": "file", "src": ["/Users/ada/a.pdf", "/Users/ada/b.pdf"]},
                            {"key": "debug", "value": "1", "type": "text", "disabled": true}
                        ]}
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

    fn by_name<'a>(requests: &'a [(String, SavedRequest)], name: &str) -> &'a SavedRequest {
        &requests.iter().find(|(_, r)| r.name == name).unwrap().1
    }

    fn collect(
        workspace: &Path,
        slug: &str,
        folders: &[collection::FolderNode],
        summaries: &[collection::RequestSummary],
        out: &mut Vec<(String, SavedRequest)>,
    ) {
        for summary in summaries {
            let request = collection::load_request(workspace, slug, &summary.path).unwrap();
            out.push((summary.path.clone(), request));
        }
        for folder in folders {
            collect(workspace, slug, &folder.folders, &folder.requests, out);
        }
    }

    fn imported(workspace: &Path) -> Vec<(String, SavedRequest)> {
        let mut out = Vec::new();
        for node in collection::list_tree(workspace).unwrap().collections {
            collect(
                workspace,
                &node.slug,
                &node.folders,
                &node.requests,
                &mut out,
            );
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn path_of(requests: &[(String, SavedRequest)], name: &str) -> String {
        requests
            .iter()
            .find(|(_, r)| r.name == name)
            .unwrap()
            .0
            .clone()
    }

    #[test]
    fn imports_v21_collection_fully() {
        let dir = tempfile::tempdir().unwrap();
        let report = import(dir.path(), &collection_fixture()).unwrap();
        assert_eq!(report.imported, 7);
        assert_eq!(report.environments, 1);
        assert_eq!(
            report.skipped,
            vec![
                "Upload avatar: 4 form-data fields not imported — a .http file cannot express a multipart body, so the request was imported without one"
            ]
        );
        assert_eq!(
            report.warnings,
            vec![
                "Login: disabled form fields (debug) were dropped — a .http file writes the whole form body as one line of text",
                "Upload avatar: file field `avatar` needs a file inside the workspace — the export referenced /Users/ada/pic.png",
                "Upload avatar: file field `attachments` needs a file inside the workspace — the export referenced /Users/ada/a.pdf, /Users/ada/b.pdf",
                "Search: the API key moved into the URL query — a .http file writes an api key as a header or a query parameter, not as typed auth",
                "Signed: awsv4 auth is not supported — imported without auth, so the request goes out unauthenticated"
            ]
        );
        assert!(report.summary.contains("untouched"));

        let requests = imported(dir.path());
        assert_eq!(requests.len(), 7);

        let upload = by_name(&requests, "Upload avatar");
        assert_eq!(upload.body, Body::None);

        let list = by_name(&requests, "List");
        assert_eq!(list.method, "GET");
        assert_eq!(list.url, "{{baseUrl}}/users");
        assert_eq!(
            list.headers,
            vec![("Accept".to_string(), "application/json".to_string())]
        );
        assert_eq!(
            list.id, "users-admin-list-http-0",
            "the id a text request carries is its address, not the uuid the import minted"
        );

        let create = by_name(&requests, "Create");
        assert_eq!(create.body, Body::json("{\"name\": \"nova\"}"));
        assert_eq!(
            create.auth,
            Auth::Bearer {
                token: "{{token}}".to_string()
            }
        );

        let gql = by_name(&requests, "Gql");
        assert_eq!(gql.kind, "graphql");
        assert_eq!(gql.body, Body::graphql("{ ping }", "{\"limit\": 5}"));

        let login = by_name(&requests, "Login");
        assert_eq!(
            login.body,
            Body::Raw {
                language: RawLanguage::Text,
                text: "user=a&pass=b".to_string()
            }
        );
        assert_eq!(
            login.headers,
            vec![(
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string()
            )]
        );

        let search = by_name(&requests, "Search");
        assert_eq!(search.url, "https://api.x.dev/search?q=1&api_key=abc");
        assert_eq!(search.auth, Auth::None);

        assert_eq!(by_name(&requests, "Signed").auth, Auth::None);

        let envs = workspace::list_environments(dir.path()).unwrap().items;
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "Demo-API-vars");
        assert_eq!(envs[0].vars.get("baseUrl").unwrap(), "https://api.x.dev");
        assert!(!envs[0].vars.contains_key("old"));
    }

    #[test]
    fn folder_tree_becomes_real_directories() {
        let dir = tempfile::tempdir().unwrap();
        let report = import(dir.path(), &collection_fixture()).unwrap();
        assert_eq!(report.collections, 1);

        let collections = collection::list_collections(dir.path()).unwrap().items;
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].slug, "demo-api");
        assert_eq!(collections[0].name, "Demo API");

        let root = collection::collections_dir(dir.path()).join("demo-api");
        assert!(root.join("users").join("admin").join("list.http").is_file());

        let tree = collection::list_tree(dir.path()).unwrap();
        let node = &tree.collections[0];
        assert_eq!(node.folders.len(), 1);
        assert_eq!(node.folders[0].name, "users");
        assert_eq!(node.folders[0].path, "users");
        assert!(node.folders[0].requests.is_empty());
        let admin = &node.folders[0].folders[0];
        assert_eq!(admin.path, "users/admin");
        assert_eq!(admin.requests[0].name, "List");
        assert_eq!(admin.requests[0].path, "users/admin/list.http#0");

        let requests = imported(dir.path());
        assert_eq!(path_of(&requests, "Create"), "create.http#0");
        assert_eq!(path_of(&requests, "Gql"), "gql.http#0");
    }

    #[test]
    fn importing_twice_creates_a_second_collection() {
        let dir = tempfile::tempdir().unwrap();
        import(dir.path(), &collection_fixture()).unwrap();
        import(dir.path(), &collection_fixture()).unwrap();
        let slugs: Vec<String> = collection::list_collections(dir.path())
            .unwrap()
            .items
            .into_iter()
            .map(|c| c.slug)
            .collect();
        assert_eq!(slugs, vec!["demo-api", "demo-api-2"]);
    }

    #[test]
    fn collection_and_folder_scripts_are_inlined_into_every_request_below_them() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Scripted",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "event": [{
                "listen": "prerequest",
                "script": {"type": "text/javascript", "exec": ["console.log('collection')"]}
            }],
            "item": [
                {
                    "name": "Folder",
                    "event": [{
                        "listen": "test",
                        "script": {"exec": ["console.log('folder')"]}
                    }],
                    "item": [{
                        "name": "Login",
                        "description": "gets a token",
                        "request": {"method": "POST", "url": "https://x.dev/login"},
                        "event": [
                            {
                                "listen": "prerequest",
                                "script": {"exec": ["const a = 1;", "console.log(a);"]}
                            },
                            {
                                "listen": "test",
                                "script": {"exec": "pm.test('ok', () => {});"}
                            }
                        ]
                    }]
                },
                {
                    "name": "Plain",
                    "request": {"method": "GET", "url": "https://x.dev/plain"},
                    "event": [{"listen": "test", "script": {"exec": ["   "]}}]
                }
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(
            report.warnings,
            vec![
                "Login: the description was dropped — neither .http nor .grpc has a line for one; keep it in a `#` comment above the request",
                "Folder: test script copied into every request below it (Login) — Mándalo has no shared scripts, so edit each copy",
                "Scripted: pre-request script copied into every request below it (Login, Plain) — Mándalo has no shared scripts, so edit each copy"
            ]
        );
        let requests = imported(dir.path());
        let login = by_name(&requests, "Login");
        assert_eq!(
            login.scripts.pre.as_deref(),
            Some(
                "// inlined by the Postman import: collection \"Scripted\" pre-request script\nconsole.log('collection')\n\nconst a = 1;\nconsole.log(a);"
            )
        );
        assert_eq!(
            login.scripts.post.as_deref(),
            Some(
                "// inlined by the Postman import: folder \"Folder\" test script\nconsole.log('folder')\n\npm.test('ok', () => {});"
            )
        );
        assert_eq!(login.description, None);

        let plain = by_name(&requests, "Plain");
        assert_eq!(
            plain.scripts.pre.as_deref(),
            Some(
                "// inlined by the Postman import: collection \"Scripted\" pre-request script\nconsole.log('collection')"
            )
        );
        assert_eq!(plain.scripts.post, None);
        assert_eq!(
            collection::load_request(dir.path(), "scripted", "folder/login.http#0")
                .unwrap()
                .scripts,
            login.scripts
        );
    }

    #[test]
    fn a_folder_script_with_no_requests_says_it_was_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Empty",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Nothing",
                "event": [{"listen": "prerequest", "script": {"exec": ["console.log(1)"]}}],
                "item": []
            }]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(
            report.warnings,
            vec![
                "Nothing: pre-request script dropped — it contains no requests to run them against"
            ]
        );
    }

    #[test]
    fn scripts_that_use_unsupported_apis_are_named_at_import_time() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Risky",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Login",
                "request": {"method": "GET", "url": "https://x.dev/a"},
                "event": [
                    {"listen": "prerequest", "script": {"exec": ["pm.sendRequest('https://x.dev', function(){});"]}},
                    {"listen": "test", "script": {"exec": ["postman.setNextRequest('Next');"]}}
                ]
            }]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(report.imported, 1);
        assert!(report.warnings[0].starts_with("Login: script uses pm.sendRequest"));
        assert!(report.warnings[0].contains("cannot make network requests"));
        assert!(report.warnings[1].contains("setNextRequest"));
    }

    #[test]
    fn folder_level_auth_overrides_the_collection_default() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Scoped",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}"}]},
            "item": [
                {
                    "name": "Admin",
                    "auth": {"type": "basic", "basic": [
                        {"key": "username", "value": "u"},
                        {"key": "password", "value": "p"}
                    ]},
                    "item": [{"name": "Inner", "request": {"method": "GET", "url": "https://x.dev/a"}}]
                },
                {"name": "Outer", "request": {"method": "GET", "url": "https://x.dev/b"}}
            ]
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(
            by_name(&requests, "Inner").auth,
            Auth::Basic {
                username: "u".to_string(),
                password: "p".to_string()
            }
        );
        assert_eq!(
            by_name(&requests, "Outer").auth,
            Auth::Bearer {
                token: "{{token}}".to_string()
            }
        );
    }

    #[test]
    fn oauth2_carries_the_stored_access_token_as_a_bearer() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Oauth",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [
                {
                    "name": "WithToken",
                    "request": {
                        "method": "GET",
                        "url": "https://x.dev/a",
                        "auth": {"type": "oauth2", "oauth2": [
                            {"key": "accessToken", "value": "abc123"},
                            {"key": "addTokenTo", "value": "header"}
                        ]}
                    }
                },
                {
                    "name": "WithoutToken",
                    "request": {
                        "method": "GET",
                        "url": "https://x.dev/b",
                        "auth": {"type": "oauth2", "oauth2": [{"key": "grant_type", "value": "authorization_code"}]}
                    }
                }
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(
            by_name(&requests, "WithToken").auth,
            Auth::Bearer {
                token: "abc123".to_string()
            }
        );
        assert_eq!(by_name(&requests, "WithoutToken").auth, Auth::None);
        assert!(report.warnings[0].contains("does not run the OAuth flow"));
        assert!(report.warnings[1].contains("no stored access token"));
    }

    #[test]
    fn a_request_written_as_a_bare_url_string_still_imports() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Shorthand",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{"name": "Ping", "request": "https://x.dev/ping"}]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        assert_eq!(report.imported, 1);
        let requests = imported(dir.path());
        assert_eq!(by_name(&requests, "Ping").url, "https://x.dev/ping");
        assert_eq!(by_name(&requests, "Ping").method, "GET");
    }

    #[test]
    fn path_variables_are_substituted_and_empty_ones_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Paths",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [
                {
                    "name": "Filled",
                    "request": {
                        "method": "GET",
                        "url": {
                            "raw": "{{baseUrl}}/users/:userId/posts/:postId",
                            "variable": [
                                {"key": "userId", "value": "{{userId}}"},
                                {"key": "postId", "value": "42"}
                            ]
                        }
                    }
                },
                {
                    "name": "Empty",
                    "request": {
                        "method": "GET",
                        "url": {
                            "raw": "{{baseUrl}}/orders/:orderId",
                            "variable": [{"key": "orderId", "value": ""}]
                        }
                    }
                }
            ]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(
            by_name(&requests, "Filled").url,
            "{{baseUrl}}/users/{{userId}}/posts/42"
        );
        assert_eq!(
            by_name(&requests, "Empty").url,
            "{{baseUrl}}/orders/:orderId"
        );
        assert_eq!(
            report.warnings,
            vec!["Empty: path variable :orderId has no value in the export — it stays literal in the URL, set it before sending"]
        );
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
        let err = import(dir.path(), &json).unwrap_err().to_string();
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
        let requests = imported(dir.path());
        assert_eq!(
            requests[0].1.url,
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
        let requests = imported(dir.path());
        assert_eq!(requests[0].1.url, "https://api.x.dev:8443?token={{token}}");
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
        let requests = imported(dir.path());
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
            vec!["collection: awsv4 auth is not supported — imported without auth, so the request goes out unauthenticated"]
        );
        let requests = imported(dir.path());
        assert_eq!(by_name(&requests, "A").auth, Auth::None);
        assert_eq!(by_name(&requests, "B").auth, Auth::None);
    }

    #[test]
    fn a_multi_file_form_field_is_reported_by_name_not_imported() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Uploads",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Attach",
                "request": {
                    "method": "POST",
                    "url": "https://x.dev/upload",
                    "body": {"mode": "formdata", "formdata": [
                        {"key": "attachments", "type": "file", "src": ["/a/one.pdf", "/a/two.pdf"]},
                        {"key": "avatar", "type": "file", "src": "/a/pic.png"},
                        {"key": "empty", "type": "file"}
                    ]}
                }
            }]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(by_name(&requests, "Attach").body, Body::None);
        assert_eq!(
            report.warnings,
            vec![
                "Attach: file field `attachments` needs a file inside the workspace — the export referenced /a/one.pdf, /a/two.pdf",
                "Attach: file field `avatar` needs a file inside the workspace — the export referenced /a/pic.png",
                "Attach: file field `empty` needs a file inside the workspace — the export referenced no path at all"
            ]
        );
        assert_eq!(
            report.skipped,
            vec![
                "Attach: 3 form-data fields not imported — a .http file cannot express a multipart body, so the request was imported without one"
            ]
        );
    }

    #[test]
    fn a_raw_body_keeps_the_language_postman_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Languages",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [
                {
                    "name": "Xml",
                    "request": {
                        "method": "POST",
                        "url": "https://x.dev/a",
                        "body": {
                            "mode": "raw",
                            "raw": "hola",
                            "options": {"raw": {"language": "xml"}}
                        }
                    }
                },
                {
                    "name": "Unlabelled",
                    "request": {
                        "method": "POST",
                        "url": "https://x.dev/b",
                        "body": {"mode": "raw", "raw": "{\"a\": 1}"}
                    }
                }
            ]
        })
        .to_string();
        import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(
            by_name(&requests, "Xml").body,
            Body::Raw {
                language: RawLanguage::Xml,
                text: "hola".to_string()
            }
        );
        assert_eq!(
            by_name(&requests, "Unlabelled").body,
            Body::Raw {
                language: RawLanguage::Json,
                text: "{\"a\": 1}".to_string()
            }
        );
    }

    #[test]
    fn a_binary_body_is_not_imported_and_asks_for_a_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "info": {
                "name": "Blobs",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "item": [{
                "name": "Blob",
                "request": {
                    "method": "PUT",
                    "url": "https://x.dev/blob",
                    "body": {"mode": "file", "file": {"src": "/Users/dev/blob.bin"}}
                }
            }]
        })
        .to_string();
        let report = import(dir.path(), &json).unwrap();
        let requests = imported(dir.path());
        assert_eq!(by_name(&requests, "Blob").body, Body::None);
        assert_eq!(
            report.warnings,
            vec!["Blob: the binary body needs a file inside the workspace — the export referenced /Users/dev/blob.bin; imported without a body, so write `< ./your-file` under the request"]
        );
    }

    #[test]
    fn unrecognized_export_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let err = import(dir.path(), "{\"foo\": 1}").unwrap_err().to_string();
        assert!(err.contains("unrecognized Postman export"));
    }
}
