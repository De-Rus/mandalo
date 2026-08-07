use crate::capability::HostPolicy;
use crate::collection::{self, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::request;
use crate::review;
use crate::scan::{self, Finding};
use crate::workspace::{self, Manifest, VarDef, WorkspaceInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A hostile repository must not be able to hang the app or fill the disk, so
/// every dimension of a remote workspace is bounded before a byte is written:
/// how many files, how big each one is, how big they are together, and how deep
/// the tree goes.
pub const MAX_FILES: usize = 400;
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 512 * 1024;
pub const MAX_DEPTH: usize = 10;

/// The extensions a workspace is made of. Anything else in the repository is
/// listed as skipped and never written — a remote collection is `.http`,
/// `.grpc` and TOML, not an arbitrary payload.
const ALLOWED: &[&str] = &[
    "toml", "http", "rest", "grpc", "ws", "mqtt", "proto", "json", "txt", "graphql", "gql", "md",
    "xml", "csv",
];

const USER_AGENT: &str = concat!("mandalo/", env!("CARGO_PKG_VERSION"));

/// Where the repository is read from. Production points at GitHub; a test
/// points at a fixture server, so the suite never touches the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub api: String,
    pub raw: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints::github()
    }
}

impl Endpoints {
    pub fn github() -> Self {
        Endpoints {
            api: "https://api.github.com".to_string(),
            raw: "https://raw.githubusercontent.com".to_string(),
        }
    }

    pub fn at(api: impl Into<String>, raw: impl Into<String>) -> Self {
        Endpoints {
            api: api.into(),
            raw: raw.into(),
        }
    }
}

/// What a user asked to open, once it has been read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RemoteSource {
    Repo {
        owner: String,
        name: String,
        reference: Option<String>,
        subdir: Option<String>,
    },
    Document {
        url: String,
    },
}

fn valid_segment(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn invalid(raw: &str) -> CoreError {
    CoreError::InvalidName(format!(
        "{raw:?} is not a collection Mándalo can open. Give a public GitHub repository as owner/name (optionally owner/name/sub/dir#branch), its https://github.com/… url, or the url of a single Mándalo bundle file."
    ))
}

fn split_reference(raw: &str) -> (&str, Option<String>) {
    match raw.split_once('#') {
        Some((before, reference)) if !reference.is_empty() => (before, Some(reference.to_string())),
        _ => (raw, None),
    }
}

fn subdir_from(parts: &[&str], raw: &str) -> CoreResult<Option<String>> {
    if parts.is_empty() {
        return Ok(None);
    }
    for part in parts {
        if !valid_segment(part) {
            return Err(invalid(raw));
        }
    }
    Ok(Some(parts.join("/")))
}

/// A url with `user:token@host` in it. People do paste these — a personal access
/// token embedded in a clone url is a documented GitHub shape — and accepting one
/// would put a live credential into a workspace name, a registry file and every
/// error message that mentions the origin.
fn carries_credentials(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") {
        return false;
    }
    rest.split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

/// Every shape a collection can be named by. A bare `owner/name` is the short
/// form the deep link uses; the browsable `tree/branch/dir` url is what a user
/// copies out of the address bar, and both have to land on the same repository.
pub fn parse_source(raw: &str) -> CoreResult<RemoteSource> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid(raw));
    }
    if trimmed.contains("://")
        && !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
    {
        return Err(CoreError::Unsupported(format!(
            "{trimmed} is not an http or https url — a public collection is read over the web. For an ssh remote you already have access to, clone it instead."
        )));
    }
    if carries_credentials(trimmed) {
        // The rejected url is never echoed: it is the credential.
        return Err(CoreError::Secret(
            "that url carries a username and password or token in it. Mándalo will not open a collection from a url with a credential in it — the url would end up in the workspace registry and in every message that names where the collection came from. Remove the credential from the url. If the repository is private, sign in to GitHub in the desktop app and clone it instead.".to_string(),
        ));
    }
    let without_scheme = trimmed
        .strip_prefix("github:")
        .or_else(|| trimmed.strip_prefix("https://github.com/"))
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("https://www.github.com/"));

    if let Some(rest) = without_scheme {
        return parse_repo(rest.trim_end_matches('/'), raw);
    }

    if let Some(rest) = trimmed
        .strip_prefix("https://raw.githubusercontent.com/")
        .or_else(|| trimmed.strip_prefix("http://raw.githubusercontent.com/"))
    {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 && parts[3..].join("/").ends_with(".json") {
            return Ok(RemoteSource::Document {
                url: trimmed.to_string(),
            });
        }
        if parts.len() < 3 {
            return Err(invalid(raw));
        }
        let subdir = subdir_from(&parts[3..], raw)?;
        return finish_repo(parts[0], parts[1], Some(parts[2].to_string()), subdir, raw);
    }

    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        return Ok(RemoteSource::Document {
            url: trimmed.to_string(),
        });
    }

    parse_repo(trimmed.trim_end_matches('/'), raw)
}

