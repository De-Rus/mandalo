use crate::assertions::Scripts;
use crate::body::{Body, FormDataRow, RawLanguage};
use crate::collection::{self, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::postman::{self, ImportReport};
use crate::request::Auth;
use crate::workspace::{self, EnvDoc, VarDef};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_NODES: usize = 400_000;
const MAX_OPERATIONS: usize = 2_000;
const MAX_REF_HOPS: usize = 32;
const MAX_EXAMPLE_DEPTH: usize = 8;
const MAX_EXAMPLE_NODES: usize = 2_000;
const MAX_LISTED: usize = 5;
const MAX_BODY_EXAMPLES: usize = 10;
const MAX_RESPONSE_EXAMPLE_LINES: usize = 14;
const MAX_RESPONSE_EXAMPLE_CHARS: usize = 700;

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Braces stay literal so `{{variable}}` templates survive into the URL.
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Version {
    V30,
    V31,
    Swagger20,
}

impl Version {
    fn label(self) -> &'static str {
        match self {
            Version::V30 => "OpenAPI 3.0",
            Version::V31 => "OpenAPI 3.1",
            Version::Swagger20 => "Swagger 2.0",
        }
    }
}

/// Which importer a file belongs to, decided by what it declares rather than by
/// its extension.
pub fn looks_like_openapi(source: &str) -> bool {
    let json = |key: &str| {
        source
            .split(key)
            .skip(1)
            .any(|rest| rest.trim_start().starts_with(':'))
    };
    json("\"openapi\"")
        || json("\"swagger\"")
        || source
            .lines()
            .any(|l| l.starts_with("openapi:") || l.starts_with("swagger:"))
}

const ROOT_OPENAPI_NAMES: &[&str] = &[
    "openapi.json",
    "openapi.yaml",
    "openapi.yml",
    "swagger.json",
    "swagger.yaml",
    "swagger.yml",
];

fn folder_has_requests(folder: &collection::FolderNode) -> bool {
    !folder.requests.is_empty() || folder.folders.iter().any(folder_has_requests)
}

fn collection_has_requests(node: &collection::CollectionNode) -> bool {
    !node.requests.is_empty() || node.folders.iter().any(folder_has_requests)
}

fn tree_has_requests(tree: &collection::Tree) -> bool {
    tree.collections.iter().any(collection_has_requests)
}

fn find_root_openapi(workspace: &Path) -> Option<std::path::PathBuf> {
    for name in ROOT_OPENAPI_NAMES {
        let path = workspace.join(name);
        if !path.is_file() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        if looks_like_openapi(&raw) {
            return Some(path);
        }
    }
    None
}

/// True when the folder ships a root OpenAPI/Swagger document (e.g. `examples/petstore`).
pub fn has_root_openapi(workspace: &Path) -> bool {
    find_root_openapi(workspace).is_some()
}

/// When a folder is opened as a workspace and still has no requests, but ships
/// an OpenAPI/Swagger file at the root (as `examples/petstore` does), import it
/// instead of leaving an empty starter collection.
pub fn seed_if_empty(workspace: &Path) -> CoreResult<Option<ImportReport>> {
    let tree = collection::list_tree(workspace)?;
    if tree_has_requests(&tree) {
        return Ok(None);
    }
    let Some(spec) = find_root_openapi(workspace) else {
        return Ok(None);
    };
    for collection in &tree.collections {
        if !collection_has_requests(collection) {
            collection::delete_collection(workspace, &collection.slug)?;
        }
    }
    let source = std::fs::read_to_string(&spec).map_err(|e| CoreError::io(spec.display(), e))?;
    Ok(Some(import(workspace, &source)?))
}

pub fn import(workspace: &Path, source: &str) -> CoreResult<ImportReport> {
    let root = parse_document(source)?;
    let version = detect_version(&root)?;
    refuse_foreign_refs(&root)?;
    Import::new(root, version).run(workspace)
}

fn parse_document(source: &str) -> CoreResult<Value> {
    if source.len() > MAX_DOCUMENT_BYTES {
        return Err(CoreError::Unsupported(format!(
            "this API specification is {} bytes, over the {MAX_DOCUMENT_BYTES}-byte import limit — split it or import the part you need",
            source.len()
        )));
    }
    let trimmed = source.trim_start();
    let root: Value = if trimmed.starts_with('{') {
        serde_json::from_str(source).map_err(|e| CoreError::Parse(format!("invalid JSON: {e}")))?
    } else {
        refuse_yaml_anchors(source)?;
        serde_norway::from_str(source)
            .map_err(|e| CoreError::Parse(format!("invalid YAML: {e}")))?
    };
    let mut budget = MAX_DOCUMENT_NODES;
    if !fits(&root, &mut budget) {
        return Err(CoreError::Unsupported(format!(
            "this API specification expands to more than {MAX_DOCUMENT_NODES} nodes — split it or import the part you need"
        )));
    }
    Ok(root)
}

fn fits(value: &Value, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    match value {
        Value::Array(items) => items.iter().all(|v| fits(v, budget)),
        Value::Object(map) => map.values().all(|v| fits(v, budget)),
        _ => true,
    }
}

/// A YAML anchor is how a one-kilobyte file expands into gigabytes, and the
/// expansion happens inside the parser where no budget can see it. No OpenAPI
/// generator emits anchors, so refusing them costs nothing and closes the hole.
fn refuse_yaml_anchors(source: &str) -> CoreResult<()> {
    static ANCHOR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let anchor = ANCHOR.get_or_init(|| {
        regex::Regex::new(r"(?m)(?::|^\s*-)[ \t]+&[A-Za-z0-9_][A-Za-z0-9_.\-]*[ \t]*$")
            .expect("valid")
    });
    match anchor.find(source) {
        None => Ok(()),
        Some(found) => Err(CoreError::Unsupported(format!(
            "this YAML uses a node anchor ({}) — Mándalo does not expand YAML anchors, because a chain of them turns a small file into an unbounded one; run the spec through a bundler that resolves them and import the result",
            found.as_str().trim().trim_start_matches([':', '-']).trim()
        ))),
    }
}

fn detect_version(root: &Value) -> CoreResult<Version> {
    if let Some(declared) = root.get("openapi").and_then(Value::as_str) {
        if declared.starts_with("3.1") {
            return Ok(Version::V31);
        }
        if declared.starts_with("3.0") {
            return Ok(Version::V30);
        }
        return Err(CoreError::Unsupported(format!(
            "unsupported OpenAPI version: {declared} — Mándalo imports OpenAPI 3.0, OpenAPI 3.1 and Swagger 2.0"
        )));
    }
    if let Some(declared) = root.get("swagger").and_then(Value::as_str) {
        if declared.starts_with("2.0") {
            return Ok(Version::Swagger20);
        }
        return Err(CoreError::Unsupported(format!(
            "unsupported Swagger version: {declared} — Mándalo imports Swagger 2.0, OpenAPI 3.0 and OpenAPI 3.1; convert this document first"
        )));
    }
    Err(CoreError::Unsupported(
        "this file is not an API specification: it has no top-level `openapi` (3.0/3.1) or `swagger` (2.0) version field".to_string(),
    ))
}

