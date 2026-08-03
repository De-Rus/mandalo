use crate::assertions::{validate_capture, Capture, Scripts, TestAssertion};
use crate::body::Body;
use crate::error::{CoreError, CoreResult};
use crate::grpc_format::{self, GrpcDoc};
use crate::http_format::{self, HttpDoc};
use crate::request::Auth;
use crate::workspace::SCHEMA_VERSION;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Field order is the on-disk order: every scalar and array-of-values comes
/// before the first table, because TOML cannot emit a value after a table.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", from = "StoredRequest")]
pub struct SavedRequest {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default, skip_serializing_if = "Body::is_none")]
    pub body: Body,
    #[serde(default)]
    pub grpc: Option<GrpcRequest>,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub tests: Vec<TestAssertion>,
    #[serde(default)]
    pub captures: Vec<Capture>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredBody {
    Text(String),
    Structured(Body),
}

#[derive(Deserialize)]
struct LegacyGraphql {
    #[serde(default)]
    query: String,
    #[serde(default)]
    variables: String,
}

/// The shape actually read from disk. Requests written before bodies were
/// modelled carry `body = "…"` plus a `[graphql]` table; both fold into
/// [`Body`] here, and only the folded shape is ever written back.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRequest {
    id: String,
    name: String,
    kind: String,
    method: String,
    url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    auth: Auth,
    #[serde(default)]
    body: Option<StoredBody>,
    #[serde(default)]
    graphql: Option<LegacyGraphql>,
    #[serde(default)]
    grpc: Option<GrpcRequest>,
    #[serde(default)]
    scripts: Scripts,
    #[serde(default)]
    tests: Vec<TestAssertion>,
    #[serde(default)]
    captures: Vec<Capture>,
}

impl From<StoredRequest> for SavedRequest {
    fn from(stored: StoredRequest) -> SavedRequest {
        let body = match (stored.body, stored.graphql) {
            (_, Some(g)) => Body::graphql(g.query, g.variables),
            (Some(StoredBody::Structured(body)), None) => body,
            (Some(StoredBody::Text(text)), None) if text.is_empty() => Body::None,
            (Some(StoredBody::Text(text)), None) => Body::raw(text),
            (None, None) => Body::None,
        };
        SavedRequest {
            id: stored.id,
            name: stored.name,
            kind: stored.kind,
            method: stored.method,
            url: stored.url,
            description: stored.description,
            headers: stored.headers,
            auth: stored.auth,
            body,
            grpc: stored.grpc,
            scripts: stored.scripts,
            tests: stored.tests,
            captures: stored.captures,
        }
    }
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

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct CollectionManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    pub id: String,
    pub slug: String,
    pub name: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionList {
    pub items: Vec<CollectionInfo>,
    pub skipped: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub method: String,
    pub path: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub name: String,
    pub path: String,
    pub folders: Vec<FolderNode>,
    pub requests: Vec<RequestSummary>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionNode {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub folders: Vec<FolderNode>,
    pub requests: Vec<RequestSummary>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tree {
    pub collections: Vec<CollectionNode>,
    pub skipped: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedPath {
    pub path: String,
}

const MANIFEST_FILE: &str = "collection.toml";

pub fn collections_dir(workspace: &Path) -> PathBuf {
    workspace.join("collections")
}

pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

fn validate_component(component: &str) -> CoreResult<()> {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.starts_with('.')
        || component.contains('/')
        || component.contains('\\')
        || component.contains('\0')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid path component: {component:?}"
        )));
    }
    Ok(())
}

pub fn validate_slug(slug: &str) -> CoreResult<()> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid collection slug: {slug:?}"
        )));
    }
    Ok(())
}

fn validate_id(id: &str) -> CoreResult<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid request id: {id:?}"
        )));
    }
    Ok(())
}

pub fn components(rel: &str) -> CoreResult<Vec<&str>> {
    let trimmed = rel.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    for part in &parts {
        validate_component(part)?;
    }
    Ok(parts)
}

fn escapes_base(base: &Path, target: &Path) -> CoreResult<bool> {
    let real_base = std::fs::canonicalize(base).map_err(|e| CoreError::io(base.display(), e))?;
    let mut probe = target.to_path_buf();
    loop {
        if probe.exists() {
            let real =
                std::fs::canonicalize(&probe).map_err(|e| CoreError::io(probe.display(), e))?;
            return Ok(!real.starts_with(&real_base));
        }
        if !probe.pop() {
            return Ok(true);
        }
    }
}

pub fn resolve_within(base: &Path, rel: &str) -> CoreResult<PathBuf> {
    let mut target = base.to_path_buf();
    for part in components(rel)? {
        target.push(part);
    }
    if escapes_base(base, &target)? {
        return Err(CoreError::PathEscape(format!(
            "path escapes the collection: {rel:?}"
        )));
    }
    Ok(target)
}

fn collection_dir(workspace: &Path, slug: &str) -> CoreResult<PathBuf> {
    validate_slug(slug)?;
    let dir = collections_dir(workspace).join(slug);
    if !dir.is_dir() {
        return Err(CoreError::NotFound(format!("unknown collection: {slug}")));
    }
    Ok(dir)
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(MANIFEST_FILE)
}

pub fn read_collection_manifest(dir: &Path) -> CoreResult<CollectionManifest> {
    let path = manifest_path(dir);
    let raw = std::fs::read_to_string(&path).map_err(|e| CoreError::io(path.display(), e))?;
    let manifest: CollectionManifest =
        toml::from_str(&raw).map_err(|e| CoreError::parse(path.display(), e))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(CoreError::Schema(format!(
            "{}: unsupported collection schema_version {} (expected {SCHEMA_VERSION})",
            path.display(),
            manifest.schema_version
        )));
    }
    Ok(manifest)
}

fn write_collection_manifest(dir: &Path, manifest: &CollectionManifest) -> CoreResult<()> {
    let raw = toml::to_string_pretty(manifest).map_err(|e| CoreError::Parse(e.to_string()))?;
    crate::workspace::atomic_write(&manifest_path(dir), &raw)
}

fn unique_dir_slug(parent: &Path, base: &str) -> String {
    if !parent.join(base).exists() {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !parent.join(&candidate).exists() {
            return candidate;
        }
        n += 1;
    }
}