fn parse_repo(rest: &str, raw: &str) -> CoreResult<RemoteSource> {
    let (path, reference) = split_reference(rest);
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return Err(invalid(raw));
    }
    let owner = parts.remove(0);
    let name = parts.remove(0);

    // `owner/name/tree/<ref>/<dir>` is what the GitHub UI puts in the address bar.
    let (reference, tail) = match (parts.first(), reference) {
        (Some(&"tree"), None) | (Some(&"blob"), None) if parts.len() >= 2 => {
            (Some(parts[1].to_string()), parts[2..].to_vec())
        }
        (_, given) => (given, parts.clone()),
    };
    let subdir = subdir_from(&tail, raw)?;
    finish_repo(owner, name, reference, subdir, raw)
}

fn finish_repo(
    owner: &str,
    name: &str,
    reference: Option<String>,
    subdir: Option<String>,
    raw: &str,
) -> CoreResult<RemoteSource> {
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !valid_segment(owner) || !valid_segment(name) {
        return Err(invalid(raw));
    }
    if let Some(reference) = &reference {
        if !valid_segment(reference) {
            return Err(invalid(raw));
        }
    }
    Ok(RemoteSource::Repo {
        owner: owner.to_string(),
        name: name.to_string(),
        reference,
        subdir,
    })
}

/// Where a remote workspace came from, kept in its `mandalo.toml` so the app can
/// say so every time it is opened — a workspace that arrived from a link must
/// never look like one the user wrote.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteOrigin {
    pub label: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub fetched_at: u64,
}

fn has_contents(dir: &Path) -> CoreResult<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    Ok(std::fs::read_dir(dir)
        .map_err(|e| CoreError::io(dir.display(), e))?
        .next()
        .is_some())
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A remote collection, fetched and held in memory. Nothing is on disk yet and
/// nothing has run: this is the raw material a review is computed from and the
/// exact bytes an adoption writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFetch {
    pub origin: RemoteOrigin,
    pub payload: RemotePayload,
    pub skipped: Vec<String>,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemotePayload {
    Tree(Vec<(String, String)>),
    Bundle(String),
}

fn extension_allowed(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| ALLOWED.contains(&e.as_str()))
}

/// A path from a repository is attacker-controlled. It has to be relative, stay
/// inside the workspace, carry no hidden directory, and stop at [`MAX_DEPTH`].
fn accept_path(path: &str) -> Result<(), String> {
    let parts = collection::components(path).map_err(|e| e.message().to_string())?;
    if parts.len() > MAX_DEPTH {
        return Err(format!("{path}: more than {MAX_DEPTH} directories deep"));
    }
    if parts.iter().any(|part| part.starts_with('.')) {
        return Err(format!(
            "{path}: a dot directory or dotfile is never a request"
        ));
    }
    if !extension_allowed(path) {
        return Err(format!("{path}: not a file a workspace is made of"));
    }
    if scan::is_local_values_file(Path::new(path)) {
        return Err(format!(
            "{path}: this file holds values that belong to one machine and is never adopted"
        ));
    }
    Ok(())
}