/// Importing a file must not make a network call, so a `$ref` that points at
/// another document is a hard stop rather than a fetch.
fn refuse_foreign_refs(root: &Value) -> CoreResult<()> {
    let mut found: BTreeSet<String> = BTreeSet::new();
    collect_foreign_refs(root, &mut found);
    if found.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = found.iter().map(|r| format!("{r:?}")).collect();
    Err(CoreError::Unsupported(format!(
        "this specification references another document: $ref {}. Mándalo never fetches a reference while importing — bundle the spec into a single self-contained file (redocly bundle, swagger-cli bundle) and import that",
        listed(&names)
    )))
}

fn collect_foreign_refs(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect_foreign_refs(v, found)),
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                if !reference.starts_with('#') {
                    found.insert(reference.to_string());
                }
            }
            map.values().for_each(|v| collect_foreign_refs(v, found));
        }
        _ => {}
    }
}

fn pointer<'a>(root: &'a Value, reference: &str) -> CoreResult<&'a Value> {
    let path = reference.strip_prefix("#/").ok_or_else(|| {
        CoreError::Unsupported(format!(
            "$ref {reference:?} is not a pointer into this document — Mándalo only resolves local `#/…` references"
        ))
    })?;
    let mut current = root;
    for raw in path.split('/') {
        let token = raw.replace("~1", "/").replace("~0", "~");
        current = current.get(&token).ok_or_else(|| {
            CoreError::NotFound(format!(
                "$ref {reference:?} points at nothing in this document"
            ))
        })?;
    }
    Ok(current)
}

/// Follows `$ref` to the value it names. The result is owned so that a caller
/// holding it is not still borrowing the document it walked out of.
fn resolved(root: &Value, value: &Value) -> CoreResult<Value> {
    let mut current = value;
    for _ in 0..MAX_REF_HOPS {
        let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
            return Ok(current.clone());
        };
        current = pointer(root, reference)?;
    }
    Err(CoreError::Unsupported(format!(
        "a $ref chain in this document is deeper than {MAX_REF_HOPS} hops — it points at itself"
    )))
}

fn as_array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn as_object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn sanitize_var_name(name: &str) -> String {
    let out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "value".to_string()
    } else {
        out
    }
}

fn listed(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(MAX_LISTED).map(String::as_str).collect();
    let rest = items.len().saturating_sub(shown.len());
    match rest {
        0 => shown.join(", "),
        n => format!("{} and {n} more", shown.join(", ")),
    }
}

struct Server {
    url: String,
    label: String,
    variables: BTreeMap<String, String>,
}

#[derive(Clone)]
struct Scheme {
    name: String,
    kind: String,
    scheme: String,
    key: String,
    placement: String,
    flows: Vec<String>,
    connect_url: Option<String>,
}

struct Parameter {
    name: String,
    place: String,
    required: bool,
    description: Option<String>,
    seed: String,
    templated: bool,
    /// The named examples that did not become the seed. An environment holds one
    /// value per variable, so the rest are written where a reader can copy them.
    alternatives: Vec<String>,
}

/// One request the operation produces. A spec that names several request-body
/// examples produces one of these per example; everything else produces one.
struct BodyVariant {
    label: Option<String>,
    summary: Option<String>,
    body: Body,
    files: Option<String>,
}

impl BodyVariant {
    fn plain(body: Body) -> BodyVariant {
        BodyVariant {
            label: None,
            summary: None,
            body,
            files: None,
        }
    }
}

/// Everything about an operation that has no line of its own in a `.http` file
/// and therefore lands in the `#` comment block above the request.
#[derive(Default)]
struct Notes {
    optional: Vec<String>,
    alternatives: Vec<String>,
    body: Option<String>,
    files: Option<String>,
    response: Option<String>,
}

/// The example generator's verdict on one schema: the value, plus the reason it
/// stopped early if it did.
struct Generated {
    value: Value,
    truncated: Option<String>,
    branched: Option<String>,
}

struct Example<'a> {
    root: &'a Value,
    path: Vec<String>,
    budget: usize,
    truncated: Option<String>,
    branched: Option<String>,
}

impl<'a> Example<'a> {
    fn new(root: &'a Value) -> Self {
        Example {
            root,
            path: Vec::new(),
            budget: MAX_EXAMPLE_NODES,
            truncated: None,
            branched: None,
        }
    }

    fn generate(root: &'a Value, schema: &'a Value) -> Generated {
        let mut walk = Example::new(root);
        let value = walk.value_of(schema, 0);
        Generated {
            value,
            truncated: walk.truncated,
            branched: walk.branched,
        }
    }

    fn stop(&mut self, what: &str) -> Value {
        if self.truncated.is_none() {
            self.truncated = Some(what.to_string());
        }
        Value::Null
    }

    /// A `$ref` is followed unless it is already on the path (a type that
    /// contains itself), the tree is deeper than `MAX_EXAMPLE_DEPTH`, or the node
    /// budget is spent. In all three cases the branch ends in `null` and the
    /// report says where — a truncated skeleton beats a hang.
    fn value_of(&mut self, schema: &'a Value, depth: usize) -> Value {
        if depth >= MAX_EXAMPLE_DEPTH {
            return self.stop("the schema nests deeper than this importer follows");
        }
        if self.budget == 0 {
            return self.stop("the example grew past the node budget");
        }
        self.budget -= 1;

        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let name = reference
                .rsplit('/')
                .next()
                .unwrap_or(reference)
                .to_string();
            if self.path.iter().any(|seen| seen == &name) {
                return self.stop(&format!("the schema {name} contains itself"));
            }
            let Ok(target) = pointer(self.root, reference) else {
                return self.stop(&format!("the schema {name} could not be resolved"));
            };
            self.path.push(name);
            let out = self.value_of(target, depth);
            self.path.pop();
            return out;
        }

        let Some(map) = schema.as_object() else {
            return Value::Null;
        };
        for field in ["example", "default", "const"] {
            if let Some(given) = map.get(field) {
                return given.clone();
            }
        }
        if let Some(first) = as_array(map.get("examples")).first() {
            return first.clone();
        }
        if let Some(first) = as_array(map.get("enum")).first() {
            return first.clone();
        }
        if let Some(arms) = map.get("allOf").and_then(Value::as_array) {
            return self.compose(arms, depth);
        }
        for keyword in ["oneOf", "anyOf"] {
            let arms = as_array(map.get(keyword));
            if let Some(first) = arms.first() {
                if arms.len() > 1 && self.branched.is_none() {
                    self.branched = Some(format!(
                        "the body follows the first of {} `{keyword}` arms",
                        arms.len()
                    ));
                }
                return self.value_of(first, depth);
            }
        }
        match primary_type(schema).as_deref() {
            Some("array") => self.array_of(map, depth),
            Some("object") => self.object_of(map, depth),
            Some("string") => Value::String(string_shape(map)),
            Some("integer") => Value::from(0),
            Some("number") => Value::from(0.0),
            Some("boolean") => Value::Bool(true),
            Some("null") => Value::Null,
            _ if map.contains_key("properties") => self.object_of(map, depth),
            _ if map.contains_key("items") => self.array_of(map, depth),
            _ => Value::Null,
        }
    }

    fn array_of(&mut self, map: &'a Map<String, Value>, depth: usize) -> Value {
        let Some(items) = map.get("items") else {
            return Value::Array(Vec::new());
        };
        Value::Array(vec![self.value_of(items, depth + 1)])
    }

    fn object_of(&mut self, map: &'a Map<String, Value>, depth: usize) -> Value {
        let Some(properties) = as_object(map.get("properties")) else {
            return Value::Object(Map::new());
        };
        let mut out = Map::new();
        for (name, schema) in properties {
            out.insert(name.clone(), self.value_of(schema, depth + 1));
        }
        Value::Object(out)
    }

    /// `allOf` is a composition, so every arm contributes. Object arms merge key
    /// by key, later arms winning; anything else falls back to the first arm that
    /// produced a value at all.
    fn compose(&mut self, arms: &'a [Value], depth: usize) -> Value {
        let mut merged = Map::new();
        let mut fallback = Value::Null;
        for arm in arms {
            match self.value_of(arm, depth) {
                Value::Object(map) => merged.extend(map),
                other if fallback.is_null() => fallback = other,
                _ => {}
            }
        }
        if merged.is_empty() {
            return fallback;
        }
        Value::Object(merged)
    }
}

