use crate::capability::{SecretStore, SecretWriter, VarSource};
use crate::error::{CoreError, CoreResult};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct WorkspaceInfo {
    pub id: String,
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceList {
    pub items: Vec<WorkspaceInfo>,
    pub active: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOpen {
    pub workspace: WorkspaceInfo,
}

/// How Sync and Export present the workspace to the outside world. The on-disk
/// Mandalo files stay the source of truth; `Postman` adds a generated mirror.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ShareFormat {
    #[default]
    Native,
    Postman,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShareConfig {
    #[serde(default)]
    pub format: ShareFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

impl ShareConfig {
    pub fn postman() -> Self {
        ShareConfig {
            format: ShareFormat::Postman,
            dir: None,
        }
    }

    pub fn is_postman(&self) -> bool {
        matches!(self.format, ShareFormat::Postman)
    }

    /// Directory under the workspace root that holds generated Postman JSON.
    pub fn dir_name(&self) -> &str {
        match self.dir.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s,
            _ => "postman",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    /// Set only on a workspace that arrived from a link. Its presence is what
    /// makes the workspace read-only, so it has to survive every rewrite of this
    /// file — and it is serialized last, because TOML puts tables after values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<crate::remote::RemoteOrigin>,
    /// Optional share settings. Written by Export when the user picks Postman,
    /// or by hand in `mandalo.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<ShareConfig>,
}

#[derive(Serialize, Deserialize, Default)]
struct Registry {
    schema_version: u32,
    #[serde(default)]
    active: String,
    #[serde(default, rename = "workspace")]
    workspaces: Vec<WorkspaceInfo>,
}

pub fn registry_path() -> CoreResult<PathBuf> {
    Ok(config_dir()?.join("workspaces.toml"))
}

pub fn config_dir() -> CoreResult<PathBuf> {
    Ok(home()?.join(".config").join("mandalo"))
}

pub fn default_workspace_path() -> CoreResult<PathBuf> {
    Ok(home()?.join("Mandalo"))
}

fn home() -> CoreResult<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| CoreError::Io("HOME is not set".to_string()))
}

fn read_registry(registry: &Path) -> CoreResult<Registry> {
    if !registry.exists() {
        return Ok(Registry {
            schema_version: SCHEMA_VERSION,
            active: String::new(),
            workspaces: Vec::new(),
        });
    }
    let raw =
        std::fs::read_to_string(registry).map_err(|e| CoreError::io(registry.display(), e))?;
    let parsed: Registry =
        toml::from_str(&raw).map_err(|e| CoreError::parse(registry.display(), e))?;
    if parsed.schema_version != SCHEMA_VERSION {
        return Err(CoreError::Schema(format!(
            "{}: unsupported registry schema_version {} (expected {SCHEMA_VERSION})",
            registry.display(),
            parsed.schema_version
        )));
    }
    Ok(parsed)
}

fn write_registry(registry: &Path, value: &Registry) -> CoreResult<()> {
    let parent = registry.parent().ok_or_else(|| {
        CoreError::Io(format!(
            "registry path has no parent: {}",
            registry.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;
    let raw = toml::to_string_pretty(value).map_err(|e| CoreError::Parse(e.to_string()))?;
    atomic_write(registry, &raw)
}

fn same_path(a: &str, b: &Path) -> bool {
    let left = std::fs::canonicalize(a).unwrap_or_else(|_| PathBuf::from(a));
    let right = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    left == right
}

fn path_string(path: &Path) -> CoreResult<String> {
    path.to_str().map(String::from).ok_or_else(|| {
        CoreError::InvalidName(format!("path is not valid UTF-8: {}", path.display()))
    })
}

fn require_absolute(path: &Path) -> CoreResult<()> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidName(format!(
            "workspace path must be absolute: {}",
            path.display()
        )));
    }
    Ok(())
}

fn manifest_path(workspace: &Path) -> PathBuf {
    workspace.join("mandalo.toml")
}

pub fn read_manifest(workspace: &Path) -> CoreResult<Option<Manifest>> {
    let path = manifest_path(workspace);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| CoreError::io(path.display(), e))?;
    let manifest: Manifest =
        toml::from_str(&raw).map_err(|e| CoreError::parse(path.display(), e))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(CoreError::Schema(format!(
            "{}: unsupported workspace schema_version {} (expected {SCHEMA_VERSION})",
            path.display(),
            manifest.schema_version
        )));
    }
    Ok(Some(manifest))
}

pub fn write_manifest(workspace: &Path, manifest: &Manifest) -> CoreResult<()> {
    std::fs::create_dir_all(workspace).map_err(|e| CoreError::io(workspace.display(), e))?;
    let raw = toml::to_string_pretty(manifest).map_err(|e| CoreError::Parse(e.to_string()))?;
    atomic_write(&manifest_path(workspace), &raw)
}

/// Share settings from `mandalo.toml`, or `None` when the stanza is absent.
pub fn share_config(workspace: &Path) -> CoreResult<Option<ShareConfig>> {
    Ok(read_manifest(workspace)?.and_then(|m| m.share))
}

/// Persist `[share]` (or clear it when `share` is `None`). Preserves id/name/remote.
pub fn set_share(workspace: &Path, share: Option<ShareConfig>) -> CoreResult<Manifest> {
    let Some(mut manifest) = read_manifest(workspace)? else {
        return Err(CoreError::NotFound(format!(
            "{} is not a Mándalo workspace — there is no mandalo.toml in it",
            workspace.display()
        )));
    };
    manifest.share = share;
    write_manifest(workspace, &manifest)?;
    Ok(manifest)
}

/// Bookkeeping a directory can hold while still being empty of the user's own
/// work. `git init` then `mandalo init` is the way a git-native workspace starts,
/// so a repository with nothing in it yet must not read as somebody else's folder.
const NOT_CONTENT: [&str; 2] = [".git", ".DS_Store"];

fn dir_is_empty(path: &Path) -> CoreResult<bool> {
    let entries = std::fs::read_dir(path).map_err(|e| CoreError::io(path.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| CoreError::io(path.display(), e))?;
        let name = entry.file_name();
        if !NOT_CONTENT.iter().any(|skip| name == *skip) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// What `open_workspace` sees before writing anything.
#[derive(Debug, PartialEq, Eq)]
enum OpenKind {
    Workspace,
    Empty,
    OpenApiRoot,
    ChildWorkspaceHint { children: Vec<String> },
    ForeignNonEmpty,
}

fn child_workspace_names(path: &Path) -> CoreResult<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| CoreError::io(path.display(), e))? {
        let entry = entry.map_err(|e| CoreError::io(path.display(), e))?;
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        if child.join("mandalo.toml").is_file() {
            if let Some(name) = child.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with('.') {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

fn classify_open_target(path: &Path) -> CoreResult<OpenKind> {
    if read_manifest(path)?.is_some() {
        return Ok(OpenKind::Workspace);
    }
    if dir_is_empty(path)? {
        return Ok(OpenKind::Empty);
    }
    let children = child_workspace_names(path)?;
    if !children.is_empty() {
        return Ok(OpenKind::ChildWorkspaceHint { children });
    }
    if crate::openapi::has_root_openapi(path) {
        return Ok(OpenKind::OpenApiRoot);
    }
    Ok(OpenKind::ForeignNonEmpty)
}

fn open_kind_conflict(path: &Path, kind: &OpenKind) -> CoreError {
    match kind {
        OpenKind::ChildWorkspaceHint { children } => {
            let listed = children
                .iter()
                .map(|name| format!("{}/{name}", path.file_name().and_then(|n| n.to_str()).unwrap_or(".")))
                .collect::<Vec<_>>()
                .join(", ");
            CoreError::Conflict(format!(
                "{} is not a Mándalo workspace — it contains other workspaces ({listed}). Open one of those folders instead, or pick an empty directory",
                path.display()
            ))
        }
        OpenKind::ForeignNonEmpty => CoreError::Conflict(format!(
            "{} is not empty and is not a Mándalo workspace — open a folder that already has mandalo.toml, import an OpenAPI/Postman file into an existing workspace, or pick an empty directory",
            path.display()
        )),
        OpenKind::Workspace | OpenKind::Empty | OpenKind::OpenApiRoot => CoreError::Conflict(
            format!("{} cannot be opened as a workspace", path.display()),
        ),
    }
}

/// If the user picked `…/<ws>/collections` or `…/<ws>/environments` of an
/// existing workspace, open `<ws>` instead of scaffolding inside it.
fn resolve_workspace_dir(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path.to_path_buf();
    };
    if !matches!(name, "collections" | "environments") {
        return path.to_path_buf();
    }
    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    if parent.join("mandalo.toml").is_file() {
        parent.to_path_buf()
    } else {
        path.to_path_buf()
    }
}

fn directory_label(workspace: &Path) -> String {
    workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Workspace")
        .to_string()
}

fn scaffold_with(workspace: &Path, name: &str, preferred_id: Option<&str>) -> CoreResult<Manifest> {
    let manifest = match read_manifest(workspace)? {
        Some(existing) => Manifest {
            schema_version: SCHEMA_VERSION,
            id: existing.id,
            name: name.to_string(),
            remote: existing.remote,
            share: existing.share,
        },
        None => Manifest {
            schema_version: SCHEMA_VERSION,
            id: preferred_id
                .map(str::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: name.to_string(),
            remote: None,
            share: None,
        },
    };
    std::fs::create_dir_all(workspace.join("environments"))
        .map_err(|e| CoreError::io(workspace.display(), e))?;
    std::fs::create_dir_all(crate::collection::collections_dir(workspace))
        .map_err(|e| CoreError::io(workspace.display(), e))?;
    write_manifest(workspace, &manifest)?;
    Ok(manifest)
}

fn scaffold(workspace: &Path, name: &str) -> CoreResult<Manifest> {
    scaffold_with(workspace, name, None)
}

fn finish_open(workspace: &Path, manifest: Manifest) -> CoreResult<Manifest> {
    crate::collection::heal_nested_workspace_scaffold(workspace);
    let _ = crate::openapi::seed_if_empty(workspace)?;
    Ok(manifest)
}

/// Make a registered path usable: restore a missing `mandalo.toml` for empty or
/// OpenAPI-root folders, seed requests when needed, or fail loud for foreign dirs.
pub fn ensure_ready(registry: &Path, workspace: &Path) -> CoreResult<Manifest> {
    require_absolute(workspace)?;
    let workspace = resolve_workspace_dir(workspace);
    if !workspace.is_dir() {
        return Err(CoreError::NotFound(format!(
            "workspace directory does not exist: {}",
            workspace.display()
        )));
    }
    if let Some(existing) = read_manifest(&workspace)? {
        return finish_open(&workspace, existing);
    }
    let kind = classify_open_target(&workspace)?;
    match kind {
        OpenKind::Empty | OpenKind::OpenApiRoot => {
            let reg = read_registry(registry)?;
            let existing = reg
                .workspaces
                .iter()
                .find(|w| same_path(&w.path, &workspace));
            let preferred_id = existing.map(|w| w.id.clone());
            let name = existing
                .map(|w| w.name.clone())
                .unwrap_or_else(|| directory_label(&workspace));
            let manifest = scaffold_with(&workspace, &name, preferred_id.as_deref())?;
            finish_open(&workspace, manifest)
        }
        OpenKind::Workspace => Err(CoreError::NotFound(format!(
            "{} is not a Mándalo workspace yet — open it first",
            workspace.display()
        ))),
        other => Err(open_kind_conflict(&workspace, &other)),
    }
}

fn register(registry: &Path, workspace: &Path, manifest: &Manifest) -> CoreResult<WorkspaceInfo> {
    let mut reg = read_registry(registry)?;
    let info = WorkspaceInfo {
        id: manifest.id.clone(),
        path: path_string(workspace)?,
        name: manifest.name.clone(),
    };
    reg.workspaces
        .retain(|w| w.id != info.id && !same_path(&w.path, workspace));
    reg.workspaces.push(info.clone());
    reg.active = info.id.clone();
    reg.schema_version = SCHEMA_VERSION;
    write_registry(registry, &reg)?;
    Ok(info)
}

fn repair_registry(registry: &Path, mut reg: Registry) -> CoreResult<Registry> {
    let mut kept = Vec::new();
    let mut active_gone = false;
    for entry in reg.workspaces.drain(..) {
        let path = PathBuf::from(&entry.path);
        if !path.is_dir() {
            if entry.id == reg.active {
                active_gone = true;
            }
            continue;
        }
        if let Some(manifest) = read_manifest(&path)? {
            let _ = finish_open(&path, manifest)?;
            kept.push(entry);
            continue;
        }
        match classify_open_target(&path)? {
            OpenKind::Empty | OpenKind::OpenApiRoot => {
                let manifest = scaffold_with(&path, &entry.name, Some(&entry.id))?;
                let _ = finish_open(&path, manifest)?;
                kept.push(entry);
            }
            _ => {
                if entry.id == reg.active {
                    active_gone = true;
                }
            }
        }
    }
    reg.workspaces = kept;
    if reg.workspaces.is_empty() {
        reg.active.clear();
    } else if active_gone || !reg.workspaces.iter().any(|w| w.id == reg.active) {
        reg.active = reg.workspaces[0].id.clone();
    }
    write_registry(registry, &reg)?;
    Ok(reg)
}

pub fn list_workspaces(registry: &Path, default_path: &Path) -> CoreResult<WorkspaceList> {
    let reg = read_registry(registry)?;
    let reg = if reg.workspaces.is_empty() {
        reg
    } else {
        repair_registry(registry, reg)?
    };
    if reg.workspaces.is_empty() {
        let info = create_workspace(registry, default_path, "Personal")?;
        return Ok(WorkspaceList {
            active: info.id.clone(),
            items: vec![info],
        });
    }
    let active = if reg.workspaces.iter().any(|w| w.id == reg.active) {
        reg.active
    } else {
        reg.workspaces[0].id.clone()
    };
    Ok(WorkspaceList {
        items: reg.workspaces,
        active,
    })
}

pub fn create_workspace(
    registry: &Path,
    workspace: &Path,
    name: &str,
) -> CoreResult<WorkspaceInfo> {
    require_absolute(workspace)?;
    let workspace = resolve_workspace_dir(workspace);
    if name.trim().is_empty() {
        return Err(CoreError::InvalidName(
            "workspace name must not be empty".to_string(),
        ));
    }
    if workspace.exists() {
        if !workspace.is_dir() {
            return Err(CoreError::InvalidName(format!(
                "workspace path is not a directory: {}",
                workspace.display()
            )));
        }
        if !dir_is_empty(&workspace)? && read_manifest(&workspace)?.is_none() {
            let kind = classify_open_target(&workspace)?;
            match kind {
                OpenKind::OpenApiRoot => {}
                OpenKind::Empty => {}
                other => return Err(open_kind_conflict(&workspace, &other)),
            }
        }
    }
    let manifest = scaffold(&workspace, name)?;
    let manifest = finish_open(&workspace, manifest)?;
    register(registry, &workspace, &manifest)
}

pub fn open_workspace(registry: &Path, workspace: &Path) -> CoreResult<WorkspaceOpen> {
    require_absolute(workspace)?;
    let workspace = resolve_workspace_dir(workspace);
    if !workspace.is_dir() {
        return Err(CoreError::NotFound(format!(
            "workspace directory does not exist: {}",
            workspace.display()
        )));
    }
    let kind = classify_open_target(&workspace)?;
    match &kind {
        OpenKind::Workspace | OpenKind::Empty | OpenKind::OpenApiRoot => {}
        other => return Err(open_kind_conflict(&workspace, other)),
    }
    let preferred = read_registry(registry)?
        .workspaces
        .into_iter()
        .find(|w| same_path(&w.path, &workspace));
    let preferred_id = preferred.as_ref().map(|w| w.id.clone());
    let name = match read_manifest(&workspace)? {
        Some(existing) => existing.name,
        None => preferred
            .map(|w| w.name)
            .unwrap_or_else(|| directory_label(&workspace)),
    };
    let manifest = scaffold_with(&workspace, &name, preferred_id.as_deref())?;
    let manifest = finish_open(&workspace, manifest)?;
    let info = register(registry, &workspace, &manifest)?;
    Ok(WorkspaceOpen { workspace: info })
}

pub fn set_active_workspace(registry: &Path, id: &str) -> CoreResult<WorkspaceInfo> {
    let reg = read_registry(registry)?;
    let info = reg
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or_else(|| CoreError::NotFound(format!("unknown workspace id: {id}")))?;
    ensure_ready(registry, Path::new(&info.path))?;
    let mut reg = read_registry(registry)?;
    if !reg.workspaces.iter().any(|w| w.id == id) {
        return Err(CoreError::NotFound(format!(
            "workspace {id} could not be repaired — remove it and open the folder again"
        )));
    }
    reg.active = id.to_string();
    write_registry(registry, &reg)?;
    reg.workspaces
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| CoreError::NotFound(format!("unknown workspace id: {id}")))
}

pub fn remove_workspace(registry: &Path, id: &str) -> CoreResult<()> {
    let mut reg = read_registry(registry)?;
    if !reg.workspaces.iter().any(|w| w.id == id) {
        return Err(CoreError::NotFound(format!("unknown workspace id: {id}")));
    }
    reg.workspaces.retain(|w| w.id != id);
    if reg.active == id {
        reg.active = reg
            .workspaces
            .first()
            .map(|w| w.id.clone())
            .unwrap_or_default();
    }
    write_registry(registry, &reg)
}

pub fn resolve_workspace(registry: &Path, workspace: &str) -> CoreResult<PathBuf> {
    let as_path = PathBuf::from(workspace);
    if as_path.is_absolute() {
        return Ok(as_path);
    }
    let reg = read_registry(registry)?;
    reg.workspaces
        .iter()
        .find(|w| w.id == workspace)
        .map(|w| PathBuf::from(&w.path))
        .ok_or_else(|| CoreError::NotFound(format!("unknown workspace: {workspace}")))
}

/// A variable **declaration**, answering two orthogonal questions.
///
/// `shared` says *where the value lives* — in the committed file, or only on
/// this machine. `secret` says *how it is treated* — masked in the UI and
/// registered with the redactor. A colleague's `devUrl` is not shared and not
/// confidential; a token is both.
///
/// The type settles both by construction. [`VarDef::Shared`] is the only
/// variant with a `value` field, so a value that is not shared cannot be
/// serialized into a committed file at all; and there is no variant for a
/// shared secret, so `secret = true` implies `shared = false` with nothing to
/// override.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum VarDef {
    Shared { value: String },
    Local { hosts: Vec<String> },
    Secret { hosts: Vec<String> },
}

impl VarDef {
    pub fn secret(hosts: &[&str]) -> Self {
        VarDef::Secret {
            hosts: lowercased(hosts),
        }
    }

    pub fn local(hosts: &[&str]) -> Self {
        VarDef::Local {
            hosts: lowercased(hosts),
        }
    }

    pub fn shared(value: impl Into<String>) -> Self {
        VarDef::Shared {
            value: value.into(),
        }
    }

    /// Confidential: masked in every view, redacted out of every diagnostic.
    pub fn is_secret(&self) -> bool {
        matches!(self, VarDef::Secret { .. })
    }

    /// Its value belongs in the committed file.
    pub fn is_shared(&self) -> bool {
        matches!(self, VarDef::Shared { .. })
    }

    pub fn hosts(&self) -> Option<&[String]> {
        match self {
            VarDef::Shared { .. } => None,
            VarDef::Local { hosts } | VarDef::Secret { hosts } => Some(hosts),
        }
    }

    fn hosts_mut(&mut self) -> Option<&mut Vec<String>> {
        match self {
            VarDef::Shared { .. } => None,
            VarDef::Local { hosts } | VarDef::Secret { hosts } => Some(hosts),
        }
    }
}

fn lowercased(hosts: &[&str]) -> Vec<String> {
    hosts.iter().map(|h| h.to_ascii_lowercase()).collect()
}

impl Serialize for VarDef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            VarDef::Shared { value } => map.serialize_entry("value", value)?,
            VarDef::Local { hosts } => {
                map.serialize_entry("shared", &false)?;
                map.serialize_entry("hosts", hosts)?;
            }
            VarDef::Secret { hosts } => {
                map.serialize_entry("secret", &true)?;
                map.serialize_entry("hosts", hosts)?;
            }
        }
        map.end()
    }
}