/// GitHub answers 404 for a repository that does not exist **and** for a private
/// one the caller cannot see — it will not confirm that a private repository is
/// there. So this is genuinely ambiguous, and the message says so rather than
/// asserting which it is.
pub fn private_or_missing(owner: &str, name: &str) -> CoreError {
    CoreError::Private(format!(
        "GitHub did not return {owner}/{name}. It may not exist, or it may be private — GitHub answers the same way for both when nobody is signed in. If it is private, a read-only preview is not what you want anyway: sign in to GitHub and clone it into a workspace of your own, which you can edit and push back to."
    ))
}

async fn get_json(url: &str, policy: &dyn HostPolicy) -> CoreResult<Value> {
    let document = request::fetch_document_with(url, policy, &[("user-agent", USER_AGENT)]).await?;
    serde_json::from_str(&document.text)
        .map_err(|e| CoreError::Parse(format!("{url} did not answer with JSON: {e}")))
}

async fn resolve_commit(
    endpoints: &Endpoints,
    owner: &str,
    name: &str,
    reference: &str,
    policy: &dyn HostPolicy,
) -> CoreResult<String> {
    let url = format!(
        "{}/repos/{owner}/{name}/commits/{reference}",
        endpoints.api.trim_end_matches('/')
    );
    let answer = request::fetch_response(&url, policy, &[("user-agent", USER_AGENT)]).await?;
    match answer.status {
        401 | 403 | 404 => return Err(private_or_missing(owner, name)),
        status if !(200..300).contains(&status) => {
            return Err(CoreError::Request(format!(
                "GitHub answered {status} for {owner}/{name} — try again in a moment"
            )))
        }
        _ => {}
    }
    let body: Value = serde_json::from_str(&answer.text)
        .map_err(|e| CoreError::Parse(format!("{url} did not answer with JSON: {e}")))?;
    body.get("sha")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            CoreError::Parse(format!(
                "{owner}/{name} has no commit called {reference:?} — check the branch name"
            ))
        })
}

/// The clone url for a repository a user turned out not to be able to read
/// anonymously. The desktop offers this; nothing in the browser build calls it,
/// because nothing in the browser build can hold the credential it needs.
pub fn clone_url(source: &RemoteSource) -> Option<String> {
    match source {
        RemoteSource::Repo { owner, name, .. } => {
            Some(format!("https://github.com/{owner}/{name}.git"))
        }
        RemoteSource::Document { .. } => None,
    }
}

struct Blob {
    path: String,
    size: usize,
}

async fn list_tree(
    endpoints: &Endpoints,
    owner: &str,
    name: &str,
    commit: &str,
    policy: &dyn HostPolicy,
) -> CoreResult<Vec<Blob>> {
    let url = format!(
        "{}/repos/{owner}/{name}/git/trees/{commit}?recursive=1",
        endpoints.api.trim_end_matches('/')
    );
    let body = get_json(&url, policy).await?;
    if body.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err(CoreError::Unsupported(format!(
            "{owner}/{name} is too large for Mándalo to list in one read — point at a subdirectory, as owner/name/that/dir"
        )));
    }
    let entries = body
        .get("tree")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::Parse(format!("{url} did not answer with a git tree")))?;
    Ok(entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|entry| {
            Some(Blob {
                path: entry.get("path").and_then(Value::as_str)?.to_string(),
                size: entry.get("size").and_then(Value::as_u64).unwrap_or(0) as usize,
            })
        })
        .collect())
}

fn strip_subdir<'a>(path: &'a str, subdir: Option<&str>) -> Option<&'a str> {
    match subdir {
        None => Some(path),
        Some(prefix) => path
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('/')),
    }
}