fn unique_request_file(dir: &Path, base: &str, ext: &str, keep: Option<&Path>) -> PathBuf {
    let free = |candidate: &Path| !candidate.exists() || keep.is_some_and(|k| k == candidate);
    let first = dir.join(format!("{base}.{ext}"));
    if free(&first) {
        return first;
    }
    let mut n = 2;
    loop {
        let candidate = dir.join(format!("{base}-{n}.{ext}"));
        if free(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Requests are plain text now: `.http`/`.rest` for HTTP and GraphQL, `.grpc` for
/// gRPC. TOML is left to environments and the two manifests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Http,
    Grpc,
}

impl Format {
    fn of(name: &str) -> Option<Format> {
        if http_format::is_http_file(name) {
            return Some(Format::Http);
        }
        if grpc_format::is_grpc_file(name) {
            return Some(Format::Grpc);
        }
        None
    }

    fn for_kind(kind: &str) -> CoreResult<Format> {
        match kind {
            "http" | "graphql" => Ok(Format::Http),
            "grpc" => Ok(Format::Grpc),
            other => Err(CoreError::Unsupported(format!(
                "unsupported request kind: {other}"
            ))),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Http => "http",
            Format::Grpc => "grpc",
        }
    }
}

enum Doc {
    Http(HttpDoc),
    Grpc(GrpcDoc),
}

impl Doc {
    fn parse(format: Format, stem: &str, source: &str) -> CoreResult<Doc> {
        Ok(match format {
            Format::Http => Doc::Http(HttpDoc::parse(stem, source)?),
            Format::Grpc => Doc::Grpc(GrpcDoc::parse(stem, source)?),
        })
    }

    fn len(&self) -> usize {
        match self {
            Doc::Http(doc) => doc.len(),
            Doc::Grpc(doc) => doc.len(),
        }
    }

    fn index_of(&self, fragment: &str) -> CoreResult<usize> {
        match self {
            Doc::Http(doc) => doc.index_of(fragment),
            Doc::Grpc(doc) => doc.index_of(fragment),
        }
    }

    fn raw(&self, index: usize) -> CoreResult<SavedRequest> {
        Ok(match self {
            Doc::Http(doc) => doc.raw(index)?.request,
            Doc::Grpc(doc) => doc.raw(index)?.request,
        })
    }

    fn resolved(&self, index: usize) -> CoreResult<SavedRequest> {
        Ok(match self {
            Doc::Http(doc) => doc.resolved(index)?.request,
            Doc::Grpc(doc) => doc.resolved(index)?.request,
        })
    }

    fn replace(&self, index: usize, request: &SavedRequest) -> CoreResult<String> {
        match self {
            Doc::Http(doc) => doc.replace(index, request),
            Doc::Grpc(doc) => doc.replace(index, request),
        }
    }

    fn remove(&self, index: usize) -> CoreResult<String> {
        match self {
            Doc::Http(doc) => doc.remove(index),
            Doc::Grpc(doc) => doc.remove(index),
        }
    }
}

fn render_one(request: &SavedRequest) -> CoreResult<String> {
    Ok(match Format::for_kind(&request.kind)? {
        Format::Http => http_format::render_request(request, "\n")?,
        Format::Grpc => grpc_format::render_request(request, "\n")?,
    })
}

/// `auth/login.http#2` addresses one request inside a file. The fragment is an
/// index — the canonical, always-unique form — or a block name when that name
/// occurs exactly once.
fn split_identity(path: &str) -> (&str, Option<&str>) {
    match path.rsplit_once('#') {
        Some((file, fragment)) if Format::of(file).is_some() => (file, Some(fragment)),
        _ => (path, None),
    }
}

fn read_doc(file: &Path, rel: &str) -> CoreResult<(Format, Doc)> {
    let format = Format::of(rel).ok_or_else(|| {
        CoreError::Unsupported(format!(
            "{rel} is not a request file — Mándalo stores HTTP and GraphQL in .http and gRPC in .grpc"
        ))
    })?;
    if !file.is_file() {
        return Err(CoreError::NotFound(format!("unknown request: {rel}")));
    }
    let raw = std::fs::read_to_string(file).map_err(|e| CoreError::io(file.display(), e))?;
    Ok((format, Doc::parse(format, rel, &raw)?))
}

fn index_in(doc: &Doc, fragment: Option<&str>, rel: &str) -> CoreResult<usize> {
    match fragment {
        Some(fragment) => doc.index_of(fragment),
        None if doc.len() == 1 => Ok(0),
        None => Err(CoreError::NotFound(format!(
            "{rel} holds {} requests — address one of them as {rel}#0",
            doc.len()
        ))),
    }
}

fn rel_string(root: &Path, path: &Path) -> CoreResult<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|e| CoreError::io(path.display(), e))?;
    let mut out = String::new();
    for part in rel.components() {
        let part = part.as_os_str().to_str().ok_or_else(|| {
            CoreError::InvalidName(format!("path is not valid UTF-8: {}", path.display()))
        })?;
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    Ok(out)
}

pub fn list_collections(workspace: &Path) -> CoreResult<CollectionList> {
    let dir = collections_dir(workspace);
    let mut list = CollectionList {
        items: Vec::new(),
        skipped: Vec::new(),
    };
    if !dir.exists() {
        return Ok(list);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        if !path.is_dir() {
            continue;
        }
        let Some(slug) = path.file_name().and_then(|n| n.to_str()) else {
            list.skipped
                .push(format!("{}: name is not valid UTF-8", path.display()));
            continue;
        };
        if slug.starts_with('.') {
            continue;
        }
        match read_collection_manifest(&path) {
            Ok(manifest) => list.items.push(CollectionInfo {
                id: manifest.id,
                slug: slug.to_string(),
                name: manifest.name,
            }),
            Err(e) => list.skipped.push(e.to_string()),
        }
    }
    list.items.sort_by(|a, b| a.name.cmp(&b.name));
    list.skipped.sort();
    Ok(list)
}

pub fn create_collection(workspace: &Path, name: &str) -> CoreResult<CollectionInfo> {
    if name.trim().is_empty() {
        return Err(CoreError::InvalidName(
            "collection name must not be empty".to_string(),
        ));
    }
    let root = collections_dir(workspace);
    std::fs::create_dir_all(&root).map_err(|e| CoreError::io(root.display(), e))?;
    let slug = unique_dir_slug(&root, &slugify(name));
    let dir = root.join(&slug);
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(dir.display(), e))?;
    let manifest = CollectionManifest {
        schema_version: SCHEMA_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
    };
    write_collection_manifest(&dir, &manifest)?;
    Ok(CollectionInfo {
        id: manifest.id,
        slug,
        name: manifest.name,
    })
}

pub fn ensure_collection(
    workspace: &Path,
    slug: &str,
    name: &str,
    id: Option<&str>,
) -> CoreResult<CollectionInfo> {
    validate_slug(slug)?;
    if name.trim().is_empty() {
        return Err(CoreError::InvalidName(
            "collection name must not be empty".to_string(),
        ));
    }
    let dir = collections_dir(workspace).join(slug);
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(dir.display(), e))?;
    let manifest = match manifest_path(&dir).exists() {
        true => read_collection_manifest(&dir)?,
        false => {
            let manifest = CollectionManifest {
                schema_version: SCHEMA_VERSION,
                id: id
                    .map(String::from)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: name.to_string(),
            };
            write_collection_manifest(&dir, &manifest)?;
            manifest
        }
    };
    Ok(CollectionInfo {
        id: manifest.id,
        slug: slug.to_string(),
        name: manifest.name,
    })
}