/// The on-disk environment document: declarations only, safe to commit.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct EnvDoc {
    pub schema_version: u32,
    pub name: String,
    pub vars: BTreeMap<String, VarDef>,
}

impl EnvDoc {
    pub fn new(name: impl Into<String>) -> Self {
        EnvDoc {
            schema_version: SCHEMA_VERSION,
            name: name.into(),
            vars: BTreeMap::new(),
        }
    }

    /// The shared-value view the runner, the CLI and importers work with. A
    /// variable that is not shared is absent by construction — it has no value
    /// to give.
    pub fn shared_view(&self) -> Environment {
        Environment {
            name: self.name.clone(),
            vars: self
                .vars
                .iter()
                .filter_map(|(k, v)| match v {
                    VarDef::Shared { value } => Some((k.clone(), value.clone())),
                    VarDef::Local { .. } | VarDef::Secret { .. } => None,
                })
                .collect(),
        }
    }

    pub fn secret_hosts(&self, key: &str) -> Option<&[String]> {
        self.vars.get(key).and_then(VarDef::hosts)
    }
}

impl From<&Environment> for EnvDoc {
    fn from(env: &Environment) -> Self {
        EnvDoc {
            schema_version: SCHEMA_VERSION,
            name: env.name.clone(),
            vars: env
                .vars
                .iter()
                .map(|(k, v)| (k.clone(), VarDef::shared(v.clone())))
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawVar {
    Flat(String),
    Table(RawVarTable),
}

#[derive(Deserialize)]
struct RawVarTable {
    value: Option<String>,
    shared: Option<bool>,
    secret: Option<bool>,
    hosts: Option<Vec<String>>,
}

/// Every combination that cannot be honoured is an error, never a coercion: a
/// file that says something impossible about a credential is not a file to
/// guess at.
fn validate_var(context: &str, key: &str, t: RawVarTable) -> CoreResult<VarDef> {
    let secret = t.secret.unwrap_or(false);
    let hosts = lowercased(
        &t.hosts
            .clone()
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );

    if secret && t.shared == Some(true) {
        return Err(CoreError::Schema(format!(
            "{context}: variable {key:?} is declared secret = true and shared = true; \
             a secret is never shared — its value stays on this machine. Remove `shared = true`"
        )));
    }
    if secret && t.value.is_some() {
        return Err(CoreError::Schema(format!(
            "{context}: variable {key:?} is declared secret = true and also carries a value; \
             a secret value never lives in a workspace file — delete the value and store it with \
             `mandalo env set-secret`"
        )));
    }
    if t.shared == Some(false) && t.value.is_some() {
        return Err(CoreError::Schema(format!(
            "{context}: variable {key:?} is declared shared = false and also carries a value; \
             a value that is not shared never lives in a workspace file — delete the value and \
             store it with `mandalo env set-local`"
        )));
    }
    if secret {
        return Ok(VarDef::Secret { hosts });
    }
    if t.shared == Some(false) {
        return Ok(VarDef::Local { hosts });
    }
    match t.value {
        Some(value) => {
            if t.hosts.is_some() {
                return Err(CoreError::Schema(format!(
                    "{context}: variable {key:?} binds hosts but is shared; \
                     a value already committed to git has nothing left to protect. \
                     Declare it `secret = true` or `shared = false` to bind it to a host"
                )));
            }
            Ok(VarDef::Shared { value })
        }
        None => Err(CoreError::Schema(format!(
            "{context}: variable {key:?} has none of a value, `secret = true` or `shared = false`"
        ))),
    }
}

#[derive(Deserialize)]
pub struct RawEnvDoc {
    schema_version: Option<u32>,
    name: String,
    #[serde(default)]
    vars: BTreeMap<String, RawVar>,
}

impl RawEnvDoc {
    /// Shape errors are `E_PARSE` (the caller may skip the file); everything this
    /// returns is `E_SCHEMA` — a file that parses but declares something forbidden.
    pub fn validate(self, context: &str) -> CoreResult<EnvDoc> {
        if let Some(version) = self.schema_version {
            if version != SCHEMA_VERSION {
                return Err(CoreError::Schema(format!(
                    "{context}: unsupported environment schema_version {version} (expected {SCHEMA_VERSION})"
                )));
            }
        }
        let mut vars = BTreeMap::new();
        for (key, raw) in self.vars {
            let def = match raw {
                RawVar::Flat(value) => VarDef::Shared { value },
                RawVar::Table(t) => validate_var(context, &key, t)?,
            };
            vars.insert(key, def);
        }
        Ok(EnvDoc {
            schema_version: SCHEMA_VERSION,
            name: self.name,
            vars,
        })
    }
}

/// The resolved plain-value view of an environment. Never carries a secret value.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentList {
    pub items: Vec<Environment>,
    pub skipped: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct EnvDocList {
    pub items: Vec<EnvDoc>,
    pub skipped: Vec<String>,
}

/// What a UI may know about a variable. `value` is the **committed** value and
/// is `None` for anything not shared, so what the editor shows is what saving
/// writes back. `set` says whether a value is reachable at all, and `source`
/// says from where.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VarInfo {
    pub shared: bool,
    pub secret: bool,
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    pub set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<VarSource>,
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentView {
    pub name: String,
    pub vars: BTreeMap<String, VarInfo>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentViewList {
    pub items: Vec<EnvironmentView>,
    pub skipped: Vec<String>,
}

impl EnvironmentView {
    pub fn build(doc: &EnvDoc, secrets: &dyn SecretStore) -> CoreResult<Self> {
        let mut vars = BTreeMap::new();
        for (key, def) in &doc.vars {
            let held = secrets.source(&doc.name, key)?;
            let info = match def {
                VarDef::Shared { value } => VarInfo {
                    shared: true,
                    secret: false,
                    value: Some(value.clone()),
                    hosts: None,
                    set: true,
                    source: held.or(Some(VarSource::File)),
                },
                VarDef::Local { hosts } | VarDef::Secret { hosts } => VarInfo {
                    shared: false,
                    secret: def.is_secret(),
                    value: None,
                    hosts: Some(hosts.clone()),
                    set: held.is_some(),
                    source: held,
                },
            };
            vars.insert(key.clone(), info);
        }
        Ok(EnvironmentView {
            name: doc.name.clone(),
            vars,
        })
    }
}

fn environments_dir(workspace: &Path) -> PathBuf {
    workspace.join("environments")
}

pub fn validate_env_name(name: &str) -> CoreResult<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid environment name: {name:?}"
        )));
    }
    Ok(())
}