/// Reads a public repository over plain HTTPS: no clone, no credential, no
/// working copy. Every fetch — the two metadata reads and every file — goes out
/// through the egress guard, so a repository cannot redirect Mándalo at a host
/// the policy would refuse.
pub async fn fetch(
    source: &RemoteSource,
    endpoints: &Endpoints,
    policy: &dyn HostPolicy,
) -> CoreResult<RemoteFetch> {
    match source {
        RemoteSource::Document { url } => {
            let document = request::fetch_document(url, policy).await?;
            if !document.text.contains("\"mandaloBundle\"") {
                return Err(CoreError::Unsupported(format!(
                    "{} is not a Mándalo bundle — a single-file collection is a bundle exported from Mándalo. Use Import for a Postman or OpenAPI document.",
                    document.url
                )));
            }
            Ok(RemoteFetch {
                origin: RemoteOrigin {
                    label: document.url.clone(),
                    url: document.url.clone(),
                    commit: None,
                    fetched_at: now_seconds(),
                },
                bytes: document.bytes,
                payload: RemotePayload::Bundle(document.text),
                skipped: Vec::new(),
            })
        }
        RemoteSource::Repo {
            owner,
            name,
            reference,
            subdir,
        } => {
            let wanted = reference.clone().unwrap_or_else(|| "HEAD".to_string());
            let commit = resolve_commit(endpoints, owner, name, &wanted, policy).await?;
            let blobs = list_tree(endpoints, owner, name, &commit, policy).await?;

            let mut skipped = Vec::new();
            let mut wanted_files: Vec<Blob> = Vec::new();
            let mut total = 0usize;
            for blob in blobs {
                let Some(relative) = strip_subdir(&blob.path, subdir.as_deref()) else {
                    continue;
                };
                if let Err(why) = accept_path(relative) {
                    skipped.push(why);
                    continue;
                }
                if blob.size > MAX_FILE_BYTES {
                    skipped.push(format!(
                        "{relative}: {} bytes, over the {MAX_FILE_BYTES} byte limit for one file",
                        blob.size
                    ));
                    continue;
                }
                total += blob.size;
                if total > MAX_TOTAL_BYTES {
                    return Err(CoreError::Unsupported(format!(
                        "{owner}/{name} holds more than {MAX_TOTAL_BYTES} bytes of collection files — Mándalo will not open it"
                    )));
                }
                if wanted_files.len() >= MAX_FILES {
                    return Err(CoreError::Unsupported(format!(
                        "{owner}/{name} holds more than {MAX_FILES} collection files — Mándalo will not open it"
                    )));
                }
                wanted_files.push(Blob {
                    path: blob.path,
                    size: blob.size,
                });
            }

            if wanted_files.is_empty() {
                return Err(CoreError::NotFound(format!(
                    "{owner}/{name}{} holds no Mándalo collection — a collection is a directory with mandalo.toml and collections/",
                    subdir.as_deref().map(|d| format!("/{d}")).unwrap_or_default()
                )));
            }

            let base = format!(
                "{}/{owner}/{name}/{commit}",
                endpoints.raw.trim_end_matches('/')
            );
            let mut files = Vec::with_capacity(wanted_files.len());
            let mut fetched_bytes = 0usize;
            for blob in &wanted_files {
                let url = format!("{base}/{}", blob.path);
                let document = request::fetch_document(&url, policy).await?;
                fetched_bytes += document.bytes;
                if fetched_bytes > MAX_TOTAL_BYTES {
                    return Err(CoreError::Unsupported(format!(
                        "{owner}/{name} sent more than {MAX_TOTAL_BYTES} bytes — Mándalo stopped reading it"
                    )));
                }
                let relative = strip_subdir(&blob.path, subdir.as_deref())
                    .expect("the path was accepted under this subdirectory")
                    .to_string();
                files.push((relative, document.text));
            }
            files.sort_by(|a, b| a.0.cmp(&b.0));

            if !files.iter().any(|(path, _)| path == "mandalo.toml")
                && !files
                    .iter()
                    .any(|(path, _)| path.starts_with("collections/"))
            {
                return Err(CoreError::NotFound(format!(
                    "{owner}/{name}{} has no collections/ directory — it is a repository, but not a Mándalo workspace",
                    subdir.as_deref().map(|d| format!("/{d}")).unwrap_or_default()
                )));
            }

            let where_at = subdir
                .as_deref()
                .map(|d| format!(" · {d}"))
                .unwrap_or_default();
            let branch = reference
                .as_deref()
                .map(|r| format!("#{r}"))
                .unwrap_or_default();
            Ok(RemoteFetch {
                origin: RemoteOrigin {
                    label: format!("github.com/{owner}/{name}{branch}{where_at}"),
                    url: format!("https://github.com/{owner}/{name}"),
                    commit: Some(commit),
                    fetched_at: now_seconds(),
                },
                payload: RemotePayload::Tree(files),
                skipped,
                bytes: fetched_bytes,
            })
        }
    }
}