pub fn rename_collection(workspace: &Path, slug: &str, name: &str) -> CoreResult<CollectionInfo> {
    if name.trim().is_empty() {
        return Err(CoreError::InvalidName(
            "collection name must not be empty".to_string(),
        ));
    }
    let dir = collection_dir(workspace, slug)?;
    let manifest = read_collection_manifest(&dir)?;
    let root = collections_dir(workspace);
    let wanted = slugify(name);
    let new_slug = if wanted == slug {
        slug.to_string()
    } else {
        unique_dir_slug(&root, &wanted)
    };
    let new_dir = root.join(&new_slug);
    if new_dir != dir {
        std::fs::rename(&dir, &new_dir).map_err(|e| CoreError::io(new_dir.display(), e))?;
    }
    let manifest = CollectionManifest {
        schema_version: SCHEMA_VERSION,
        id: manifest.id,
        name: name.to_string(),
    };
    write_collection_manifest(&new_dir, &manifest)?;
    Ok(CollectionInfo {
        id: manifest.id,
        slug: new_slug,
        name: manifest.name,
    })
}

pub fn delete_collection(workspace: &Path, slug: &str) -> CoreResult<()> {
    let dir = collection_dir(workspace, slug)?;
    std::fs::remove_dir_all(&dir).map_err(|e| CoreError::io(dir.display(), e))
}

/// One file yields one summary per request inside it, each addressed by
/// `<path>#<index>`.
fn summaries(root: &Path, path: &Path, format: Format) -> CoreResult<Vec<RequestSummary>> {
    let rel = rel_string(root, path)?;
    let raw = std::fs::read_to_string(path).map_err(|e| CoreError::io(path.display(), e))?;
    let doc = Doc::parse(format, &rel, &raw).map_err(|e| match e {
        CoreError::Parse(m) => CoreError::Parse(format!("{rel}: {m}")),
        CoreError::Unsupported(m) => CoreError::Unsupported(format!("{rel}: {m}")),
        CoreError::PathEscape(m) => CoreError::PathEscape(format!("{rel}: {m}")),
        other => other,
    })?;
    let mut out = Vec::with_capacity(doc.len());
    for index in 0..doc.len() {
        let request = doc.raw(index)?;
        out.push(RequestSummary {
            id: request.id,
            name: request.name,
            kind: request.kind,
            method: request.method,
            path: format!("{rel}#{index}"),
        });
    }
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    skipped: &mut Vec<String>,
) -> CoreResult<(Vec<FolderNode>, Vec<RequestSummary>)> {
    let mut folders = Vec::new();
    let mut requests = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            skipped.push(format!("{}: name is not valid UTF-8", path.display()));
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            let (child_folders, child_requests) = walk(root, &path, skipped)?;
            folders.push(FolderNode {
                name: name.to_string(),
                path: rel_string(root, &path)?,
                folders: child_folders,
                requests: child_requests,
            });
            continue;
        }
        if dir == root && name == MANIFEST_FILE {
            continue;
        }
        let Some(format) = Format::of(name) else {
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                skipped.push(format!(
                    "{}: requests are .http and .grpc files now — this TOML request has to be converted",
                    rel_string(root, &path)?
                ));
            }
            continue;
        };
        match summaries(root, &path, format) {
            Ok(found) => requests.extend(found),
            Err(e) => skipped.push(e.to_string()),
        }
    }
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    // Index order inside a file is the author's order, so only the file part sorts.
    requests.sort_by(|a, b| {
        let file = |p: &str| {
            p.rsplit_once('#')
                .map(|(f, _)| f.to_string())
                .unwrap_or_default()
        };
        file(&a.path).cmp(&file(&b.path))
    });
    Ok((folders, requests))
}

pub fn list_tree(workspace: &Path) -> CoreResult<Tree> {
    let listed = list_collections(workspace)?;
    let mut tree = Tree {
        collections: Vec::new(),
        skipped: listed.skipped,
    };
    for info in listed.items {
        let dir = collections_dir(workspace).join(&info.slug);
        let (folders, requests) = walk(&dir, &dir, &mut tree.skipped)?;
        tree.collections.push(CollectionNode {
            id: info.id,
            slug: info.slug,
            name: info.name,
            folders,
            requests,
        });
    }
    tree.skipped.sort();
    Ok(tree)
}

fn validate_request(request: &SavedRequest) -> CoreResult<()> {
    validate_id(&request.id)?;
    if request.name.trim().is_empty() {
        return Err(CoreError::InvalidName(
            "request name must not be empty".to_string(),
        ));
    }
    for capture in &request.captures {
        validate_capture(capture)?;
    }
    Ok(())
}

/// Saves one request. An existing request is edited in place, so only the spans
/// that changed move and every comment, blank line and alignment around them
/// survives; a new one gets its own file.
pub fn save_request(
    workspace: &Path,
    slug: &str,
    path: Option<&str>,
    folder: Option<&str>,
    request: &SavedRequest,
) -> CoreResult<SavedPath> {
    validate_request(request)?;
    let format = Format::for_kind(&request.kind)?;
    let root = collection_dir(workspace, slug)?;

    if let Some(existing) = path {
        let (file, fragment) = split_identity(existing);
        let target = resolve_within(&root, file)?;
        let (found, doc) = read_doc(&target, file)?;
        if found != format {
            return Err(CoreError::Unsupported(format!(
                "{file} holds {} requests, so a {} request cannot be saved into it",
                found.extension(),
                request.kind
            )));
        }
        let index = index_in(&doc, fragment, file)?;
        let updated = doc.replace(index, request)?;
        crate::workspace::atomic_write(&target, &updated)?;
        return Ok(SavedPath {
            path: format!("{}#{index}", rel_string(&root, &target)?),
        });
    }

    let dir = resolve_within(&root, folder.unwrap_or(""))?;
    if !dir.is_dir() {
        return Err(CoreError::NotFound(format!(
            "unknown folder: {}",
            folder.unwrap_or("")
        )));
    }
    let target = unique_request_file(&dir, &slugify(&request.name), format.extension(), None);
    crate::workspace::atomic_write(&target, &render_one(request)?)?;
    Ok(SavedPath {
        path: format!("{}#0", rel_string(&root, &target)?),
    })
}