pub fn sanitize_env_name(workspace: &Path, name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let sanitized = if sanitized.is_empty() {
        "imported".to_string()
    } else {
        sanitized
    };
    if sanitized == name || !env_path(workspace, &sanitized).exists() {
        return sanitized;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{sanitized}-{n}");
        if !env_path(workspace, &candidate).exists() {
            return candidate;
        }
        n += 1;
    }
}

fn env_path(workspace: &Path, name: &str) -> PathBuf {
    environments_dir(workspace).join(format!("{name}.toml"))
}

pub fn atomic_write(path: &Path, contents: &str) -> CoreResult<()> {
    let parent = path.parent().ok_or_else(|| {
        CoreError::Io(format!("path has no parent directory: {}", path.display()))
    })?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| CoreError::InvalidName(format!(
                "path has no valid file name: {}",
                path.display()
            )))?
    ));
    std::fs::write(&tmp, contents).map_err(|e| CoreError::io(tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| CoreError::io(path.display(), e))
}

pub fn validate_var_name(name: &str) -> CoreResult<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid variable name: {name:?}"
        )));
    }
    Ok(())
}

/// Reads one environment document. A file whose *shape* is broken is an
/// `E_PARSE`; a file that declares a secret with a value is an `E_SCHEMA` and is
/// never coerced into something usable.
pub fn read_env_doc(workspace: &Path, name: &str) -> CoreResult<Option<EnvDoc>> {
    validate_env_name(name)?;
    let path = env_path(workspace, name);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| CoreError::io(path.display(), e))?;
    let parsed: RawEnvDoc =
        toml::from_str(&raw).map_err(|e| CoreError::parse(path.display(), e))?;
    Ok(Some(parsed.validate(&path.display().to_string())?))
}