/// A script a remote collection carries. Named, never run: the review says one
/// is there and what it is attached to, and it stays inert until the user sends
/// that request themselves.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptNote {
    pub collection: String,
    pub request: String,
    pub hook: String,
    pub lines: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEnvironment {
    pub name: String,
    pub declared: Vec<String>,
    pub shared_values: usize,
    pub awaiting_values: usize,
}

/// Everything a user needs to decide whether to adopt a stranger's collection,
/// computed without running any of it. `adopt` takes the `token` back and
/// refuses when it no longer describes the same bytes.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteReview {
    pub origin: RemoteOrigin,
    pub files: usize,
    pub bytes: usize,
    pub collections: usize,
    pub requests: usize,
    pub environments: Vec<RemoteEnvironment>,
    pub hosts: Vec<String>,
    pub templated_hosts: Vec<String>,
    pub scripts: Vec<ScriptNote>,
    pub findings: Vec<Finding>,
    pub skipped: Vec<String>,
    pub token: String,
}

/// Writes the fetched bytes into a directory. Used twice with the same input:
/// once into a scratch directory to compute the review, once into the workspace
/// the user agreed to.
fn materialize(fetch: &RemoteFetch, dir: &Path) -> CoreResult<()> {
    std::fs::create_dir_all(dir).map_err(|e| CoreError::io(dir.display(), e))?;
    match &fetch.payload {
        RemotePayload::Tree(files) => {
            for (path, text) in files {
                accept_path(path).map_err(CoreError::PathEscape)?;
                let target = collection::resolve_within(dir, path)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CoreError::io(parent.display(), e))?;
                }
                std::fs::write(&target, text).map_err(|e| CoreError::io(target.display(), e))?;
            }
            if workspace::read_manifest(dir)?.is_none() {
                workspace::write_manifest(
                    dir,
                    &Manifest {
                        schema_version: workspace::SCHEMA_VERSION,
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fetch.origin.label.clone(),
                        remote: None,
                        share: None,
                    },
                )?;
            }
        }
        RemotePayload::Bundle(json) => {
            workspace::write_manifest(
                dir,
                &Manifest {
                    schema_version: workspace::SCHEMA_VERSION,
                    id: uuid::Uuid::new_v4().to_string(),
                    name: fetch.origin.label.clone(),
                    remote: None,
                    share: None,
                },
            )?;
            std::fs::create_dir_all(dir.join("environments"))
                .map_err(|e| CoreError::io(dir.display(), e))?;
            std::fs::create_dir_all(collection::collections_dir(dir))
                .map_err(|e| CoreError::io(dir.display(), e))?;
            crate::bundle::import(dir, json)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HostRef {
    Named(String),
    Templated(String),
}

/// The host a saved url will contact. A `{{variable}}` inside the authority is
/// not a host yet, and guessing at one would be a lie — it is listed as what it
/// is instead.
fn host_of(url: &str) -> Option<HostRef> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let after_scheme = with_scheme.split_once("://").map(|(_, rest)| rest)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    if authority.contains("{{") {
        return Some(HostRef::Templated(trimmed.to_string()));
    }
    let parsed = reqwest::Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(HostRef::Named(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }))
}

fn walk_requests(
    workspace: &Path,
    slug: &str,
    paths: &[String],
    out: &mut Vec<(String, SavedRequest)>,
) -> CoreResult<()> {
    for path in paths {
        out.push((
            slug.to_string(),
            collection::load_request_source(workspace, slug, path)?,
        ));
    }
    Ok(())
}