/// Writes a request to an exact path, creating the file if it is not there. The
/// extension has to agree with the request kind — a silent rewrite into the other
/// format would lose whatever the format cannot express.
pub fn put_request(
    workspace: &Path,
    slug: &str,
    path: &str,
    request: &SavedRequest,
) -> CoreResult<SavedPath> {
    validate_request(request)?;
    let format = Format::for_kind(&request.kind)?;
    let root = collection_dir(workspace, slug)?;
    let (file, fragment) = split_identity(path);
    let target = resolve_within(&root, file)?;
    match Format::of(file) {
        Some(found) if found == format => {}
        Some(found) => {
            return Err(CoreError::InvalidName(format!(
                "a {} request cannot be written to a .{} file: {path}",
                request.kind,
                found.extension()
            )))
        }
        None => {
            return Err(CoreError::InvalidName(format!(
                "a {} request path must end in .{}: {path}",
                request.kind,
                format.extension()
            )))
        }
    }
    let parent = target
        .parent()
        .ok_or_else(|| CoreError::Io(format!("request has no parent directory: {path}")))?;
    std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;

    if target.is_file() {
        let (_, doc) = read_doc(&target, file)?;
        let index = index_in(&doc, fragment, file)?;
        let updated = doc.replace(index, request)?;
        crate::workspace::atomic_write(&target, &updated)?;
        return Ok(SavedPath {
            path: format!("{}#{index}", rel_string(&root, &target)?),
        });
    }
    crate::workspace::atomic_write(&target, &render_one(request)?)?;
    Ok(SavedPath {
        path: format!("{}#0", rel_string(&root, &target)?),
    })
}

/// The request the runner sends: file-scoped `@vars` already applied, and gRPC
/// proto paths resolved against the workspace and proven to be inside it.
pub fn load_request(workspace: &Path, slug: &str, path: &str) -> CoreResult<SavedRequest> {
    let root = collection_dir(workspace, slug)?;
    let (file, fragment) = split_identity(path);
    let target = resolve_within(&root, file)?;
    let (_, doc) = read_doc(&target, file)?;
    let index = index_in(&doc, fragment, file)?;
    let mut request = doc.resolved(index)?;
    if let Some(grpc) = request.grpc.as_mut() {
        let mut resolved = Vec::with_capacity(grpc.proto_paths.len());
        for rel in &grpc.proto_paths {
            if rel.contains("{{") {
                return Err(CoreError::Unsupported(format!(
                    "{file}: a proto path that is only known at send time cannot be checked against the workspace — write {rel:?} as a workspace-relative path"
                )));
            }
            let found = crate::body::resolve_file(Some(workspace), rel)?;
            resolved.push(found.to_string_lossy().into_owned());
        }
        grpc.proto_paths = resolved;
    }
    Ok(request)
}

/// The request exactly as written, `{{vars}}` intact. This is what an editor loads:
/// the resolved view would bake the file's own variables into literals the moment
/// it was saved back.
pub fn load_request_source(workspace: &Path, slug: &str, path: &str) -> CoreResult<SavedRequest> {
    let root = collection_dir(workspace, slug)?;
    let (file, fragment) = split_identity(path);
    let target = resolve_within(&root, file)?;
    let (_, doc) = read_doc(&target, file)?;
    let index = index_in(&doc, fragment, file)?;
    doc.raw(index)
}

/// Deletes one request. A path with no fragment deletes the whole file; deleting
/// the last request in a file deletes the file with it.
pub fn delete_request(workspace: &Path, slug: &str, path: &str) -> CoreResult<()> {
    let root = collection_dir(workspace, slug)?;
    let (file, fragment) = split_identity(path);
    let target = resolve_within(&root, file)?;
    if !target.is_file() {
        return Err(CoreError::NotFound(format!("unknown request: {path}")));
    }
    let Some(fragment) = fragment else {
        return std::fs::remove_file(&target).map_err(|e| CoreError::io(target.display(), e));
    };
    let (_, doc) = read_doc(&target, file)?;
    let index = doc.index_of(fragment)?;
    if doc.len() == 1 {
        return std::fs::remove_file(&target).map_err(|e| CoreError::io(target.display(), e));
    }
    let updated = doc.remove(index)?;
    crate::workspace::atomic_write(&target, &updated)
}

pub fn create_folder(workspace: &Path, slug: &str, path: &str) -> CoreResult<()> {
    let root = collection_dir(workspace, slug)?;
    let target = resolve_within(&root, path)?;
    if target == root {
        return Err(CoreError::InvalidName(
            "folder path must not be empty".to_string(),
        ));
    }
    if target.exists() {
        return Err(CoreError::Conflict(format!(
            "folder already exists: {path}"
        )));
    }
    std::fs::create_dir_all(&target).map_err(|e| CoreError::io(target.display(), e))
}

pub fn create_folder_named(
    workspace: &Path,
    slug: &str,
    parent: &str,
    name: &str,
) -> CoreResult<SavedPath> {
    let root = collection_dir(workspace, slug)?;
    let parent_dir = resolve_within(&root, parent)?;
    if !parent_dir.is_dir() {
        return Err(CoreError::NotFound(format!("unknown folder: {parent}")));
    }
    let dir = parent_dir.join(unique_dir_slug(&parent_dir, &slugify(name)));
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(dir.display(), e))?;
    Ok(SavedPath {
        path: rel_string(&root, &dir)?,
    })
}

pub fn delete_folder(workspace: &Path, slug: &str, path: &str) -> CoreResult<()> {
    let root = collection_dir(workspace, slug)?;
    let target = resolve_within(&root, path)?;
    if target == root {
        return Err(CoreError::InvalidName(
            "folder path must not be empty".to_string(),
        ));
    }
    if !target.is_dir() {
        return Err(CoreError::NotFound(format!("unknown folder: {path}")));
    }
    std::fs::remove_dir_all(&target).map_err(|e| CoreError::io(target.display(), e))
}

pub fn rename_folder(
    workspace: &Path,
    slug: &str,
    path: &str,
    name: &str,
) -> CoreResult<SavedPath> {
    validate_component(name)?;
    let root = collection_dir(workspace, slug)?;
    let target = resolve_within(&root, path)?;
    if target == root {
        return Err(CoreError::InvalidName(
            "folder path must not be empty".to_string(),
        ));
    }
    if !target.is_dir() {
        return Err(CoreError::NotFound(format!("unknown folder: {path}")));
    }
    let parent = target
        .parent()
        .ok_or_else(|| CoreError::Io(format!("folder has no parent directory: {path}")))?;
    let renamed = parent.join(name);
    if renamed != target {
        if renamed.exists() {
            return Err(CoreError::Conflict(format!(
                "folder already exists: {name}"
            )));
        }
        std::fs::rename(&target, &renamed).map_err(|e| CoreError::io(renamed.display(), e))?;
    }
    Ok(SavedPath {
        path: rel_string(&root, &renamed)?,
    })
}