pub fn list_env_docs(workspace: &Path) -> CoreResult<EnvDocList> {
    let dir = environments_dir(workspace);
    let mut list = EnvDocList {
        items: Vec::new(),
        skipped: Vec::new(),
    };
    if !dir.exists() {
        return Ok(list);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if crate::scan::is_local_values_file(&path) {
            continue;
        }
        let context = path.display().to_string();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|raw| toml::from_str::<RawEnvDoc>(&raw).map_err(|e| e.to_string()));
        match parsed {
            Ok(doc) => list.items.push(doc.validate(&context)?),
            Err(e) => list.skipped.push(format!("{context}: {e}")),
        }
    }
    list.items.sort_by(|a, b| a.name.cmp(&b.name));
    list.skipped.sort();
    Ok(list)
}

pub fn list_environments(workspace: &Path) -> CoreResult<EnvironmentList> {
    let docs = list_env_docs(workspace)?;
    Ok(EnvironmentList {
        items: docs.items.iter().map(EnvDoc::shared_view).collect(),
        skipped: docs.skipped,
    })
}

pub fn list_environment_views(
    workspace: &Path,
    secrets: &dyn SecretStore,
) -> CoreResult<EnvironmentViewList> {
    let docs = list_env_docs(workspace)?;
    let mut items = Vec::with_capacity(docs.items.len());
    for doc in &docs.items {
        items.push(EnvironmentView::build(doc, secrets)?);
    }
    Ok(EnvironmentViewList {
        items,
        skipped: docs.skipped,
    })
}

pub fn save_env_doc(workspace: &Path, doc: &EnvDoc) -> CoreResult<PathBuf> {
    crate::remote::ensure_writable(workspace)?;
    validate_env_name(&doc.name)?;
    for key in doc.vars.keys() {
        validate_var_name(key)?;
    }
    let dir = environments_dir(workspace);
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(dir.display(), e))?;
    let path = env_path(workspace, &doc.name);
    let raw = toml::to_string_pretty(doc).map_err(|e| CoreError::Parse(e.to_string()))?;
    atomic_write(&path, &raw)?;
    Ok(path)
}