fn is_multipart_form_data(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
}

/// A property that carries bytes: `format: binary` is how 3.0 spells it,
/// `contentMediaType` how 3.1 does, and an array of either is one field holding
/// several files.
fn is_file_property(schema: &Value) -> bool {
    if schema.get("contentMediaType").is_some() {
        return true;
    }
    if schema.get("format").and_then(Value::as_str) == Some("binary") {
        return true;
    }
    schema.get("items").is_some_and(is_file_property)
}

fn primary_type(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(String::from),
        _ => None,
    }
}

fn string_shape(map: &Map<String, Value>) -> String {
    match map.get("format").and_then(Value::as_str) {
        Some("date-time") => "2026-01-01T00:00:00Z",
        Some("date") => "2026-01-01",
        Some("time") => "00:00:00Z",
        Some("uuid") => "00000000-0000-0000-0000-000000000000",
        Some("email" | "idn-email") => "user@example.com",
        Some("hostname" | "idn-hostname") => "example.com",
        Some("uri" | "url" | "uri-reference") => "https://example.com",
        Some("ipv4") => "127.0.0.1",
        Some("ipv6") => "::1",
        Some("byte") => "aGVsbG8=",
        Some("password") => "",
        Some("binary") => "",
        _ => "string",
    }
    .to_string()
}

struct Import {
    root: Value,
    version: Version,
    report: ImportReport,
    seeds: BTreeMap<String, String>,
    secrets: BTreeSet<String>,
}

impl Import {
    fn new(root: Value, version: Version) -> Self {
        Import {
            root,
            version,
            report: ImportReport {
                imported: 0,
                collections: 0,
                environments: 0,
                skipped: Vec::new(),
                warnings: Vec::new(),
                summary: String::new(),
            },
            seeds: BTreeMap::new(),
            secrets: BTreeSet::new(),
        }
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.report.warnings.push(message.into());
    }

    fn title(&self) -> String {
        text(self.root.pointer("/info/title"))
            .unwrap_or("openapi-import")
            .to_string()
    }

    fn schemes(&self) -> BTreeMap<String, Scheme> {
        let raw = match self.version {
            Version::Swagger20 => as_object(self.root.get("securityDefinitions")),
            _ => as_object(self.root.pointer("/components/securitySchemes")),
        };
        let mut out = BTreeMap::new();
        for (name, declared) in raw.into_iter().flatten() {
            let Ok(definition) = resolved(&self.root, declared) else {
                continue;
            };
            let kind = definition
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // Swagger 2.0 spells HTTP basic as its own type; 3.x spells it as
            // `http` with `scheme: basic`. One shape reaches the mapper.
            let (kind, scheme) = match kind.as_str() {
                "basic" => ("http".to_string(), "basic".to_string()),
                _ => (
                    kind,
                    definition
                        .get("scheme")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                ),
            };
            out.insert(
                name.clone(),
                Scheme {
                    name: name.clone(),
                    kind,
                    scheme,
                    key: definition
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(name)
                        .to_string(),
                    placement: definition
                        .get("in")
                        .and_then(Value::as_str)
                        .unwrap_or("header")
                        .to_string(),
                    flows: as_object(definition.get("flows"))
                        .map(|f| f.keys().cloned().collect())
                        .unwrap_or_else(|| {
                            definition
                                .get("flow")
                                .and_then(Value::as_str)
                                .map(|f| vec![f.to_string()])
                                .unwrap_or_default()
                        }),
                    connect_url: definition
                        .get("openIdConnectUrl")
                        .and_then(Value::as_str)
                        .map(String::from),
                },
            );
        }
        out
    }

    fn servers(&mut self) -> Vec<Server> {
        let declared: Vec<Server> = match self.version {
            Version::Swagger20 => self.swagger_servers(),
            _ => as_array(self.root.get("servers"))
                .iter()
                .filter_map(|s| self.server_from(s))
                .collect(),
        };
        if !declared.is_empty() {
            return declared;
        }
        self.warn(
            "the specification declares no server URL — the environment carries an empty `baseUrl`, so set it before sending",
        );
        vec![Server {
            url: String::new(),
            label: "default".to_string(),
            variables: BTreeMap::new(),
        }]
    }