fn folder_paths(folders: &[collection::FolderNode], out: &mut Vec<String>) {
    for folder in folders {
        for request in &folder.requests {
            out.push(request.path.clone());
        }
        folder_paths(&folder.folders, out);
    }
}

/// Reads what was fetched, on disk in a scratch directory, and says what it is.
/// Parsing only: no request is sent and no script is evaluated.
pub fn review(fetch: &RemoteFetch) -> CoreResult<RemoteReview> {
    let scratch =
        tempfile::tempdir().map_err(|e| CoreError::io("a scratch directory for the review", e))?;
    materialize(fetch, scratch.path())?;
    let mut out = review_of(fetch, scratch.path())?;
    out.token = token_for(fetch)?;
    Ok(out)
}

fn review_of(fetch: &RemoteFetch, dir: &Path) -> CoreResult<RemoteReview> {
    let tree = collection::list_tree(dir)?;
    let mut requests = Vec::new();
    for node in &tree.collections {
        let mut paths: Vec<String> = node.requests.iter().map(|r| r.path.clone()).collect();
        folder_paths(&node.folders, &mut paths);
        walk_requests(dir, &node.slug, &paths, &mut requests)?;
    }

    let mut hosts: BTreeSet<String> = BTreeSet::new();
    let mut templated: BTreeSet<String> = BTreeSet::new();
    let mut scripts = Vec::new();
    for (slug, request) in &requests {
        match host_of(&request.url) {
            Some(HostRef::Named(host)) => {
                hosts.insert(host);
            }
            Some(HostRef::Templated(raw)) => {
                templated.insert(raw);
            }
            None => {}
        }
        for (hook, source) in [
            ("pre", &request.scripts.pre),
            ("post", &request.scripts.post),
        ] {
            if let Some(source) = source {
                scripts.push(ScriptNote {
                    collection: slug.clone(),
                    request: request.name.clone(),
                    hook: hook.to_string(),
                    lines: source.lines().count(),
                });
            }
        }
    }

    let docs = workspace::list_env_docs(dir)?;
    let environments: Vec<RemoteEnvironment> = docs
        .items
        .iter()
        .map(|doc| RemoteEnvironment {
            name: doc.name.clone(),
            declared: doc.vars.keys().cloned().collect(),
            shared_values: doc.vars.values().filter(|v| v.is_shared()).count(),
            awaiting_values: doc
                .vars
                .values()
                .filter(|v| matches!(v, VarDef::Secret { .. } | VarDef::Local { .. }))
                .count(),
        })
        .collect();

    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();
    let mut findings = scan::scan_files(&files)?;
    for finding in &mut findings {
        if let Ok(relative) = finding.path.strip_prefix(dir) {
            finding.path = relative.to_path_buf();
        }
    }

    // A file the fetch declined is a bounded, explained omission. A file that
    // arrived and would not parse is half a workspace, and a stranger's half
    // workspace is refused rather than shown as if it were whole.
    let corrupt: Vec<String> = tree
        .skipped
        .iter()
        .chain(docs.skipped.iter())
        .cloned()
        .collect();
    if !corrupt.is_empty() {
        return Err(CoreError::Parse(format!(
            "this collection did not load whole — Mándalo could not read {}. Nothing has been opened.",
            corrupt.join("; ")
        )));
    }

    Ok(RemoteReview {
        origin: fetch.origin.clone(),
        files: match &fetch.payload {
            RemotePayload::Tree(files) => files.len(),
            RemotePayload::Bundle(_) => 1,
        },
        bytes: fetch.bytes,
        collections: tree.collections.len(),
        requests: requests.len(),
        environments,
        hosts: hosts.into_iter().collect(),
        templated_hosts: templated.into_iter().collect(),
        scripts,
        findings,
        skipped: fetch.skipped.clone(),
        token: String::new(),
    })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> CoreResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn token_for(fetch: &RemoteFetch) -> CoreResult<String> {
    let mut parts = vec![
        fetch.origin.url.clone(),
        fetch.origin.commit.clone().unwrap_or_default(),
    ];
    match &fetch.payload {
        RemotePayload::Tree(files) => {
            for (path, text) in files {
                parts.push(path.clone());
                parts.push(text.clone());
            }
        }
        RemotePayload::Bundle(json) => parts.push(json.clone()),
    }
    review::token("remote", &parts)
}

/// Writes the reviewed bytes into a workspace of their own and registers it —
/// read-only, and stamped with where it came from. `token` has to be the one the
/// review returned, so what was shown is what lands.
pub fn adopt(
    fetch: &RemoteFetch,
    token: &str,
    registry: &Path,
    dest: &Path,
) -> CoreResult<WorkspaceInfo> {
    if token_for(fetch)? != token {
        return Err(review::stale("remote collection"));
    }
    if has_contents(dest)? {
        return Err(CoreError::Conflict(format!(
            "{} already has something in it — a remote collection is opened into a directory of its own",
            dest.display()
        )));
    }
    materialize(fetch, dest)?;
    let existing = workspace::read_manifest(dest)?.ok_or_else(|| {
        CoreError::Io("the remote workspace lost its manifest while being written".to_string())
    })?;
    workspace::write_manifest(
        dest,
        &Manifest {
            schema_version: workspace::SCHEMA_VERSION,
            id: existing.id,
            name: existing.name,
            remote: Some(fetch.origin.clone()),
            share: None,
        },
    )?;
    workspace::open_workspace(registry, dest).map(|opened| opened.workspace)
}

/// Where this workspace came from, or `None` for one the user owns.
pub fn origin(workspace: &Path) -> CoreResult<Option<RemoteOrigin>> {
    Ok(workspace::read_manifest(workspace)?.and_then(|m| m.remote))
}

/// The gate every write passes through. A workspace that arrived from a link is
/// somebody else's; editing it in place would quietly diverge from the thing the
/// link points at, and the user never asked to own it.
pub fn ensure_writable(workspace: &Path) -> CoreResult<()> {
    match origin(workspace)? {
        None => Ok(()),
        Some(origin) => Err(CoreError::ReadOnly(format!(
            "this workspace is a read-only copy of {} — save a copy of it to make changes",
            origin.label
        ))),
    }
}

fn copy_tree(from: &Path, to: &Path) -> CoreResult<()> {
    std::fs::create_dir_all(to).map_err(|e| CoreError::io(to.display(), e))?;
    for entry in std::fs::read_dir(from).map_err(|e| CoreError::io(from.display(), e))? {
        let entry = entry.map_err(|e| CoreError::io(from.display(), e))?;
        let path = entry.path();
        if path.is_symlink() || path.file_name() == Some(std::ffi::OsStr::new(".git")) {
            continue;
        }
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|e| CoreError::io(target.display(), e))?;
        }
    }
    Ok(())
}