/// Saves an environment, **routing every value by its declaration**: a shared
/// variable's value goes into the committed file, and anything not shared is
/// diverted to this machine's own store instead. The caller hands over a flat
/// name→value map and cannot get the split wrong.
///
/// Every declaration the file already had is kept — a caller that only knows
/// about values cannot erase them. An empty value for a non-shared variable
/// clears it, which is what emptying the field in an editor means.
pub fn save_environment(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &Environment,
) -> CoreResult<PathBuf> {
    validate_env_name(&env.name)?;
    let mut doc = EnvDoc::new(&env.name);
    let unshared: BTreeMap<String, VarDef> = read_env_doc(workspace, &env.name)?
        .map(|previous| {
            previous
                .vars
                .into_iter()
                .filter(|(_, def)| !def.is_shared())
                .collect()
        })
        .unwrap_or_default();

    for (key, value) in &env.vars {
        validate_var_name(key)?;
        match unshared.get(key) {
            Some(def) => {
                if value.is_empty() {
                    secrets.delete(&env.name, key)?;
                } else {
                    secrets.set(&env.name, key, value)?;
                    if def.is_secret() {
                        crate::redact::register(&env.name, key, value);
                    }
                }
            }
            None => {
                doc.vars.insert(key.clone(), VarDef::shared(value.clone()));
            }
        }
    }
    doc.vars.extend(unshared);
    save_env_doc(workspace, &doc)
}

/// Declares `key` secret — masked, redacted, and stored only on this machine —
/// and writes its value there. A shared variable of the same name is promoted:
/// its value leaves the committed file.
pub fn set_secret(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &str,
    key: &str,
    value: &str,
) -> CoreResult<PathBuf> {
    set_unshared(workspace, secrets, env, key, value, true)
}

/// Declares `key` local to this machine without marking it confidential — a
/// colleague's `devUrl`, which must not be shared and is pointless to mask.
pub fn set_local(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &str,
    key: &str,
    value: &str,
) -> CoreResult<PathBuf> {
    set_unshared(workspace, secrets, env, key, value, false)
}

fn set_unshared(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &str,
    key: &str,
    value: &str,
    secret: bool,
) -> CoreResult<PathBuf> {
    validate_env_name(env)?;
    validate_var_name(key)?;
    if value.is_empty() {
        return Err(CoreError::Secret(format!(
            "{env}.{key} was given an empty value; clear it instead of storing nothing"
        )));
    }
    let mut doc = read_env_doc(workspace, env)?.unwrap_or_else(|| EnvDoc::new(env));
    let hosts = doc
        .secret_hosts(key)
        .map(<[String]>::to_vec)
        .unwrap_or_default();
    secrets.set(env, key, value)?;
    if secret {
        crate::redact::register(env, key, value);
    }
    let def = if secret {
        VarDef::Secret { hosts }
    } else {
        VarDef::Local { hosts }
    };
    doc.vars.insert(key.to_string(), def);
    save_env_doc(workspace, &doc)
}

/// Removes the value from this machine. The declaration stays, so the workspace
/// still describes which variables the requests need.
pub fn clear_secret(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &str,
    key: &str,
) -> CoreResult<()> {
    validate_env_name(env)?;
    validate_var_name(key)?;
    let doc = read_env_doc(workspace, env)?
        .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {env}")))?;
    match doc.vars.get(key) {
        Some(def) if !def.is_shared() => secrets.delete(env, key),
        _ => Err(CoreError::NotFound(format!(
            "{env}.{key} holds no value on this machine; it is shared, so its value is in the environment file"
        ))),
    }
}

/// Drops a variable from the environment file, and its value from this machine
/// when it was a secret.
pub fn delete_var(
    workspace: &Path,
    secrets: &dyn SecretWriter,
    env: &str,
    key: &str,
) -> CoreResult<PathBuf> {
    validate_env_name(env)?;
    validate_var_name(key)?;
    let mut doc = read_env_doc(workspace, env)?
        .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {env}")))?;
    match doc.vars.remove(key) {
        Some(VarDef::Local { .. }) | Some(VarDef::Secret { .. }) => secrets.delete(env, key)?,
        Some(VarDef::Shared { .. }) => {}
        None => {
            return Err(CoreError::NotFound(format!(
                "unknown variable: {env}.{key}"
            )))
        }
    }
    save_env_doc(workspace, &doc)
}

/// Binds a variable to a host, so its value can never be sent anywhere else.
/// Only a variable whose value stays on this machine has anything left to
/// protect, so only those can be bound.
pub fn bind_secret_host(
    workspace: &Path,
    env: &str,
    key: &str,
    host: &str,
) -> CoreResult<Vec<String>> {
    validate_env_name(env)?;
    validate_var_name(key)?;
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(CoreError::InvalidName("host must not be empty".to_string()));
    }
    let mut doc = read_env_doc(workspace, env)?
        .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {env}")))?;
    let hosts = match doc.vars.get_mut(key).and_then(VarDef::hosts_mut) {
        Some(hosts) => hosts,
        None => {
            return Err(CoreError::NotFound(format!(
                "{env}.{key} is shared, so there is nothing to bind to a host"
            )))
        }
    };
    if !hosts.contains(&host) {
        hosts.push(host);
        hosts.sort();
    }
    let bound = hosts.clone();
    save_env_doc(workspace, &doc)?;
    Ok(bound)
}

/// Which of `env`'s non-shared variables have a value on this machine. A
/// workspace that arrived from a colleague answers `false` for all of them,
/// which is what the app shows instead of failing cryptically at send time.
pub fn secret_status(
    workspace: &Path,
    secrets: &dyn SecretStore,
    env: &str,
) -> CoreResult<BTreeMap<String, bool>> {
    let doc = read_env_doc(workspace, env)?
        .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {env}")))?;
    let mut status = BTreeMap::new();
    for (key, def) in &doc.vars {
        if !def.is_shared() {
            status.insert(key.clone(), secrets.source(env, key)?.is_some());
        }
    }
    Ok(status)
}

/// Non-shared variables of `env` with no value anywhere on this machine.
pub fn missing_values(
    workspace: &Path,
    secrets: &dyn SecretStore,
    env: &str,
) -> CoreResult<Vec<String>> {
    Ok(secret_status(workspace, secrets, env)?
        .into_iter()
        .filter(|(_, held)| !held)
        .map(|(key, _)| key)
        .collect())
}