    fn swagger_servers(&self) -> Vec<Server> {
        let Some(host) = text(self.root.get("host")) else {
            return Vec::new();
        };
        let base = self
            .root
            .get("basePath")
            .and_then(Value::as_str)
            .unwrap_or("");
        let schemes: Vec<&str> = as_array(self.root.get("schemes"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let schemes = if schemes.is_empty() {
            vec!["https"]
        } else {
            schemes
        };
        schemes
            .into_iter()
            .map(|scheme| Server {
                url: format!("{scheme}://{host}{base}")
                    .trim_end_matches('/')
                    .to_string(),
                label: format!("{scheme}-{host}"),
                variables: BTreeMap::new(),
            })
            .collect()
    }

    fn server_from(&self, declared: &Value) -> Option<Server> {
        let raw = text(declared.get("url"))?;
        let mut variables = BTreeMap::new();
        let mut url = raw.to_string();
        for (name, definition) in as_object(declared.get("variables")).into_iter().flatten() {
            if !valid_var_name(name) {
                continue;
            }
            let value = definition
                .get("default")
                .map(scalar_text)
                .or_else(|| as_array(definition.get("enum")).first().map(scalar_text))
                .unwrap_or_default();
            url = url.replace(&format!("{{{name}}}"), &format!("{{{{{name}}}}}"));
            variables.insert(name.clone(), value);
        }
        let label = text(declared.get("description"))
            .map(String::from)
            .unwrap_or_else(|| {
                raw.split_once("://")
                    .map(|(_, rest)| rest)
                    .unwrap_or(raw)
                    .trim_end_matches('/')
                    .to_string()
            });
        Some(Server {
            url: url.trim_end_matches('/').to_string(),
            label,
            variables,
        })
    }

    fn run(mut self, workspace: &Path) -> CoreResult<ImportReport> {
        let title = self.title();
        let schemes = self.schemes();
        let servers = self.servers();
        let collection = collection::create_collection(workspace, &title)?;
        self.report.collections = 1;

        let global = security_from(self.root.get("security"), &schemes);
        self.walk_paths(workspace, &collection.slug, &schemes, &global)?;
        self.note_webhooks();

        let mut environments = Vec::new();
        for server in &servers {
            let name =
                workspace::sanitize_env_name(workspace, &format!("{title}-{}", server.label));
            let mut doc = EnvDoc::new(name.clone());
            doc.vars
                .insert("baseUrl".to_string(), VarDef::shared(server.url.clone()));
            for (key, value) in &server.variables {
                doc.vars.insert(key.clone(), VarDef::shared(value.clone()));
            }
            for (key, value) in &self.seeds {
                doc.vars
                    .entry(key.clone())
                    .or_insert_with(|| VarDef::shared(value.clone()));
            }
            for key in &self.secrets {
                doc.vars.insert(key.clone(), VarDef::secret(&[]));
            }
            workspace::save_env_doc(workspace, &doc)?;
            environments.push(name);
            self.report.environments += 1;
        }
        if environments.len() > 1 {
            self.warn(format!(
                "the specification lists {} servers, so each one became its own environment ({}) — pick one when you run",
                environments.len(),
                listed(&environments)
            ));
        }
        if !self.secrets.is_empty() {
            let names: Vec<String> = self.secrets.iter().cloned().collect();
            self.warn(format!(
                "auth is declared but has no value here: set {} with `mandalo env set` — until then those requests go out unauthenticated",
                listed(&names)
            ));
        }
        self.report.summary = format!(
            "{} imported into the collection {:?}. Path and required query parameters became {{{{variables}}}} seeded in the environment; importing the same spec again creates a new collection and never touches this one.",
            self.version.label(),
            collection.slug
        );
        Ok(self.report)
    }

    fn note_webhooks(&mut self) {
        let count = as_object(self.root.get("webhooks"))
            .map(Map::len)
            .unwrap_or(0);
        if count > 0 {
            self.warn(format!(
                "{count} webhook definitions were not imported — a webhook describes a request the API sends to you, so there is nothing here to send"
            ));
        }
    }

    fn walk_paths(
        &mut self,
        workspace: &Path,
        slug: &str,
        schemes: &BTreeMap<String, Scheme>,
        global: &Security,
    ) -> CoreResult<()> {
        let paths = as_object(self.root.get("paths"))
            .cloned()
            .unwrap_or_default();
        let mut folders: BTreeMap<String, String> = BTreeMap::new();
        for (path, declared) in &paths {
            let Ok(item) = resolved(&self.root, declared) else {
                self.report
                    .skipped
                    .push(format!("{path}: the path item could not be resolved"));
                continue;
            };
            let shared = as_array(item.get("parameters")).to_vec();
            for method in METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                if self.report.imported >= MAX_OPERATIONS {
                    self.report.skipped.push(format!(
                        "{} {path}: the specification declares more than {MAX_OPERATIONS} operations, which is the import limit — split it and import the rest separately",
                        method.to_uppercase()
                    ));
                    return Ok(());
                }
                if !self.import_operation(
                    workspace,
                    slug,
                    &mut folders,
                    schemes,
                    global,
                    path,
                    method,
                    operation,
                    &shared,
                )? {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn import_operation(
        &mut self,
        workspace: &Path,
        slug: &str,
        folders: &mut BTreeMap<String, String>,
        schemes: &BTreeMap<String, Scheme>,
        global: &Security,
        path: &str,
        method: &str,
        operation: &Value,
        shared: &[Value],
    ) -> CoreResult<bool> {
        let name = text(operation.get("operationId"))
            .or_else(|| text(operation.get("summary")))
            .map(|n| n.replace(['\r', '\n'], " ").trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("{} {path}", method.to_uppercase()));

        let mut parameters = Vec::new();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for declared in operation
            .get("parameters")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .chain(shared)
        {
            let Ok(declared) = resolved(&self.root, declared) else {
                self.warn(format!(
                    "{name}: a parameter reference could not be resolved"
                ));
                continue;
            };
            let Some(parameter) = self.parameter_from(&declared) else {
                continue;
            };
            if seen.insert((parameter.name.clone(), parameter.place.clone())) {
                parameters.push(parameter);
            }
        }

        let mut url = format!(
            "{{{{baseUrl}}}}{}",
            self.templated_path(&name, path, &parameters)
        );
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut query: Vec<String> = Vec::new();
        let mut cookies: Vec<String> = Vec::new();
        let mut optional: Vec<String> = Vec::new();

        for parameter in &parameters {
            let value = match parameter.templated {
                true => format!("{{{{{}}}}}", parameter.name),
                false => parameter.seed.clone(),
            };
            let note = match &parameter.description {
                Some(d) => format!("  {}={value} — {d}", parameter.name),
                None => format!("  {}={value}", parameter.name),
            };
            match (parameter.place.as_str(), parameter.required) {
                ("path", _) => {}
                ("query", true) => query.push(format!(
                    "{}={}",
                    encode_query_component(&parameter.name),
                    encode_query_component(&value)
                )),
                ("header", true) => headers.push((parameter.name.clone(), value)),
                ("cookie", true) => cookies.push(format!("{}={value}", parameter.name)),
                (_, false) => optional.push(note),
                (other, _) => self.warn(format!(
                    "{name}: parameter {:?} is declared `in: {other}`, which is not a place a request can put it — it was not imported",
                    parameter.name
                )),
            }
        }
        if !cookies.is_empty() {
            headers.push(("Cookie".to_string(), cookies.join("; ")));
        }

        let auth = self.auth_for(&name, operation, schemes, global, &mut headers, &mut query);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query.join("&"));
        }

        let variants = self.body_for(&name, operation, &mut headers);

        let mut tags: Vec<String> = as_array(operation.get("tags"))
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        tags.retain(|t| !t.trim().is_empty());
        let folder = match tags.first() {
            None => String::new(),
            Some(tag) => match folders.get(tag) {
                Some(existing) => existing.clone(),
                None => {
                    let created = collection::create_folder_named(workspace, slug, "", tag)?;
                    folders.insert(tag.clone(), created.path.clone());
                    created.path
                }
            },
        };
        if tags.len() > 1 {
            self.warn(format!(
                "{name}: the operation is tagged {} — it was filed under {:?} only, because a request lives in one folder",
                listed(&tags),
                tags[0]
            ));
        }

        let mut notes = Notes {
            optional,
            alternatives: parameters
                .iter()
                .flat_map(|p| p.alternatives.clone())
                .collect(),
            body: None,
            files: None,
            response: self.response_note(operation),
        };

        let fanned_out = variants.len() > 1;
        if fanned_out {
            let labels: Vec<String> = variants.iter().filter_map(|v| v.label.clone()).collect();
            self.warn(format!(
                "{name}: the spec names {} request examples — one request was imported for each ({})",
                labels.len(),
                listed(&labels)
            ));
        }
        for variant in variants {
            if self.report.imported >= MAX_OPERATIONS {
                self.report.skipped.push(format!(
                    "{name}: the specification declares more than {MAX_OPERATIONS} requests, which is the import limit — split it and import the rest separately"
                ));
                return Ok(false);
            }
            let request_name = match (&variant.label, fanned_out) {
                (Some(label), true) => format!("{name} ({label})"),
                _ => name.clone(),
            };
            notes.body = match (&variant.label, &variant.summary) {
                (Some(label), Some(summary)) => {
                    Some(format!("Body: the spec's {label:?} example — {summary}"))
                }
                (Some(label), None) => Some(format!("Body: the spec's {label:?} example.")),
                (None, _) => None,
            };
            notes.files = variant.files.clone();
            let request = SavedRequest {
                id: uuid::Uuid::new_v4().to_string(),
                name: request_name,
                kind: "http".to_string(),
                method: method.to_uppercase(),
                url: url.clone(),
                description: self.description_for(operation, &notes),
                headers: headers.clone(),
                auth: auth.clone(),
                body: variant.body,
                grpc: None,
                stream: None,
                scripts: Scripts::default(),
                tests: Vec::new(),
                captures: Vec::new(),
            };
            collection::save_request(workspace, slug, None, Some(&folder), &request)?;
            self.report.imported += 1;
        }
        Ok(true)
    }

    fn description_for(&self, operation: &Value, notes: &Notes) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(summary) = text(operation.get("summary")) {
            parts.push(summary.trim().to_string());
        }
        if let Some(description) = text(operation.get("description")) {
            parts.push(description.trim().to_string());
        }
        if operation.get("deprecated").and_then(Value::as_bool) == Some(true) {
            parts.push("DEPRECATED in the specification.".to_string());
        }
        if let Some(body) = &notes.body {
            parts.push(body.clone());
        }
        if let Some(files) = &notes.files {
            parts.push(files.clone());
        }
        if !notes.optional.is_empty() {
            parts.push(format!(
                "Optional parameters the spec declares — a .http file has no disabled row, so add the ones you want:\n{}",
                notes.optional.join("\n")
            ));
        }
        if !notes.alternatives.is_empty() {
            parts.push(format!(
                "Other parameter values the spec gives as examples — a variable holds one at a time:\n{}",
                notes.alternatives.join("\n")
            ));
        }
        if let Some(response) = &notes.response {
            parts.push(response.clone());
        }
        match parts.is_empty() {
            true => None,
            false => Some(defuse_directives(&parts.join("\n\n"))),
        }
    }

    /// The example the spec gives for the first success response, as reference
    /// prose. It is never turned into an assertion: an import seeds path and
    /// query parameters with placeholders, so any test written from a response
    /// would fail on the first run and teach the user to delete tests.
    fn response_note(&self, operation: &Value) -> Option<String> {
        let responses = as_object(operation.get("responses"))?;
        let (status, declared) = responses
            .iter()
            .filter(|(code, _)| code.starts_with('2'))
            .min_by_key(|(code, _)| code.as_str())
            .or_else(|| responses.get_key_value("default"))?;
        let declared = resolved(&self.root, declared).ok()?;
        let value = match self.version {
            Version::Swagger20 => as_object(declared.get("examples"))?
                .values()
                .next()?
                .clone(),
            _ => {
                let content = as_object(declared.get("content"))?;
                let types: Vec<String> = content.keys().cloned().collect();
                let media = &content[&pick_content_type(&types)?];
                media.get("example").cloned().or_else(|| {
                    as_object(media.get("examples"))
                        .and_then(|m| m.values().next())
                        .and_then(|entry| resolved(&self.root, entry).ok())
                        .and_then(|entry| entry.get("value").cloned())
                })?
            }
        };
        let rendered = match &value {
            Value::String(raw) => raw.clone(),
            other => serde_json::to_string_pretty(other).ok()?,
        };
        Some(format!(
            "Example {status} response from the specification — reference only, nothing here asserts it:\n{}",
            clip(&rendered)
        ))
    }

    fn templated_path(&mut self, name: &str, path: &str, parameters: &[Parameter]) -> String {
        let mut out = path.to_string();
        for parameter in parameters.iter().filter(|p| p.place == "path") {
            let brace = format!("{{{}}}", parameter.name);
            let replacement = match parameter.templated {
                true => format!("{{{{{}}}}}", parameter.name),
                false => parameter.seed.clone(),
            };
            out = out.replace(&brace, &replacement);
        }
        if out.contains('{') && !out.contains("{{") {
            self.warn(format!(
                "{name}: the path {path:?} templates a parameter the operation never declares — it stays literal in the URL"
            ));
        }
        out
    }

    fn parameter_from(&mut self, declared: &Value) -> Option<Parameter> {
        let name = text(declared.get("name"))?.to_string();
        let place = declared
            .get("in")
            .and_then(Value::as_str)
            .unwrap_or("query")
            .to_string();
        let required =
            declared.get("required").and_then(Value::as_bool) == Some(true) || place == "path";
        // Swagger 2.0 puts the type keywords on the parameter itself; 3.x nests
        // them under `schema`. Both reach the same generator.
        let schema = declared.get("schema").unwrap_or(declared);
        let named = self.named_examples(declared);
        let singular = declared.get("example").map(scalar_text);
        let seed = singular
            .clone()
            .or_else(|| named.first().map(|(_, value, _)| scalar_text(value)))
            .unwrap_or_else(|| scalar_text(&Example::generate(&self.root, schema).value));
        // The first named example became the seed unless `example` already had;
        // the ones left over are what the comment block offers.
        let spare = usize::from(singular.is_none());
        let alternatives = named
            .iter()
            .skip(spare)
            .take(MAX_LISTED)
            .map(|(key, value, summary)| match summary {
                Some(summary) => format!(
                    "  {name}={} — example {key:?}: {summary}",
                    scalar_text(value)
                ),
                None => format!("  {name}={} — example {key:?}", scalar_text(value)),
            })
            .collect();
        let templated = valid_var_name(&name);
        if templated && (required || place == "path") {
            self.seeds
                .entry(name.clone())
                .or_insert_with(|| seed.clone());
        }
        Some(Parameter {
            name,
            place,
            required,
            description: text(declared.get("description")).map(|d| d.replace('\n', " ")),
            seed,
            templated,
            alternatives,
        })
    }

    fn auth_for(
        &mut self,
        name: &str,
        operation: &Value,
        schemes: &BTreeMap<String, Scheme>,
        global: &Security,
        headers: &mut Vec<(String, String)>,
        query: &mut Vec<String>,
    ) -> Auth {
        let own = security_from(operation.get("security"), schemes);
        let effective = if matches!(own, Security::Unset) {
            global
        } else {
            &own
        };
        let scheme = match effective {
            Security::None | Security::Unset => return Auth::None,
            Security::Unknown(missing) => {
                self.warn(format!(
                    "{name}: the operation requires the security scheme {missing:?}, which the document never defines — imported without auth"
                ));
                return Auth::None;
            }
            Security::One {
                scheme,
                alternatives,
                combined,
            } => {
                if *alternatives > 1 {
                    self.warn(format!(
                        "{name}: the specification accepts {alternatives} alternative security requirements — the first one ({:?}) was imported",
                        scheme.name
                    ));
                }
                if *combined > 1 {
                    self.warn(format!(
                        "{name}: the operation requires {combined} schemes at once, which a request can carry only one of — {:?} was imported",
                        scheme.name
                    ));
                }
                scheme.clone()
            }
        };

        let var = sanitize_var_name(&scheme.name);
        match (scheme.kind.as_str(), scheme.scheme.as_str()) {
            ("http", "bearer") => {
                self.secrets.insert(var.clone());
                Auth::inherited(Auth::Bearer {
                    token: format!("{{{{{var}}}}}"),
                })
            }
            ("http", "basic") => {
                let user = format!("{var}User");
                let password = format!("{var}Password");
                self.secrets.insert(user.clone());
                self.secrets.insert(password.clone());
                Auth::inherited(Auth::Basic {
                    username: format!("{{{{{user}}}}}"),
                    password: format!("{{{{{password}}}}}"),
                })
            }
            ("http", other) => {
                self.warn(format!(
                    "{name}: HTTP {other:?} auth is not supported — imported without auth, so the request goes out unauthenticated"
                ));
                Auth::None
            }
            ("apiKey", _) => {
                self.secrets.insert(var.clone());
                let value = format!("{{{{{var}}}}}");
                match scheme.placement.as_str() {
                    "query" => {
                        query.push(format!(
                            "{}={}",
                            encode_query_component(&scheme.key),
                            encode_query_component(&value)
                        ));
                        self.warn(format!(
                            "{name}: the API key moved into the URL query — a .http file writes an api key as a header or a query parameter, not as typed auth"
                        ));
                        Auth::None
                    }
                    "cookie" => {
                        headers.push(("Cookie".to_string(), format!("{}={value}", scheme.key)));
                        self.warn(format!(
                            "{name}: the API key is sent as a Cookie header — a .http file has no cookie jar"
                        ));
                        Auth::None
                    }
                    // A .http file writes an api key as the header it always was,
                    // and reads it back as a header. There is no `inherited`
                    // marker for it, so an unset key stops the request instead of
                    // quietly dropping the way an unset bearer token does.
                    _ => {
                        self.warn(format!(
                            "{name}: the API key is written as the header {} — set {var} before running, or the request stops on the unresolved variable",
                            scheme.key
                        ));
                        Auth::Apikey {
                            key: scheme.key.clone(),
                            value,
                            placement: "header".to_string(),
                        }
                    }
                }
            }
            ("oauth2", _) => {
                let flows = match scheme.flows.is_empty() {
                    true => "no flow".to_string(),
                    false => format!("the {} flow", listed(&scheme.flows)),
                };
                self.warn(format!(
                    "{name}: OAuth 2.0 ({:?}, {flows}) imported without auth — Mándalo does not run OAuth flows; paste a token into a bearer variable to send it",
                    scheme.name
                ));
                Auth::None
            }
            ("openIdConnect", _) => {
                let url = scheme.connect_url.as_deref().unwrap_or("no discovery URL");
                self.warn(format!(
                    "{name}: OpenID Connect ({:?}, {url}) imported without auth — Mándalo does not run discovery or token flows",
                    scheme.name
                ));
                Auth::None
            }
            (other, _) => {
                self.warn(format!(
                    "{name}: {other:?} auth is not supported — imported without auth, so the request goes out unauthenticated"
                ));
                Auth::None
            }
        }
    }

    fn body_for(
        &mut self,
        name: &str,
        operation: &Value,
        headers: &mut Vec<(String, String)>,
    ) -> Vec<BodyVariant> {
        match self.version {
            Version::Swagger20 => vec![self.swagger_body(name, operation, headers)],
            _ => self.v3_bodies(name, operation, headers),
        }
    }

    fn v3_bodies(
        &mut self,
        name: &str,
        operation: &Value,
        headers: &mut Vec<(String, String)>,
    ) -> Vec<BodyVariant> {
        let empty = vec![BodyVariant::plain(Body::None)];
        let Some(declared) = operation.get("requestBody") else {
            return empty;
        };
        let Ok(declared) = resolved(&self.root, declared) else {
            self.warn(format!(
                "{name}: the request body reference could not be resolved — imported without a body"
            ));
            return empty;
        };
        let Some(content) = as_object(declared.get("content")) else {
            return empty;
        };
        let types: Vec<String> = content.keys().cloned().collect();
        let Some(chosen) = pick_content_type(&types) else {
            return empty;
        };
        if types.len() > 1 {
            self.warn(format!(
                "{name}: the operation accepts {} — the body was written as {chosen}",
                listed(&types)
            ));
        }
        let media = &content[&chosen];

        if is_multipart_form_data(&chosen) {
            return vec![self.multipart_body(name, media)];
        }

        // `example` is the singular shorthand and wins over `examples`; the two are
        // mutually exclusive in a spec that validates.
        if let Some(value) = media.get("example").cloned() {
            return vec![BodyVariant::plain(
                self.body_of(name, &chosen, value, headers),
            )];
        }
        let named = self.named_examples(media);
        if named.is_empty() {
            let example = match media.get("schema") {
                Some(schema) => Example::generate(&self.root, schema),
                None => Generated {
                    value: Value::Null,
                    truncated: None,
                    branched: None,
                },
            };
            self.note_example(name, &example);
            return vec![BodyVariant::plain(self.body_of(
                name,
                &chosen,
                example.value,
                headers,
            ))];
        }

        let total = named.len();
        let mut kept = named;
        if total > MAX_BODY_EXAMPLES {
            let dropped: Vec<String> = kept
                .split_off(MAX_BODY_EXAMPLES)
                .into_iter()
                .map(|(key, _, _)| key)
                .collect();
            self.report.skipped.push(format!(
                "{name}: {} of the {total} named request examples were not imported — an operation writes at most {MAX_BODY_EXAMPLES} requests ({})",
                dropped.len(),
                listed(&dropped)
            ));
        }
        let mut out = Vec::with_capacity(kept.len());
        for (label, value, summary) in kept {
            out.push(BodyVariant {
                body: self.body_of(name, &chosen, value, headers),
                label: Some(label),
                summary,
                files: None,
            });
        }
        out
    }

    /// The Media Object's `examples` map: a name, the value it carries, and the
    /// one-line summary its author wrote for it.
    fn named_examples(&self, media: &Value) -> Vec<(String, Value, Option<String>)> {
        let Some(examples) = as_object(media.get("examples")) else {
            return Vec::new();
        };
        examples
            .iter()
            .filter_map(|(key, entry)| {
                let entry = resolved(&self.root, entry).ok()?;
                let value = entry.get("value")?.clone();
                let summary = text(entry.get("summary"))
                    .or_else(|| text(entry.get("description")))
                    .map(|s| s.replace(['\r', '\n'], " ").trim().to_string());
                Some((key.clone(), value, summary))
            })
            .collect()
    }

    fn swagger_body(
        &mut self,
        name: &str,
        operation: &Value,
        headers: &mut Vec<(String, String)>,
    ) -> BodyVariant {
        let consumes: Vec<String> = as_array(operation.get("consumes"))
            .iter()
            .chain(as_array(self.root.get("consumes")))
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        let chosen = pick_content_type(&consumes).unwrap_or_else(|| "application/json".to_string());

        let mut form: Vec<(String, String)> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        let mut payload: Option<Generated> = None;
        for entry in as_array(operation.get("parameters")) {
            let Ok(declared) = resolved(&self.root, entry) else {
                continue;
            };
            match declared.get("in").and_then(Value::as_str) {
                Some("body") => {
                    let schema = declared.get("schema").cloned().unwrap_or(Value::Null);
                    payload = Some(Example::generate(&self.root, &schema));
                }
                Some("formData") => {
                    let Some(key) = text(declared.get("name")) else {
                        continue;
                    };
                    if declared.get("type").and_then(Value::as_str) == Some("file") {
                        files.push(key.to_string());
                        form.push((key.to_string(), String::new()));
                        continue;
                    }
                    let generated = Example::generate(&self.root, &declared).value;
                    form.push((key.to_string(), scalar_text(&generated)));
                }
                _ => {}
            }
        }
        if let Some(example) = payload {
            self.note_example(name, &example);
            let body = self.body_of(name, &chosen, example.value, headers);
            return BodyVariant::plain(body);
        }
        if form.is_empty() {
            return BodyVariant::plain(Body::None);
        }
        if !files.is_empty() {
            if !is_multipart_form_data(&chosen) {
                self.warn(format!(
                    "{name}: the body was written as multipart/form-data — the operation declares a file field, which no other body carries"
                ));
            }
            let unresolved: Vec<(String, String)> = files
                .iter()
                .map(|key| (key.clone(), "the spec declares it as a file".to_string()))
                .collect();
            self.warn_unresolved_files(name, &files);
            let mut rows = Vec::with_capacity(form.len());
            for (key, value) in form {
                match postman::unwritable_form_row(&key, &value) {
                    Some(why) => {
                        self.warn(format!("{name}: form field `{key}` was dropped — {why}"))
                    }
                    None => rows.push(FormDataRow::text(key, value)),
                }
            }
            return BodyVariant {
                files: Some(postman::unresolved_files_note(&unresolved)),
                ..BodyVariant::plain(Body::Formdata { rows })
            };
        }
        let object: Map<String, Value> = form
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let body = self.body_of(
            name,
            "application/x-www-form-urlencoded",
            Value::Object(object),
            headers,
        );
        BodyVariant::plain(body)
    }

    /// A `multipart/form-data` schema becomes the field lines the format writes:
    /// one row per property, a file property left empty because a spec names no
    /// file a workspace could hold.
    fn multipart_body(&mut self, name: &str, media: &Value) -> BodyVariant {
        let schema = media
            .get("schema")
            .and_then(|s| resolved(&self.root, s).ok())
            .unwrap_or(Value::Null);
        let properties: Vec<(String, Value)> = as_object(schema.get("properties"))
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let seeded: Vec<(String, Value)> = match properties.is_empty() {
            false => properties,
            true => as_object(media.get("example"))
                .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        };
        if seeded.is_empty() {
            self.report.skipped.push(format!(
                "{name}: the multipart/form-data body was not imported — its schema names no properties, so there are no form fields to write"
            ));
            return BodyVariant::plain(Body::None);
        }

        let encoding = as_object(media.get("encoding"))
            .cloned()
            .unwrap_or_default();
        let mut rows: Vec<FormDataRow> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        let mut unresolved: Vec<(String, String)> = Vec::new();
        let mut typed_text: Vec<String> = Vec::new();
        for (key, property) in seeded {
            let property = resolved(&self.root, &property).unwrap_or(property);
            let part_type = text(encoding.get(&key).and_then(|e| e.get("contentType")))
                .or_else(|| text(property.get("contentMediaType")))
                .map(String::from);
            if let Some(why) = postman::unwritable_form_row(&key, "") {
                self.warn(format!("{name}: form field `{key}` was dropped — {why}"));
                continue;
            }
            if is_file_property(&property) {
                let referenced = match &part_type {
                    Some(part_type) => {
                        format!("the spec declares it as a file, sent as {part_type}")
                    }
                    None => "the spec declares it as a file".to_string(),
                };
                files.push(key.clone());
                unresolved.push((key.clone(), referenced));
                rows.push(FormDataRow::text(key, ""));
                continue;
            }
            let value = match property.is_object() {
                true => Example::generate(&self.root, &property).value,
                false => property,
            };
            let value = scalar_text(&value);
            if let Some(why) = postman::unwritable_form_row(&key, &value) {
                self.warn(format!("{name}: form field `{key}` was dropped — {why}"));
                continue;
            }
            if part_type.is_some() {
                typed_text.push(key.clone());
            }
            rows.push(FormDataRow::text(key, value));
        }
        if rows.is_empty() {
            return BodyVariant::plain(Body::None);
        }
        if !typed_text.is_empty() {
            self.warn(format!(
                "{name}: the encoding content type on text fields ({}) was dropped — a .http file sets `; type=` on file fields only",
                listed(&typed_text)
            ));
        }
        if !files.is_empty() {
            self.warn_unresolved_files(name, &files);
        }
        BodyVariant {
            files: match unresolved.is_empty() {
                true => None,
                false => Some(postman::unresolved_files_note(&unresolved)),
            },
            ..BodyVariant::plain(Body::Formdata { rows })
        }
    }

    fn warn_unresolved_files(&mut self, name: &str, files: &[String]) {
        let pick = files
            .iter()
            .map(|key| format!("`{key} = < ./your-file`"))
            .collect::<Vec<_>>()
            .join(", ");
        self.warn(format!(
            "{name}: the form-data file fields ({}) arrived empty — a spec names no file a workspace could hold, so point each at one with {pick}",
            listed(files)
        ));
    }

    fn note_example(&mut self, name: &str, example: &Generated) {
        if let Some(why) = &example.branched {
            self.warn(format!("{name}: {why}"));
        }
        if let Some(why) = &example.truncated {
            self.warn(format!(
                "{name}: the example body stops early because {why} — it is a skeleton, not a complete request"
            ));
        }
    }

    fn body_of(
        &mut self,
        name: &str,
        content_type: &str,
        value: Value,
        headers: &mut Vec<(String, String)>,
    ) -> Body {
        let lower = content_type.to_ascii_lowercase();
        let set = |headers: &mut Vec<(String, String)>, value: &str| {
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                headers.push(("Content-Type".to_string(), value.to_string()));
            }
        };
        if lower.starts_with("multipart/") {
            self.report.skipped.push(format!(
                "{name}: the {content_type} body was not imported — a .http file writes multipart/form-data as `name = value` field lines and no other multipart subtype, so the request was imported without one"
            ));
            return Body::None;
        }
        if lower.contains("octet-stream")
            || lower.starts_with("image/")
            || lower.starts_with("audio/")
            || lower.starts_with("video/")
        {
            self.warn(format!(
                "{name}: the {content_type} body needs a file inside the workspace — imported without a body, so write `< ./your-file` under the request"
            ));
            return Body::None;
        }
        if lower.contains("x-www-form-urlencoded") {
            set(headers, "application/x-www-form-urlencoded");
            let pairs: Vec<String> = value
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(k, v)| {
                            format!(
                                "{}={}",
                                encode_query_component(k),
                                encode_query_component(&scalar_text(v))
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Body::Raw {
                language: RawLanguage::Text,
                text: pairs.join("&"),
            };
        }
        if lower.contains("xml") {
            set(headers, content_type);
            return Body::Raw {
                language: RawLanguage::Xml,
                text: scalar_text(&value),
            };
        }
        if lower.contains("json") {
            set(headers, content_type);
            let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
            return Body::json(text);
        }
        set(headers, content_type);
        Body::Raw {
            language: RawLanguage::Text,
            text: scalar_text(&value),
        }
    }
}

enum Security {
    /// The scope says nothing, so the scope above it decides.
    Unset,
    /// `security: []` — explicitly no auth, and nothing is inherited.
    None,
    Unknown(String),
    One {
        scheme: Scheme,
        alternatives: usize,
        combined: usize,
    },
}

/// A description is written as `#` comment lines, and a comment whose text
/// starts with `@` is a directive the `.http` parser refuses. Quoting the line
/// keeps the words and defuses the token.
/// A spec's response example can be pages long, and it is reference prose, not
/// payload — a `#` comment block that dwarfs the request helps nobody.
fn clip(text: &str) -> String {
    let mut out = String::new();
    let mut cut = false;
    for (index, line) in text.lines().enumerate() {
        if index >= MAX_RESPONSE_EXAMPLE_LINES
            || out.len() + line.len() > MAX_RESPONSE_EXAMPLE_CHARS
        {
            cut = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    let mut out = out.trim_end().to_string();
    if cut {
        out.push_str("\n… cut here — the whole example is in the specification");
    }
    out
}

fn defuse_directives(description: &str) -> String {
    description
        .lines()
        .map(|line| match line.trim_start().starts_with('@') {
            true => format!("> {}", line.trim_start()),
            false => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn security_from(declared: Option<&Value>, schemes: &BTreeMap<String, Scheme>) -> Security {
    let Some(requirements) = declared.and_then(Value::as_array) else {
        return Security::Unset;
    };
    if requirements.is_empty() {
        return Security::None;
    }
    let Some(first) = requirements.first().and_then(Value::as_object) else {
        return Security::None;
    };
    let names: Vec<String> = first.keys().cloned().collect();
    let Some(chosen) = names.first().and_then(|n| schemes.get(n)) else {
        return Security::Unknown(names.first().cloned().unwrap_or_default());
    };
    Security::One {
        scheme: chosen.clone(),
        alternatives: requirements.len(),
        combined: names.len(),
    }
}

fn pick_content_type(types: &[String]) -> Option<String> {
    let rank = |t: &str| {
        let lower = t.to_ascii_lowercase();
        match () {
            _ if lower == "application/json" => 0,
            _ if lower.ends_with("+json") || lower.contains("json") => 1,
            _ if lower.contains("x-www-form-urlencoded") => 2,
            _ if lower.contains("xml") => 3,
            _ if lower.starts_with("text/") => 4,
            _ if lower.starts_with("multipart/") => 6,
            _ => 5,
        }
    };
    types
        .iter()
        .min_by_key(|t| (rank(t), t.to_string()))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_str(dir: &Path, source: &str) -> CoreResult<ImportReport> {
        import(dir, source)
    }

    #[test]
    fn a_missing_version_field_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let err = import_str(dir.path(), "{\"paths\": {}}").unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("no top-level `openapi`"));
    }

    #[test]
    fn swagger_1_2_names_the_version_it_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let err = import_str(dir.path(), "{\"swagger\": \"1.2\", \"paths\": {}}")
            .unwrap_err()
            .to_string();
        assert!(err.contains("1.2"), "{err}");
        assert!(err.contains("Swagger 2.0"), "{err}");
    }

    #[test]
    fn openapi_4_names_the_version_it_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let err = import_str(dir.path(), "{\"openapi\": \"4.0.0\", \"paths\": {}}")
            .unwrap_err()
            .to_string();
        assert!(err.contains("4.0.0"), "{err}");
    }

    #[test]
    fn a_document_over_the_byte_limit_is_refused_before_it_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let huge = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        let err = import_str(dir.path(), &huge).unwrap_err().to_string();
        assert!(err.contains("import limit"), "{err}");
    }

    #[test]
    fn yaml_anchors_are_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let source = "openapi: 3.0.3\nx-base: &shared\n  a: 1\npaths: {}\n";
        let err = import_str(dir.path(), source).unwrap_err().to_string();
        assert!(err.contains("anchor"), "{err}");
        assert!(err.contains("&shared"), "{err}");
    }

    #[test]
    fn a_prose_ampersand_is_not_mistaken_for_an_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let source =
            "openapi: 3.0.3\ninfo:\n  title: Terms & conditions\n  version: '1'\npaths: {}\n";
        assert_eq!(import_str(dir.path(), source).unwrap().imported, 0);
    }

    #[test]
    fn a_description_that_starts_with_an_at_sign_still_writes_a_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = serde_json::json!({
            "openapi": "3.0.3",
            "info": {"title": "Directive", "version": "1"},
            "servers": [{"url": "https://x.dev"}],
            "paths": {"/a": {"get": {
                "operationId": "readA",
                "description": "@deprecated use /b instead"
            }}}
        })
        .to_string();
        assert_eq!(import_str(dir.path(), &source).unwrap().imported, 1);
        let written = collection::load_request_source(dir.path(), "directive", "reada.http#0");
        assert_eq!(written.unwrap().name, "readA");
    }