pub fn move_request(
    workspace: &Path,
    slug: &str,
    from: &str,
    to_folder: &str,
) -> CoreResult<SavedPath> {
    let root = collection_dir(workspace, slug)?;
    let (file, fragment) = split_identity(from);
    let source = resolve_within(&root, file)?;
    if !source.is_file() {
        return Err(CoreError::NotFound(format!("unknown request: {from}")));
    }
    let dir = resolve_within(&root, to_folder)?;
    if !dir.is_dir() {
        return Err(CoreError::NotFound(format!("unknown folder: {to_folder}")));
    }
    let format = Format::of(file)
        .ok_or_else(|| CoreError::Unsupported(format!("{file} is not a request file")))?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| CoreError::InvalidName(format!("request has no file name: {from}")))?;

    // Without a fragment the whole file moves; with one, that request is cut out of
    // its file and lands in its own file in the target folder.
    let Some(fragment) = fragment else {
        let target = unique_request_file(&dir, stem, format.extension(), Some(&source));
        if target != source {
            std::fs::rename(&source, &target).map_err(|e| CoreError::io(target.display(), e))?;
        }
        return Ok(SavedPath {
            path: rel_string(&root, &target)?,
        });
    };

    let (_, doc) = read_doc(&source, file)?;
    let index = doc.index_of(fragment)?;
    let request = doc.raw(index)?;
    if doc.len() == 1 && source.parent() == Some(dir.as_path()) {
        return Ok(SavedPath {
            path: format!("{}#0", rel_string(&root, &source)?),
        });
    }
    let target = unique_request_file(&dir, &slugify(&request.name), format.extension(), None);
    crate::workspace::atomic_write(&target, &render_one(&request)?)?;
    if doc.len() == 1 {
        std::fs::remove_file(&source).map_err(|e| CoreError::io(source.display(), e))?;
    } else {
        crate::workspace::atomic_write(&source, &doc.remove(index)?)?;
    }
    Ok(SavedPath {
        path: format!("{}#0", rel_string(&root, &target)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::{CaptureScope, StatusOp};
    use crate::body::{FormDataRow, FormRow, RawLanguage};
    use crate::workspace;

    fn workspace_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(collections_dir(dir.path())).unwrap();
        workspace::write_manifest(
            dir.path(),
            &workspace::Manifest {
                schema_version: SCHEMA_VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                name: "Test".to_string(),
            },
        )
        .unwrap();
        dir
    }

    fn request(name: &str) -> SavedRequest {
        SavedRequest {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: "http".to_string(),
            method: "POST".to_string(),
            url: "https://x.dev/users".to_string(),
            description: None,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            auth: Auth::Bearer {
                token: "{{token}}".to_string(),
            },
            body: Body::json("{\"a\": 1}"),
            grpc: None,
            scripts: Scripts::default(),
            tests: Vec::new(),
            captures: Vec::new(),
        }
    }

    fn save_new(ws: &Path, slug: &str, folder: Option<&str>, request: &SavedRequest) -> String {
        save_request(ws, slug, None, folder, request).unwrap().path
    }

    #[test]
    fn slugify_normalizes_and_collapses() {
        assert_eq!(slugify("Acme API"), "acme-api");
        assert_eq!(slugify("  List / Users  "), "list-users");
        assert_eq!(slugify("ÜBER"), "ber");
        assert_eq!(slugify("···"), "untitled");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("v2.1"), "v2-1");
    }

    #[test]
    fn create_and_list_collections() {
        let ws = workspace_dir();
        let acme = create_collection(ws.path(), "Acme API").unwrap();
        let other = create_collection(ws.path(), "Beta").unwrap();
        assert_eq!(acme.slug, "acme-api");
        assert!(uuid::Uuid::parse_str(&acme.id).is_ok());
        let list = list_collections(ws.path()).unwrap();
        assert_eq!(list.items, vec![acme, other]);
        assert!(list.skipped.is_empty());
    }

    #[test]
    fn collection_slug_collisions_are_deduped() {
        let ws = workspace_dir();
        assert_eq!(
            create_collection(ws.path(), "Acme API").unwrap().slug,
            "acme-api"
        );
        assert_eq!(
            create_collection(ws.path(), "acme  api").unwrap().slug,
            "acme-api-2"
        );
        assert_eq!(
            create_collection(ws.path(), "ACME-API").unwrap().slug,
            "acme-api-3"
        );
    }

    #[test]
    fn empty_collection_name_fails_loud() {
        let ws = workspace_dir();
        assert!(create_collection(ws.path(), "   ").is_err());
    }

    #[test]
    fn collection_without_manifest_is_skipped_not_fatal() {
        let ws = workspace_dir();
        create_collection(ws.path(), "Good").unwrap();
        std::fs::create_dir_all(collections_dir(ws.path()).join("orphan")).unwrap();
        let list = list_collections(ws.path()).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.skipped.len(), 1);
        assert!(list.skipped[0].contains("orphan"));
    }

    #[test]
    fn rename_collection_moves_the_directory() {
        let ws = workspace_dir();
        let created = create_collection(ws.path(), "Acme API").unwrap();
        save_new(ws.path(), &created.slug, None, &request("Ping"));
        let renamed = rename_collection(ws.path(), &created.slug, "Zeta Corp").unwrap();
        assert_eq!(renamed.slug, "zeta-corp");
        assert_eq!(renamed.id, created.id);
        assert!(!collections_dir(ws.path()).join("acme-api").exists());
        assert_eq!(
            list_tree(ws.path()).unwrap().collections[0].requests[0].name,
            "Ping"
        );
    }

    #[test]
    fn rename_collection_to_same_name_keeps_slug() {
        let ws = workspace_dir();
        let created = create_collection(ws.path(), "Acme API").unwrap();
        let renamed = rename_collection(ws.path(), &created.slug, "Acme API").unwrap();
        assert_eq!(renamed.slug, "acme-api");
    }

    #[test]
    fn delete_collection_removes_everything() {
        let ws = workspace_dir();
        let created = create_collection(ws.path(), "Acme").unwrap();
        save_new(ws.path(), &created.slug, None, &request("Ping"));
        delete_collection(ws.path(), &created.slug).unwrap();
        assert!(list_collections(ws.path()).unwrap().items.is_empty());
    }

    #[test]
    fn unknown_collection_fails_loud() {
        let ws = workspace_dir();
        assert!(delete_collection(ws.path(), "nope")
            .unwrap_err()
            .to_string()
            .contains("unknown collection"));
        assert!(load_request(ws.path(), "nope", "a.http").is_err());
    }

    #[test]
    fn collection_slug_traversal_rejected() {
        let ws = workspace_dir();
        for slug in ["../evil", "", "Acme", "a/b", "..", "a_b"] {
            assert!(
                delete_collection(ws.path(), slug).is_err(),
                "slug {slug:?} was accepted"
            );
        }
    }

    #[test]
    fn save_load_delete_roundtrip_at_root() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let req = request("List Users");
        let path = save_new(ws.path(), &c.slug, None, &req);
        assert_eq!(path, "list-users.http#0");
        assert_eq!(
            load_request(ws.path(), &c.slug, &path).unwrap(),
            SavedRequest {
                id: "list-users-http-0".to_string(),
                ..req.clone()
            }
        );
        delete_request(ws.path(), &c.slug, &path).unwrap();
        assert!(load_request(ws.path(), &c.slug, &path).is_err());
    }

    #[test]
    fn request_filename_collisions_are_deduped() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        assert_eq!(
            save_new(ws.path(), &c.slug, None, &request("Ping")),
            "ping.http#0"
        );
        assert_eq!(
            save_new(ws.path(), &c.slug, None, &request("ping")),
            "ping-2.http#0"
        );
        assert_eq!(
            save_new(ws.path(), &c.slug, None, &request("PING")),
            "ping-3.http#0"
        );
    }

    #[test]
    fn request_named_collection_does_not_clobber_the_manifest() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let path = save_new(ws.path(), &c.slug, None, &request("Collection"));
        assert_eq!(path, "collection.http#0");
        assert_eq!(
            read_collection_manifest(&collections_dir(ws.path()).join(&c.slug))
                .unwrap()
                .name,
            "Acme"
        );
    }

    #[test]
    fn saving_into_folders_keeps_relative_paths() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        create_folder(ws.path(), &c.slug, "users/admin").unwrap();
        let path = save_new(ws.path(), &c.slug, Some("users/admin"), &request("List"));
        assert_eq!(path, "users/admin/list.http#0");
        assert_eq!(
            load_request(ws.path(), &c.slug, &path).unwrap().name,
            "List"
        );
    }

    #[test]
    fn saving_into_missing_folder_fails_loud() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let err = save_request(ws.path(), &c.slug, None, Some("nope"), &request("A"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown folder"));
    }

    #[test]
    fn updating_by_path_renames_the_request_and_keeps_the_file() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        let mut req = request("List");
        let path = save_new(ws.path(), &c.slug, Some("users"), &req);
        assert_eq!(path, "users/list.http#0");
        req.name = "List All".to_string();
        let saved = save_request(ws.path(), &c.slug, Some(&path), None, &req)
            .unwrap()
            .path;
        assert_eq!(saved, path);
        assert!(collections_dir(ws.path())
            .join(&c.slug)
            .join("users/list.http")
            .is_file());
        assert_eq!(
            load_request(ws.path(), &c.slug, &saved).unwrap().name,
            "List All"
        );
    }

    #[test]
    fn updating_without_renaming_keeps_the_same_file() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let mut req = request("List");
        let path = save_new(ws.path(), &c.slug, None, &req);
        req.url = "https://x.dev/v2".to_string();
        let again = save_request(ws.path(), &c.slug, Some(&path), None, &req)
            .unwrap()
            .path;
        assert_eq!(again, path);
        assert_eq!(
            load_request(ws.path(), &c.slug, &path).unwrap().url,
            "https://x.dev/v2"
        );
        assert_eq!(
            list_tree(ws.path()).unwrap().collections[0].requests.len(),
            1
        );
    }

    #[test]
    fn updating_an_unknown_path_fails_loud() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let err = save_request(ws.path(), &c.slug, Some("ghost.http"), None, &request("A"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown request"));
    }

    #[test]
    fn invalid_request_id_and_name_rejected() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let mut req = request("A");
        req.id = "../evil".to_string();
        assert!(save_request(ws.path(), &c.slug, None, None, &req)
            .unwrap_err()
            .to_string()
            .contains("invalid request id"));
        let mut req = request("  ");
        req.id = "ok".to_string();
        assert!(save_request(ws.path(), &c.slug, None, None, &req)
            .unwrap_err()
            .to_string()
            .contains("name must not be empty"));
    }

    #[test]
    fn invalid_capture_is_rejected_on_save() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let mut req = request("A");
        req.captures = vec![Capture {
            from: "cookie.session".to_string(),
            into: "session".to_string(),
            scope: CaptureScope::Run,
        }];
        assert!(save_request(ws.path(), &c.slug, None, None, &req)
            .unwrap_err()
            .to_string()
            .contains("invalid capture source"));
    }

    #[test]
    fn scripts_roundtrip_on_disk_and_declarative_tests_are_refused() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let mut req = request("Login");
        req.scripts = Scripts {
            pre: Some("pm.environment.set('a', 1)".to_string()),
            post: Some("console.log(pm.response.code)".to_string()),
        };
        let path = save_new(ws.path(), &c.slug, None, &req);
        assert_eq!(
            load_request(ws.path(), &c.slug, &path).unwrap(),
            SavedRequest {
                id: "login-http-0".to_string(),
                ..req.clone()
            }
        );

        let mut declarative = request("Asserted");
        declarative.tests = vec![TestAssertion::Status {
            op: StatusOp::Eq,
            value: 200,
        }];
        let mut capturing = request("Capturing");
        capturing.captures = vec![Capture {
            from: "body.$.token".to_string(),
            into: "token".to_string(),
            scope: CaptureScope::Session,
        }];
        for req in [declarative, capturing] {
            let err = save_request(ws.path(), &c.slug, None, None, &req)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("cannot carry declarative tests or captures"),
                "{err}"
            );
        }
    }

    #[test]
    fn grpc_and_graphql_requests_roundtrip() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
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
            metadata: vec![("x-trace".to_string(), "1".to_string())],
        });
        std::fs::create_dir_all(ws.path().join("protos")).unwrap();
        std::fs::write(ws.path().join("protos/echo.proto"), "syntax = \"proto3\";").unwrap();
        let mut gql = request("Gql");
        gql.kind = "graphql".to_string();
        gql.body = Body::graphql("{ ping }", "{}");
        let a = save_new(ws.path(), &c.slug, None, &grpc);
        let b = save_new(ws.path(), &c.slug, None, &gql);
        assert_eq!(
            load_request(ws.path(), &c.slug, &b).unwrap(),
            SavedRequest {
                id: "gql-http-0".to_string(),
                ..gql.clone()
            }
        );

        // `load_request` hands the runner an absolute, workspace-confined proto path;
        // the source view keeps the workspace-relative one the file wrote.
        assert_eq!(
            load_request_source(ws.path(), &c.slug, &a).unwrap(),
            SavedRequest {
                id: "echo-grpc-0".to_string(),
                ..grpc.clone()
            }
        );
        assert_eq!(
            load_request(ws.path(), &c.slug, &a)
                .unwrap()
                .grpc
                .unwrap()
                .proto_paths,
            vec![ws
                .path()
                .canonicalize()
                .unwrap()
                .join("protos/echo.proto")
                .to_string_lossy()
                .into_owned()]
        );

        let raw =
            std::fs::read_to_string(collections_dir(ws.path()).join(&c.slug).join("echo.grpc"))
                .unwrap();
        assert!(raw.contains("proto: protos/echo.proto"), "{raw}");
        assert!(raw.contains("/test.v1.Echo/Say"), "{raw}");
    }

    #[test]
    fn every_body_mode_the_text_format_carries_survives_a_roundtrip() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        for (kind, body) in [
            ("http", Body::None),
            (
                "http",
                Body::Raw {
                    language: RawLanguage::Text,
                    text: "hola".to_string(),
                },
            ),
            (
                "http",
                Body::Raw {
                    language: RawLanguage::Json,
                    text: "{\"a\": 1}".to_string(),
                },
            ),
            (
                "http",
                Body::Binary {
                    file: "files/report.pdf".to_string(),
                    content_type: None,
                },
            ),
            ("graphql", Body::graphql("{ ping }", "{\"id\": 1}")),
        ] {
            let mut req = request("Roundtrip");
            req.kind = kind.to_string();
            req.body = body.clone();
            let path = save_request(ws.path(), &c.slug, None, None, &req)
                .unwrap()
                .path;
            assert_eq!(
                load_request(ws.path(), &c.slug, &path).unwrap().body,
                body,
                "{body:?}"
            );
        }
    }

    #[test]
    fn a_form_body_the_text_format_cannot_write_is_refused() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        for body in [
            Body::Urlencoded {
                rows: vec![
                    FormRow::new("user", "ada"),
                    FormRow {
                        enabled: false,
                        description: Some("only for staging".to_string()),
                        ..FormRow::new("debug", "1")
                    },
                ],
            },
            Body::Formdata {
                rows: vec![
                    FormDataRow::text("caption", "hola"),
                    FormDataRow::file("avatar", "files/avatar.png"),
                    FormDataRow {
                        content_type: Some("application/pdf".to_string()),
                        ..FormDataRow::files("docs", ["files/a.pdf", "files/b.pdf"])
                    },
                ],
            },
        ] {
            let mut req = request("Refused");
            req.body = body.clone();
            let err = save_request(ws.path(), &c.slug, None, None, &req)
                .unwrap_err()
                .to_string();
            assert!(err.contains("a .http file"), "{body:?}: {err}");
        }
    }

    #[test]
    fn a_toml_request_left_in_a_collection_is_reported_not_read() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let dir = collections_dir(ws.path()).join(&c.slug);
        std::fs::write(
            dir.join("old.toml"),
            "id = \"old\"\nname = \"Old\"\nkind = \"http\"\nmethod = \"POST\"\nurl = \"https://x.dev/a\"\n",
        )
        .unwrap();

        let tree = list_tree(ws.path()).unwrap();
        assert!(tree.collections[0].requests.is_empty());
        assert_eq!(tree.skipped.len(), 1);
        assert!(tree.skipped[0].contains("old.toml"), "{:?}", tree.skipped);
        assert!(
            tree.skipped[0].contains("has to be converted"),
            "{:?}",
            tree.skipped
        );

        let err = load_request(ws.path(), &c.slug, "old.toml")
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a request file"), "{err}");
    }

    #[test]
    fn tree_reports_folders_requests_and_corrupt_files() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        create_folder(ws.path(), &c.slug, "users/admin").unwrap();
        save_new(ws.path(), &c.slug, None, &request("Root Ping"));
        save_new(ws.path(), &c.slug, Some("users"), &request("List"));
        save_new(ws.path(), &c.slug, Some("users/admin"), &request("Purge"));
        std::fs::write(
            collections_dir(ws.path()).join(&c.slug).join("bad.http"),
            "### Bad\nGETT https://x.dev/a\n",
        )
        .unwrap();

        let tree = list_tree(ws.path()).unwrap();
        assert_eq!(tree.collections.len(), 1);
        let node = &tree.collections[0];
        assert_eq!(node.slug, "acme");
        assert_eq!(node.requests.len(), 1);
        assert_eq!(node.requests[0].path, "root-ping.http#0");
        assert_eq!(node.requests[0].method, "POST");
        assert_eq!(node.requests[0].kind, "http");
        assert_eq!(node.folders.len(), 1);
        let users = &node.folders[0];
        assert_eq!(users.name, "users");
        assert_eq!(users.path, "users");
        assert_eq!(users.requests[0].path, "users/list.http#0");
        let admin = &users.folders[0];
        assert_eq!(admin.path, "users/admin");
        assert_eq!(admin.requests[0].path, "users/admin/purge.http#0");
        assert_eq!(tree.skipped.len(), 1);
        assert!(tree.skipped[0].contains("bad.http"), "{:?}", tree.skipped);
    }

    #[test]
    fn one_file_holding_many_requests_is_listed_as_one_row_each() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        std::fs::write(
            collections_dir(ws.path()).join(&c.slug).join("auth.http"),
            "### Login\nPOST https://x.dev/login\n\n### Me\nGET https://x.dev/me\n",
        )
        .unwrap();

        let node = &list_tree(ws.path()).unwrap().collections[0];
        assert_eq!(
            node.requests
                .iter()
                .map(|r| (r.name.as_str(), r.path.as_str()))
                .collect::<Vec<_>>(),
            vec![("Login", "auth.http#0"), ("Me", "auth.http#1")]
        );
        assert_eq!(
            load_request(ws.path(), &c.slug, "auth.http#1").unwrap().url,
            "https://x.dev/me"
        );
        assert_eq!(
            load_request(ws.path(), &c.slug, "auth.http#Me")
                .unwrap()
                .url,
            "https://x.dev/me"
        );

        delete_request(ws.path(), &c.slug, "auth.http#0").unwrap();
        let node = &list_tree(ws.path()).unwrap().collections[0];
        assert_eq!(node.requests.len(), 1);
        assert_eq!(node.requests[0].path, "auth.http#0");
        assert_eq!(node.requests[0].name, "Me");
    }

    #[test]
    fn tree_sorts_folders_and_requests_by_name() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "zulu").unwrap();
        create_folder(ws.path(), &c.slug, "alpha").unwrap();
        save_new(ws.path(), &c.slug, None, &request("Zeta"));
        save_new(ws.path(), &c.slug, None, &request("Alpha"));
        let node = &list_tree(ws.path()).unwrap().collections[0];
        assert_eq!(
            node.folders
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zulu"]
        );
        assert_eq!(
            node.requests
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Zeta"]
        );
    }

    #[test]
    fn folder_create_rename_delete() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        assert!(create_folder(ws.path(), &c.slug, "users")
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        save_new(ws.path(), &c.slug, Some("users"), &request("List"));
        let renamed = rename_folder(ws.path(), &c.slug, "users", "people").unwrap();
        assert_eq!(renamed.path, "people");
        assert_eq!(
            list_tree(ws.path()).unwrap().collections[0].folders[0].requests[0].path,
            "people/list.http#0"
        );
        delete_folder(ws.path(), &c.slug, "people").unwrap();
        assert!(list_tree(ws.path()).unwrap().collections[0]
            .folders
            .is_empty());
    }

    #[test]
    fn rename_folder_onto_existing_name_fails_loud() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "a").unwrap();
        create_folder(ws.path(), &c.slug, "b").unwrap();
        assert!(rename_folder(ws.path(), &c.slug, "a", "b")
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        assert!(rename_folder(ws.path(), &c.slug, "a", "../x").is_err());
    }

    #[test]
    fn folder_commands_reject_the_collection_root() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        assert!(create_folder(ws.path(), &c.slug, "").is_err());
        assert!(delete_folder(ws.path(), &c.slug, "/").is_err());
        assert!(rename_folder(ws.path(), &c.slug, "", "x").is_err());
    }

    #[test]
    fn move_request_between_folders_and_to_root() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        create_folder(ws.path(), &c.slug, "archive").unwrap();
        let path = save_new(ws.path(), &c.slug, Some("users"), &request("List"));
        let moved = move_request(ws.path(), &c.slug, &path, "archive")
            .unwrap()
            .path;
        assert_eq!(moved, "archive/list.http#0");
        let back = move_request(ws.path(), &c.slug, &moved, "").unwrap().path;
        assert_eq!(back, "list.http#0");
        assert_eq!(
            load_request(ws.path(), &c.slug, &back).unwrap().name,
            "List"
        );
    }

    #[test]
    fn move_request_dedupes_on_collision() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        create_folder(ws.path(), &c.slug, "users").unwrap();
        save_new(ws.path(), &c.slug, None, &request("List"));
        let path = save_new(ws.path(), &c.slug, Some("users"), &request("List"));
        let moved = move_request(ws.path(), &c.slug, &path, "").unwrap().path;
        assert_eq!(moved, "list-2.http#0");
    }

    #[test]
    fn move_request_into_itself_is_a_noop() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let path = save_new(ws.path(), &c.slug, None, &request("List"));
        assert_eq!(
            move_request(ws.path(), &c.slug, &path, "").unwrap().path,
            path
        );
    }

    #[test]
    fn move_request_to_missing_folder_fails_loud() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let path = save_new(ws.path(), &c.slug, None, &request("List"));
        assert!(move_request(ws.path(), &c.slug, &path, "ghost")
            .unwrap_err()
            .to_string()
            .contains("unknown folder"));
    }

    #[test]
    fn traversal_paths_are_rejected_everywhere() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let secret = ws.path().join("secret.toml");
        std::fs::write(&secret, "name = \"secret\"").unwrap();
        for path in [
            "../secret.toml",
            "../../etc/passwd",
            "users/../../secret.toml",
        ] {
            assert!(load_request(ws.path(), &c.slug, path).is_err(), "{path}");
            assert!(delete_request(ws.path(), &c.slug, path).is_err(), "{path}");
            assert!(create_folder(ws.path(), &c.slug, path).is_err(), "{path}");
            assert!(delete_folder(ws.path(), &c.slug, path).is_err(), "{path}");
            assert!(
                move_request(ws.path(), &c.slug, path, "").is_err(),
                "{path}"
            );
            assert!(
                save_request(ws.path(), &c.slug, None, Some(path), &request("A")).is_err(),
                "{path}"
            );
        }
        assert!(secret.exists());
    }

    #[test]
    fn absolute_paths_are_rejected() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let outside = ws.path().join("outside.toml");
        std::fs::write(&outside, "x = 1").unwrap();
        let absolute = outside.to_str().unwrap();
        assert!(delete_request(ws.path(), &c.slug, absolute).is_err());
        assert!(load_request(ws.path(), &c.slug, absolute).is_err());
        assert!(outside.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folder_escape_is_rejected() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.toml"), "x = 1").unwrap();
        std::os::unix::fs::symlink(
            outside.path(),
            collections_dir(ws.path()).join(&c.slug).join("escape"),
        )
        .unwrap();
        let err = load_request(ws.path(), &c.slug, "escape/secret.toml")
            .unwrap_err()
            .to_string();
        assert!(err.contains("escapes the collection"));
        assert!(delete_request(ws.path(), &c.slug, "escape/secret.toml").is_err());
        assert!(outside.path().join("secret.toml").exists());
    }

    #[test]
    fn dotfiles_are_invisible_to_the_tree() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let dir = collections_dir(ws.path()).join(&c.slug);
        std::fs::write(dir.join(".ping.toml.tmp"), "broken [").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let tree = list_tree(ws.path()).unwrap();
        assert!(tree.collections[0].requests.is_empty());
        assert!(tree.collections[0].folders.is_empty());
        assert!(tree.skipped.is_empty());
        assert!(create_folder(ws.path(), &c.slug, ".git/objects").is_err());
    }

    #[test]
    fn saving_leaves_no_temp_files() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        save_new(ws.path(), &c.slug, None, &request("Ping"));
        let mut names: Vec<String> = std::fs::read_dir(collections_dir(ws.path()).join(&c.slug))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, vec!["collection.toml", "ping.http"]);
    }

    #[test]
    fn put_request_creates_intermediate_folders() {
        let ws = workspace_dir();
        let c = create_collection(ws.path(), "Acme").unwrap();
        let req = request("List");
        let saved = put_request(ws.path(), &c.slug, "users/admin/list.http", &req).unwrap();
        assert_eq!(saved.path, "users/admin/list.http#0");
        assert_eq!(
            load_request(ws.path(), &c.slug, &saved.path).unwrap(),
            SavedRequest {
                id: "users-admin-list-http-0".to_string(),
                ..req.clone()
            }
        );
        assert!(put_request(ws.path(), &c.slug, "users/list", &req)
            .unwrap_err()
            .to_string()
            .contains("must end in .http"));
        assert!(put_request(ws.path(), &c.slug, "users/list.grpc", &req)
            .unwrap_err()
            .to_string()
            .contains("cannot be written to a .grpc file"));
        assert!(put_request(ws.path(), &c.slug, "../evil.http", &req).is_err());
    }
}