/// Turns a remote workspace into an ordinary one the user owns: same files, new
/// identity, no origin stamp, writable. The original is left exactly as it was.
pub fn save_copy(
    source: &Path,
    registry: &Path,
    dest: &Path,
    name: &str,
) -> CoreResult<WorkspaceInfo> {
    if origin(source)?.is_none() {
        return Err(CoreError::Conflict(format!(
            "{} is already a workspace you own — there is nothing to copy out of",
            source.display()
        )));
    }
    if has_contents(dest)? {
        return Err(CoreError::Conflict(format!(
            "{} already has something in it — save the copy somewhere empty",
            dest.display()
        )));
    }
    copy_tree(source, dest)?;
    workspace::write_manifest(
        dest,
        &Manifest {
            schema_version: workspace::SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            remote: None,
            share: None,
        },
    )?;
    workspace::open_workspace(registry, dest).map(|opened| opened.workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(raw: &str) -> (String, String, Option<String>, Option<String>) {
        match parse_source(raw).unwrap() {
            RemoteSource::Repo {
                owner,
                name,
                reference,
                subdir,
            } => (owner, name, reference, subdir),
            other => panic!("{raw} parsed as {other:?}"),
        }
    }

    #[test]
    fn a_bare_owner_slash_name_is_a_repository() {
        assert_eq!(
            repo("acme/collections"),
            ("acme".into(), "collections".into(), None, None)
        );
    }

    #[test]
    fn a_branch_and_a_subdirectory_both_come_through() {
        assert_eq!(
            repo("acme/collections/apis/billing#next"),
            (
                "acme".into(),
                "collections".into(),
                Some("next".into()),
                Some("apis/billing".into())
            )
        );
    }

    #[test]
    fn the_url_a_user_copies_out_of_the_address_bar_works() {
        assert_eq!(
            repo("https://github.com/acme/collections/tree/main/apis"),
            (
                "acme".into(),
                "collections".into(),
                Some("main".into()),
                Some("apis".into())
            )
        );
        assert_eq!(
            repo("https://github.com/acme/collections.git"),
            ("acme".into(), "collections".into(), None, None)
        );
        assert_eq!(
            repo("github:acme/collections"),
            ("acme".into(), "collections".into(), None, None)
        );
    }

    #[test]
    fn a_lone_url_is_a_bundle_document() {
        assert_eq!(
            parse_source("https://example.dev/team.json").unwrap(),
            RemoteSource::Document {
                url: "https://example.dev/team.json".into()
            }
        );
    }

    #[test]
    fn a_raw_github_url_is_read_as_the_repository_it_points_into() {
        assert_eq!(
            repo("https://raw.githubusercontent.com/acme/collections/main/apis"),
            (
                "acme".into(),
                "collections".into(),
                Some("main".into()),
                Some("apis".into())
            )
        );
    }

    #[test]
    fn nonsense_and_other_protocols_are_refused_by_name() {
        assert!(parse_source("").is_err());
        assert!(parse_source("acme").is_err());
        assert!(parse_source("acme/../etc").is_err());
        assert!(parse_source("acme/name/../../etc").is_err());
        let e = parse_source("ssh://git@github.com/acme/x").unwrap_err();
        assert!(e.to_string().contains("http or https"), "{e}");
    }

    #[test]
    fn a_url_with_a_token_in_it_is_refused_without_ever_echoing_the_url() {
        let e = parse_source("https://ghp_00000000000000000000@github.com/acme/x").unwrap_err();
        assert_eq!(e.code(), "E_SECRET");
        assert!(!e.to_string().contains("ghp_"), "the url was echoed back");
        assert!(e.to_string().contains("desktop app"), "{e}");
        assert!(parse_source("https://user:tok@example.dev/a.json").is_err());
    }

    #[test]
    fn a_path_that_climbs_out_or_hides_is_never_written() {
        assert!(accept_path("../outside.http").is_err());
        assert!(accept_path("/etc/passwd").is_err());
        assert!(accept_path(".env").is_err());
        assert!(accept_path(".github/workflows/ci.http").is_err());
        assert!(accept_path("collections/a/.secrets.toml").is_err());
        assert!(accept_path("run.sh").is_err());
        assert!(accept_path(&format!("{}x.http", "a/".repeat(MAX_DEPTH))).is_err());
        assert!(accept_path("collections/mock/http/echo.http").is_ok());
    }

    #[test]
    fn a_templated_authority_is_reported_as_a_template_not_guessed_at() {
        assert_eq!(
            host_of("https://api.example.dev/users"),
            Some(HostRef::Named("api.example.dev".into()))
        );
        assert_eq!(
            host_of("https://api.example.dev:8443/users"),
            Some(HostRef::Named("api.example.dev:8443".into()))
        );
        assert_eq!(
            host_of("{{base}}/users"),
            Some(HostRef::Templated("{{base}}/users".into()))
        );
        assert_eq!(
            host_of("https://{{tenant}}.example.dev/users"),
            Some(HostRef::Templated(
                "https://{{tenant}}.example.dev/users".into()
            ))
        );
        assert_eq!(
            host_of("https://api.example.dev/{{id}}"),
            Some(HostRef::Named("api.example.dev".into()))
        );
    }
}