    #[test]
    fn the_importer_is_chosen_by_what_the_file_declares() {
        assert!(looks_like_openapi("{\"openapi\": \"3.0.3\"}"));
        assert!(looks_like_openapi("openapi: 3.1.0\npaths: {}\n"));
        assert!(looks_like_openapi("{\"swagger\":\"2.0\"}"));
        assert!(
            !looks_like_openapi("{\"info\": {\"name\": \"talks about \\\"openapi\\\" a lot\"}}"),
            "the word in a description is not a version field"
        );
    }

    #[test]
    fn seed_if_empty_imports_a_root_openapi_and_drops_empty_starters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("collections")).unwrap();
        std::fs::create_dir_all(dir.path().join("environments")).unwrap();
        collection::create_collection(dir.path(), "petstore").unwrap();
        std::fs::write(
            dir.path().join("openapi.json"),
            r#"{
              "openapi": "3.0.3",
              "info": {"title": "Petstore", "version": "1"},
              "servers": [{"url": "https://petstore.example"}],
              "paths": {
                "/pets": {
                  "get": {"operationId": "listPets", "tags": ["pets"]}
                }
              }
            }"#,
        )
        .unwrap();
        let report = seed_if_empty(dir.path()).unwrap().expect("seeded");
        assert_eq!(report.imported, 1);
        let tree = collection::list_tree(dir.path()).unwrap();
        assert_eq!(tree.collections.len(), 1);
        assert!(tree_has_requests(&tree));
        assert!(seed_if_empty(dir.path()).unwrap().is_none());
    }

    #[test]
    fn pointer_decodes_escaped_tokens() {
        let root = serde_json::json!({"paths": {"/a~b": {"x": 1}}});
        assert_eq!(pointer(&root, "#/paths/~1a~0b/x").unwrap(), &Value::from(1));
    }

    #[test]
    fn content_type_preference_is_json_first_and_multipart_last() {
        assert_eq!(
            pick_content_type(&[
                "multipart/form-data".to_string(),
                "application/json".to_string(),
                "text/plain".to_string()
            ]),
            Some("application/json".to_string())
        );
        assert_eq!(
            pick_content_type(&["multipart/form-data".to_string(), "text/csv".to_string()]),
            Some("text/csv".to_string())
        );
        assert_eq!(pick_content_type(&[]), None);
    }

    #[test]
    fn a_self_referencing_schema_terminates() {
        let root = serde_json::json!({
            "components": {"schemas": {"Node": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "child": {"$ref": "#/components/schemas/Node"}
                }
            }}}
        });
        let schema = serde_json::json!({"$ref": "#/components/schemas/Node"});
        let generated = Example::generate(&root, &schema);
        assert_eq!(generated.value["id"], Value::from("string"));
        assert_eq!(generated.value["child"], Value::Null);
        assert!(generated.truncated.unwrap().contains("contains itself"));
    }

    #[test]
    fn format_and_enum_drive_the_generated_scalar() {
        let root = Value::Null;
        let cases = [
            (
                serde_json::json!({"type": "string", "format": "uuid"}),
                Value::from("00000000-0000-0000-0000-000000000000"),
            ),
            (
                serde_json::json!({"type": "string", "format": "date-time"}),
                Value::from("2026-01-01T00:00:00Z"),
            ),
            (
                serde_json::json!({"type": "string", "enum": ["a", "b"]}),
                Value::from("a"),
            ),
            (
                serde_json::json!({"type": ["string", "null"]}),
                Value::from("string"),
            ),
            (serde_json::json!({"type": "integer"}), Value::from(0)),
            (serde_json::json!({"type": "boolean"}), Value::from(true)),
        ];
        for (schema, expected) in cases {
            assert_eq!(
                Example::generate(&root, &schema).value,
                expected,
                "{schema}"
            );
        }
    }
}