pub fn delete_environment(workspace: &Path, name: &str) -> CoreResult<()> {
    crate::remote::ensure_writable(workspace)?;
    validate_env_name(name)?;
    let path = env_path(workspace, name);
    std::fs::remove_file(&path).map_err(|e| CoreError::io(path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::MemorySecrets;

    fn env(name: &str, pairs: &[(&str, &str)]) -> Environment {
        Environment {
            name: name.to_string(),
            vars: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn empty_workspace_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let list = list_environments(dir.path()).unwrap();
        assert!(list.items.is_empty());
        assert!(list.skipped.is_empty());
    }

    #[test]
    fn save_then_list_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let staging = env(
            "staging",
            &[("base", "https://staging.x.dev"), ("token", "t1")],
        );
        let prod = env("prod", &[("base", "https://x.dev")]);
        save_environment(dir.path(), &MemorySecrets::new(), &staging).unwrap();
        save_environment(dir.path(), &MemorySecrets::new(), &prod).unwrap();
        assert_eq!(
            list_environments(dir.path()).unwrap().items,
            vec![prod, staging]
        );
    }

    #[test]
    fn save_writes_readable_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("staging", &[("base", "b")]),
        )
        .unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            raw,
            "schema_version = 1\nname = \"staging\"\n\n[vars.base]\nvalue = \"b\"\n"
        );
    }

    fn read_raw(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join("environments").join(format!("{name}.toml"))).unwrap()
    }

    fn write_raw(dir: &Path, name: &str, raw: &str) {
        std::fs::create_dir_all(dir.join("environments")).unwrap();
        std::fs::write(dir.join("environments").join(format!("{name}.toml")), raw).unwrap();
    }

    #[test]
    fn a_secret_declaration_can_never_serialize_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = EnvDoc::new("prod");
        doc.vars
            .insert("base".to_string(), VarDef::shared("https://api.acme.com"));
        doc.vars.insert(
            "access_token".to_string(),
            VarDef::secret(&["api.acme.com"]),
        );
        save_env_doc(dir.path(), &doc).unwrap();
        let raw = read_raw(dir.path(), "prod");
        assert_eq!(
            raw,
            "schema_version = 1\nname = \"prod\"\n\n[vars.access_token]\nsecret = true\nhosts = [\"api.acme.com\"]\n\n[vars.base]\nvalue = \"https://api.acme.com\"\n"
        );
        let secret_table = raw.split("[vars.access_token]").nth(1).unwrap();
        let secret_table = secret_table.split("[vars.").next().unwrap();
        assert!(!secret_table.contains("value"), "{raw}");
        assert_eq!(read_env_doc(dir.path(), "prod").unwrap(), Some(doc));
    }

    #[test]
    fn a_secret_with_a_value_on_disk_is_a_hard_schema_error() {
        let dir = tempfile::tempdir().unwrap();
        write_raw(
            dir.path(),
            "prod",
            "name = \"prod\"\n\n[vars.token]\nsecret = true\nvalue = \"leaked-in-git\"\n",
        );
        let err = read_env_doc(dir.path(), "prod").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(err.to_string().contains("secret = true"), "{err}");
        assert!(!err.to_string().contains("leaked-in-git"), "{err}");
        assert_eq!(
            list_env_docs(dir.path()).unwrap_err().code(),
            "E_SCHEMA",
            "a listing must not quietly skip the dangerous file"
        );
    }

    #[test]
    fn a_var_that_declares_none_of_the_three_things_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        write_raw(
            dir.path(),
            "prod",
            "name = \"prod\"\n\n[vars.token]\nhosts = [\"api.acme.com\"]\n",
        );
        let err = read_env_doc(dir.path(), "prod").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");

        write_raw(
            dir.path(),
            "other",
            "name = \"other\"\n\n[vars.token]\nvalue = \"v\"\nhosts = [\"api.acme.com\"]\n",
        );
        let err = read_env_doc(dir.path(), "other").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(
            err.to_string().contains("binds hosts but is shared"),
            "{err}"
        );
    }

    #[test]
    fn the_legacy_flat_format_migrates_in_memory_and_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        write_raw(
            dir.path(),
            "prod",
            "name = \"prod\"\n[vars]\nbase = \"https://x.dev\"\nregion = \"eu\"\n",
        );
        let doc = read_env_doc(dir.path(), "prod").unwrap().unwrap();
        assert_eq!(doc.vars["base"], VarDef::shared("https://x.dev"));
        assert_eq!(doc.vars["region"], VarDef::shared("eu"));
        assert_eq!(
            list_environments(dir.path()).unwrap().items,
            vec![env("prod", &[("base", "https://x.dev"), ("region", "eu")])]
        );

        save_environment(dir.path(), &MemorySecrets::new(), &doc.shared_view()).unwrap();
        assert!(read_raw(dir.path(), "prod").contains("[vars.base]"));
        assert_eq!(read_env_doc(dir.path(), "prod").unwrap().unwrap(), doc);
    }

    #[test]
    fn saving_diverts_an_unshared_value_instead_of_writing_it_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t").unwrap();

        save_environment(
            dir.path(),
            &store,
            &env(
                "prod",
                &[("base", "https://x.dev"), ("token", "typed-here")],
            ),
        )
        .unwrap();
        assert!(!read_raw(dir.path(), "prod").contains("typed-here"));
        assert!(read_raw(dir.path(), "prod").contains("secret = true"));
        assert_eq!(
            store.get("prod", "token").unwrap(),
            Some("typed-here".to_string())
        );
    }

    #[test]
    fn saving_keeps_the_unshared_declarations_it_does_not_know_about() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t").unwrap();
        bind_secret_host(dir.path(), "prod", "token", "API.acme.com").unwrap();

        save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("prod", &[("base", "https://x.dev")]),
        )
        .unwrap();
        let doc = read_env_doc(dir.path(), "prod").unwrap().unwrap();
        assert_eq!(
            doc.vars["token"],
            VarDef::Secret {
                hosts: vec!["api.acme.com".to_string()]
            }
        );
        assert_eq!(doc.vars["base"], VarDef::shared("https://x.dev"));
    }

    #[test]
    fn set_secret_promotes_a_shared_variable_and_takes_its_value_out_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("prod", &[("token", "was-committed")]),
        )
        .unwrap();
        assert!(read_raw(dir.path(), "prod").contains("was-committed"));

        set_secret(dir.path(), &store, "prod", "token", "now-on-this-machine").unwrap();
        let raw = read_raw(dir.path(), "prod");
        assert!(!raw.contains("was-committed"), "{raw}");
        assert!(!raw.contains("now-on-this-machine"), "{raw}");
        assert!(raw.contains("secret = true"), "{raw}");
        assert_eq!(
            store.get("prod", "token").unwrap(),
            Some("now-on-this-machine".to_string())
        );
    }

    #[test]
    fn secret_status_reports_only_what_this_machine_holds() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t-token").unwrap();
        let mut doc = read_env_doc(dir.path(), "prod").unwrap().unwrap();
        doc.vars
            .insert("other".to_string(), VarDef::Secret { hosts: Vec::new() });
        doc.vars.insert("base".to_string(), VarDef::shared("b"));
        save_env_doc(dir.path(), &doc).unwrap();

        let status = secret_status(dir.path(), &store, "prod").unwrap();
        assert_eq!(
            status,
            BTreeMap::from([("token".to_string(), true), ("other".to_string(), false)])
        );

        clear_secret(dir.path(), &store, "prod", "token").unwrap();
        assert!(!secret_status(dir.path(), &store, "prod").unwrap()["token"]);
        assert!(
            read_env_doc(dir.path(), "prod").unwrap().unwrap().vars["token"].is_secret(),
            "clearing a value must keep the declaration"
        );
    }

    #[test]
    fn set_secret_refuses_an_empty_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        assert_eq!(
            set_secret(dir.path(), &store, "prod", "token", "")
                .unwrap_err()
                .code(),
            "E_SECRET"
        );
    }

    #[test]
    fn deleting_a_variable_removes_the_declaration_and_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t-token").unwrap();
        delete_var(dir.path(), &store, "prod", "token").unwrap();
        assert!(read_env_doc(dir.path(), "prod")
            .unwrap()
            .unwrap()
            .vars
            .is_empty());
        assert_eq!(store.get("prod", "token").unwrap(), None);
        assert_eq!(
            delete_var(dir.path(), &store, "prod", "token")
                .unwrap_err()
                .code(),
            "E_NOT_FOUND"
        );
    }

    #[test]
    fn binding_a_host_is_idempotent_and_lowercased() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t-token").unwrap();
        assert_eq!(
            bind_secret_host(dir.path(), "prod", "token", "API.Acme.com").unwrap(),
            vec!["api.acme.com".to_string()]
        );
        assert_eq!(
            bind_secret_host(dir.path(), "prod", "token", "api.acme.com").unwrap(),
            vec!["api.acme.com".to_string()]
        );
        assert_eq!(
            bind_secret_host(dir.path(), "prod", "base", "x.dev")
                .unwrap_err()
                .code(),
            "E_NOT_FOUND"
        );
    }

    #[test]
    fn a_view_never_carries_a_secret_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("prod", &[("base", "https://x.dev")]),
        )
        .unwrap();
        set_secret(dir.path(), &store, "prod", "token", "s3cr3t-view-value").unwrap();
        bind_secret_host(dir.path(), "prod", "token", "x.dev").unwrap();

        let list = list_environment_views(dir.path(), &store).unwrap();
        let vars = &list.items[0].vars;
        assert_eq!(
            vars["base"],
            VarInfo {
                shared: true,
                secret: false,
                value: Some("https://x.dev".to_string()),
                hosts: None,
                set: true,
                source: Some(VarSource::File),
            }
        );
        assert_eq!(
            vars["token"],
            VarInfo {
                shared: false,
                secret: true,
                value: None,
                hosts: Some(vec!["x.dev".to_string()]),
                set: true,
                source: Some(VarSource::Local),
            }
        );
        let json = serde_json::to_string(&list.items[0]).unwrap();
        assert!(!json.contains("s3cr3t-view-value"), "{json}");
    }

    #[test]
    fn a_future_environment_schema_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        write_raw(dir.path(), "prod", "schema_version = 99\nname = \"prod\"\n");
        let err = read_env_doc(dir.path(), "prod").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(err.to_string().contains("schema_version 99"));
    }

    #[test]
    fn variable_names_are_validated_on_the_way_in() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemorySecrets::new();
        assert!(save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("prod", &[("../evil", "x")])
        )
        .is_err());
        assert!(set_secret(dir.path(), &store, "prod", "../evil", "s3cr3t-value").is_err());
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &MemorySecrets::new(), &env("staging", &[])).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path().join("environments"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["staging.toml"]);
    }

    #[test]
    fn rejects_path_traversal_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_environment(dir.path(), &MemorySecrets::new(), &env("../evil", &[])).is_err());
        assert!(save_environment(dir.path(), &MemorySecrets::new(), &env("", &[])).is_err());
    }

    #[test]
    fn rejects_non_ascii_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            save_environment(dir.path(), &MemorySecrets::new(), &env("prodücción", &[])).is_err()
        );
    }

    #[test]
    fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &MemorySecrets::new(), &env("tmp", &[])).unwrap();
        delete_environment(dir.path(), "tmp").unwrap();
        assert!(list_environments(dir.path()).unwrap().items.is_empty());
    }

    #[test]
    fn delete_rejects_traversal_and_absolute_names() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.toml");
        std::fs::write(&outside, "name = \"outside\"").unwrap();
        let err = delete_environment(dir.path(), "../outside")
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid environment name"));
        assert!(outside.exists());

        let abs = dir.path().join("abs.toml");
        std::fs::write(&abs, "name = \"abs\"").unwrap();
        let abs_name = abs.with_extension("");
        let err = delete_environment(dir.path(), abs_name.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid environment name"));
        assert!(abs.exists());
    }

    #[test]
    fn corrupt_env_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &MemorySecrets::new(), &env("good", &[])).unwrap();
        let bad = dir.path().join("environments").join("bad.toml");
        std::fs::write(&bad, "not [ valid toml").unwrap();
        let list = list_environments(dir.path()).unwrap();
        assert_eq!(list.items, vec![env("good", &[])]);
        assert_eq!(list.skipped.len(), 1);
        assert!(list.skipped[0].contains("bad.toml"));
    }

    fn registry_file(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("config").join("workspaces.toml")
    }

    #[test]
    fn first_run_creates_and_activates_the_default_workspace() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let default = home.path().join("Mandalo");
        let list = list_workspaces(&registry, &default).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].name, "Personal");
        assert_eq!(list.items[0].path, default.to_str().unwrap());
        assert_eq!(list.active, list.items[0].id);
        assert!(default.join("mandalo.toml").exists());
        assert!(default.join("environments").is_dir());
        assert!(default.join("collections").is_dir());
        assert_eq!(list_workspaces(&registry, &default).unwrap(), list);
    }

    #[test]
    fn registry_is_written_in_the_documented_shape() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        create_workspace(&registry, &home.path().join("Work"), "Work").unwrap();
        let raw = std::fs::read_to_string(&registry).unwrap();
        assert!(raw.contains("schema_version = 1"));
        assert!(raw.contains("active = "));
        assert!(raw.contains("[[workspace]]"));
        assert!(raw.contains("name = \"Work\""));
    }

    #[test]
    fn create_workspace_scaffolds_and_activates() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("Personal");
        let first = create_workspace(&registry, &path, "Personal").unwrap();
        let second = create_workspace(&registry, &home.path().join("Work"), "Work").unwrap();
        assert!(uuid::Uuid::parse_str(&first.id).is_ok());
        assert_ne!(first.id, second.id);
        let list = list_workspaces(&registry, &path).unwrap();
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.active, second.id);
        let manifest = read_manifest(&path).unwrap().unwrap();
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
        assert_eq!(manifest.id, first.id);
    }

    #[test]
    fn create_workspace_rejects_relative_paths_and_empty_names() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        assert!(create_workspace(&registry, Path::new("relative"), "X")
            .unwrap_err()
            .to_string()
            .contains("must be absolute"));
        assert!(create_workspace(&registry, &home.path().join("A"), " ")
            .unwrap_err()
            .to_string()
            .contains("name must not be empty"));
    }

    #[test]
    fn create_workspace_refuses_a_non_empty_foreign_directory() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("Stuff");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("notes.txt"), "hi").unwrap();
        let err = create_workspace(&registry, &path, "Stuff")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not empty and is not a Mándalo workspace")
                || err.contains("not a Mándalo workspace"),
            "{err}"
        );
        assert!(!path.join("mandalo.toml").exists());
    }

    #[test]
    fn create_workspace_adopts_an_existing_workspace_id() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("Personal");
        let first = create_workspace(&registry, &path, "Personal").unwrap();
        let again = create_workspace(&registry, &path, "Renamed").unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.name, "Renamed");
        assert_eq!(list_workspaces(&registry, &path).unwrap().items.len(), 1);
    }

    #[test]
    fn open_workspace_refuses_foreign_non_empty_directories() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("repo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("README.md"), "hi").unwrap();
        let err = open_workspace(&registry, &path).unwrap_err().to_string();
        assert!(err.contains("not empty"), "{err}");
        assert!(!path.join("mandalo.toml").exists());
    }

    #[test]
    fn open_workspace_hints_child_workspaces() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let parent = home.path().join("examples");
        let child = parent.join("mock-workspace");
        std::fs::create_dir_all(&child).unwrap();
        create_workspace(&registry, &child, "Mock").unwrap();
        let err = open_workspace(&registry, &parent).unwrap_err().to_string();
        assert!(err.contains("mock-workspace"), "{err}");
        assert!(!parent.join("mandalo.toml").exists());
    }

    #[test]
    fn open_workspace_seeds_a_root_openapi_document() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("petstore");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("README.md"), "demo").unwrap();
        std::fs::write(
            path.join("openapi.json"),
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
        let opened = open_workspace(&registry, &path).unwrap();
        assert!(path.join("mandalo.toml").exists());
        let tree = crate::collection::list_tree(&path).unwrap();
        assert_eq!(tree.collections.len(), 1);
        assert!(
            tree.collections[0]
                .folders
                .iter()
                .any(|f| !f.requests.is_empty())
                || !tree.collections[0].requests.is_empty()
        );
        assert_eq!(opened.workspace.name, "petstore");
    }

    #[test]
    fn list_workspaces_repairs_stale_openapi_roots() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("petstore");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("openapi.json"),
            r#"{
              "openapi": "3.0.3",
              "info": {"title": "Petstore", "version": "1"},
              "servers": [{"url": "https://petstore.example"}],
              "paths": {
                "/ping": {"get": {"operationId": "ping"}}
              }
            }"#,
        )
        .unwrap();
        let id = "11111111-1111-1111-1111-111111111111";
        write_registry(
            &registry,
            &Registry {
                schema_version: SCHEMA_VERSION,
                active: id.to_string(),
                workspaces: vec![WorkspaceInfo {
                    id: id.to_string(),
                    path: path_string(&path).unwrap(),
                    name: "petstore".to_string(),
                }],
            },
        )
        .unwrap();
        let list = list_workspaces(&registry, &home.path().join("Personal")).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].id, id);
        assert!(path.join("mandalo.toml").exists());
        let manifest = read_manifest(&path).unwrap().unwrap();
        assert_eq!(manifest.id, id);
        ensure_ready(&registry, &path).unwrap();
        let tree = crate::collection::list_tree(&path).unwrap();
        assert!(!tree.collections.is_empty());
    }

    #[test]
    fn list_workspaces_drops_unrepairable_stale_entries() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("random");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("code.rs"), "fn main() {}").unwrap();
        let id = "22222222-2222-2222-2222-222222222222";
        write_registry(
            &registry,
            &Registry {
                schema_version: SCHEMA_VERSION,
                active: id.to_string(),
                workspaces: vec![WorkspaceInfo {
                    id: id.to_string(),
                    path: path_string(&path).unwrap(),
                    name: "random".to_string(),
                }],
            },
        )
        .unwrap();
        let list = list_workspaces(&registry, &home.path().join("Personal")).unwrap();
        assert!(!list.items.iter().any(|w| w.id == id));
        assert!(!path.join("mandalo.toml").exists());
        assert!(!list.items.is_empty());
    }

    #[test]
    fn open_workspace_registers_and_names_after_the_directory() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let path = home.path().join("Team API");
        std::fs::create_dir_all(&path).unwrap();
        let opened = open_workspace(&registry, &path).unwrap();
        assert_eq!(opened.workspace.name, "Team API");
        assert!(path.join("mandalo.toml").exists());
        let reopened = open_workspace(&registry, &path).unwrap();
        assert_eq!(reopened.workspace.id, opened.workspace.id);
        assert_eq!(list_workspaces(&registry, &path).unwrap().items.len(), 1);
    }

    #[test]
    fn open_workspace_requires_an_existing_directory() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        assert!(open_workspace(&registry, &home.path().join("ghost"))
            .unwrap_err()
            .to_string()
            .contains("does not exist"));
    }

    #[test]
    fn open_workspace_redirects_collections_subdir_to_parent() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let root = home.path().join("Team");
        let created = create_workspace(&registry, &root, "Team").unwrap();
        let nested = root.join("collections");
        let opened = open_workspace(&registry, &nested).unwrap();
        assert_eq!(opened.workspace.id, created.id);
        assert_eq!(opened.workspace.path, path_string(&root).unwrap());
        assert!(!nested.join("mandalo.toml").exists());
        assert!(!nested.join("collections").exists());
        assert!(!nested.join("environments").exists());
    }

    #[test]
    fn set_active_workspace_switches_and_rejects_unknown_ids() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let first = create_workspace(&registry, &home.path().join("A"), "A").unwrap();
        create_workspace(&registry, &home.path().join("B"), "B").unwrap();
        let active = set_active_workspace(&registry, &first.id).unwrap();
        assert_eq!(active, first);
        assert_eq!(
            list_workspaces(&registry, &home.path().join("A"))
                .unwrap()
                .active,
            first.id
        );
        assert!(set_active_workspace(&registry, "nope")
            .unwrap_err()
            .to_string()
            .contains("unknown workspace id"));
    }

    #[test]
    fn remove_workspace_unregisters_without_deleting_files() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let a = create_workspace(&registry, &home.path().join("A"), "A").unwrap();
        let b = create_workspace(&registry, &home.path().join("B"), "B").unwrap();
        remove_workspace(&registry, &b.id).unwrap();
        let list = list_workspaces(&registry, &home.path().join("A")).unwrap();
        assert_eq!(list.items, vec![a.clone()]);
        assert_eq!(list.active, a.id);
        assert!(home.path().join("B").join("mandalo.toml").exists());
        assert!(remove_workspace(&registry, &b.id)
            .unwrap_err()
            .to_string()
            .contains("unknown workspace id"));
    }

    #[test]
    fn removing_the_last_workspace_recreates_the_default_on_next_list() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let a = create_workspace(&registry, &home.path().join("A"), "A").unwrap();
        remove_workspace(&registry, &a.id).unwrap();
        let default = home.path().join("Mandalo");
        let list = list_workspaces(&registry, &default).unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].name, "Personal");
    }

    #[test]
    fn resolve_workspace_accepts_ids_and_absolute_paths() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        let a = create_workspace(&registry, &home.path().join("A"), "A").unwrap();
        assert_eq!(
            resolve_workspace(&registry, &a.id).unwrap(),
            home.path().join("A")
        );
        assert_eq!(
            resolve_workspace(&registry, home.path().join("A").to_str().unwrap()).unwrap(),
            home.path().join("A")
        );
        assert!(resolve_workspace(&registry, "nope")
            .unwrap_err()
            .to_string()
            .contains("unknown workspace"));
    }

    #[test]
    fn future_registry_schema_fails_loud() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        std::fs::create_dir_all(registry.parent().unwrap()).unwrap();
        std::fs::write(&registry, "schema_version = 99\nactive = \"\"\n").unwrap();
        let err = list_workspaces(&registry, &home.path().join("A"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported registry schema_version 99"));
    }

    #[test]
    fn future_workspace_schema_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mandalo.toml"),
            "schema_version = 99\nid = \"x\"\nname = \"X\"\n",
        )
        .unwrap();
        let err = read_manifest(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsupported workspace schema_version 99"));
    }

    #[test]
    fn registry_writes_leave_no_temp_files() {
        let home = tempfile::tempdir().unwrap();
        let registry = registry_file(&home);
        create_workspace(&registry, &home.path().join("A"), "A").unwrap();
        let names: Vec<_> = std::fs::read_dir(registry.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["workspaces.toml"]);
    }

    /// `git init` then `mandalo init` is how a git-native workspace starts, so a
    /// repository with no work in it yet has to scaffold like any empty folder.
    #[test]
    fn a_freshly_initialised_repository_still_counts_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"junk").unwrap();

        let registry = dir.path().join("registry.toml");
        let created = create_workspace(&registry, dir.path(), "Repo").unwrap();

        assert_eq!(created.name, "Repo");
        assert!(dir.path().join("mandalo.toml").is_file());
    }

    #[test]
    fn a_directory_with_real_work_in_it_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("README.md"), b"# api").unwrap();

        let registry = dir.path().join("registry.toml");
        let error = create_workspace(&registry, dir.path(), "Repo").unwrap_err();

        assert_eq!(error.code(), "E_CONFLICT");
    }

    #[test]
    fn sanitize_maps_invalid_chars_and_dedupes_collisions() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging (EU)"),
            "Staging--EU-"
        );
        save_environment(dir.path(), &MemorySecrets::new(), &env("Staging--EU-", &[])).unwrap();
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging (EU)"),
            "Staging--EU--2"
        );
        save_environment(
            dir.path(),
            &MemorySecrets::new(),
            &env("Staging--EU--2", &[]),
        )
        .unwrap();
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging (EU)"),
            "Staging--EU--3"
        );
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging--EU-"),
            "Staging--EU-"
        );
        assert_eq!(sanitize_env_name(dir.path(), "···"), "---");
        assert_eq!(sanitize_env_name(dir.path(), ""), "imported");
    }
}
