use crate::collection::{self, CollectionNode, FolderNode, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::postman::{self, ExportWarnings, ImportReport};
use crate::review;
use crate::scan::{self, Finding};
use crate::workspace::{self, EnvDoc, RawEnvDoc, ShareFormat, VarDef};
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

/// Sorted by file, then by the index inside it as a number — `#10` follows `#9`,
/// which sorting the whole string would not do.
fn paths(node: &CollectionNode) -> Vec<String> {
    let mut out: Vec<String> = node.requests.iter().map(|r| r.path.clone()).collect();
    flatten(&node.folders, &mut out);
    out.sort_by_key(|path| {
        let (file, fragment) = collection::split_identity(path);
        (
            file.to_string(),
            fragment.and_then(|f| f.parse::<usize>().ok()).unwrap_or(0),
        )
    });
    out
}

/// What of the workspace an export carries. `None` means "everything of this
/// kind", so the default value is the whole workspace and narrowing is opt-in.
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportSelection {
    pub collections: Option<Vec<CollectionSelection>>,
    pub environments: Option<Vec<String>>,
}

/// A collection, and optionally only part of it. Both lists empty means the
/// whole collection; otherwise a request is in when a folder covers it or it is
/// named outright.
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CollectionSelection {
    pub slug: String,
    pub folders: Vec<String>,
    pub requests: Vec<String>,
}

impl CollectionSelection {
    pub fn whole(slug: impl Into<String>) -> Self {
        CollectionSelection {
            slug: slug.into(),
            folders: Vec::new(),
            requests: Vec::new(),
        }
    }

    fn is_whole(&self) -> bool {
        self.folders.is_empty() && self.requests.is_empty()
    }

    fn covers(&self, path: &str) -> bool {
        if self.is_whole() {
            return true;
        }
        let (file, _) = collection::split_identity(path);
        self.folders
            .iter()
            .any(|folder| under(path, folder.trim_matches('/')))
            || self.requests.iter().any(|r| r == path || r == file)
    }
}

fn under(path: &str, folder: &str) -> bool {
    !folder.is_empty() && path.starts_with(folder) && path[folder.len()..].starts_with('/')
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncludedRequest {
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncludedCollection {
    pub slug: String,
    pub name: String,
    pub requests: Vec<IncludedRequest>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportIncluded {
    pub collections: Vec<IncludedCollection>,
    pub environments: Vec<String>,
    pub request_count: usize,
}

/// Said positively on purpose: a reader should see that the mechanism worked,
/// not have to deduce it from an absence. Secret and local values are both
/// missing for the same reason — they never lived in the workspace.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportExcluded {
    pub secret_values: usize,
    pub local_values: usize,
    pub withheld_names: Vec<String>,
    pub collections: Vec<String>,
    pub requests: usize,
    pub environments: Vec<String>,
}

/// What an export would write, before anything is written. `run_export` takes
/// the `token` back and refuses to write when the workspace has moved on.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    pub included: ExportIncluded,
    pub excluded: ExportExcluded,
    pub findings: Vec<Finding>,
    pub bytes: usize,
    pub blocked: bool,
    pub token: String,
    /// `bundle` or `postman` — what `json` actually holds.
    pub format: String,
    pub warnings: Vec<String>,
    #[serde(skip)]
    json: String,
}

impl ExportPlan {
    pub fn json(&self) -> &str {
        &self.json
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
    pub path: String,
    pub bytes: usize,
    pub requests: usize,
    pub collections: usize,
    pub environments: usize,
    pub forced: bool,
}

fn unknown(kind: &str, names: &[String]) -> CoreError {
    CoreError::NotFound(format!(
        "cannot export: no such {kind} in this workspace: {}",
        names.join(", ")
    ))
}

/// Resolve export format: explicit override, else `[share]`, else Mandalo bundle.
pub fn resolve_format(
    workspace: &Path,
    override_format: Option<ShareFormat>,
) -> CoreResult<ShareFormat> {
    if let Some(format) = override_format {
        return Ok(format);
    }
    Ok(workspace::share_config(workspace)?
        .map(|c| c.format)
        .unwrap_or(ShareFormat::Native))
}

pub fn plan_export(workspace: &Path, selection: &ExportSelection) -> CoreResult<ExportPlan> {
    plan_export_as(workspace, selection, None)
}

pub fn plan_export_as(
    workspace: &Path,
    selection: &ExportSelection,
    format: Option<ShareFormat>,
) -> CoreResult<ExportPlan> {
    match resolve_format(workspace, format)? {
        ShareFormat::Native => plan_bundle(workspace, selection),
        ShareFormat::Postman => plan_postman(workspace, selection),
    }
}

fn plan_bundle(workspace: &Path, selection: &ExportSelection) -> CoreResult<ExportPlan> {
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

    if let Some(wanted) = &selection.collections {
        let missing: Vec<String> = wanted
            .iter()
            .filter(|w| !tree.collections.iter().any(|n| n.slug == w.slug))
            .map(|w| w.slug.clone())
            .collect();
        if !missing.is_empty() {
            return Err(unknown("collection", &missing));
        }
    }

    let mut included = ExportIncluded::default();
    let mut excluded = ExportExcluded::default();
    let mut collections = Vec::new();
    let mut unmatched = Vec::new();

    for node in &tree.collections {
        let all = paths(node);
        let want = match &selection.collections {
            None => Some(CollectionSelection::whole(&node.slug)),
            Some(list) => list.iter().find(|c| c.slug == node.slug).cloned(),
        };
        let Some(want) = want else {
            excluded.collections.push(node.slug.clone());
            excluded.requests += all.len();
            continue;
        };
        for folder in &want.folders {
            if !all.iter().any(|p| under(p, folder.trim_matches('/'))) {
                unmatched.push(format!("{}:{folder}", node.slug));
            }
        }
        for request in &want.requests {
            if !all
                .iter()
                .any(|p| p == request || collection::split_identity(p).0 == request)
            {
                unmatched.push(format!("{}:{request}", node.slug));
            }
        }
        let mut requests = Vec::new();
        let mut listed = Vec::new();
        for path in all {
            if !want.covers(&path) {
                excluded.requests += 1;
                continue;
            }
            // The source view, not the resolved one: a bundle travels to another
            // machine, where a baked-in absolute proto path resolves to nothing.
            let request = collection::load_request_source(workspace, &node.slug, &path)?;
            listed.push(IncludedRequest {
                path: path.clone(),
                name: request.name.clone(),
            });
            requests.push(BundleRequest { path, request });
        }
        included.request_count += requests.len();
        included.collections.push(IncludedCollection {
            slug: node.slug.clone(),
            name: node.name.clone(),
            requests: listed,
        });
        collections.push(BundleCollection {
            id: node.id.clone(),
            slug: node.slug.clone(),
            name: node.name.clone(),
            requests,
        });
    }
    if !unmatched.is_empty() {
        return Err(unknown("folder or request", &unmatched));
    }

    if let Some(wanted) = &selection.environments {
        let missing: Vec<String> = wanted
            .iter()
            .filter(|name| !environments.items.iter().any(|e| &&e.name == name))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(unknown("environment", &missing));
        }
    }
    let mut envs = Vec::new();
    for doc in environments.items {
        let wanted = match &selection.environments {
            None => true,
            Some(list) => list.iter().any(|name| name == &doc.name),
        };
        if !wanted {
            excluded.environments.push(doc.name.clone());
            continue;
        }
        for (key, def) in &doc.vars {
            match def {
                VarDef::Shared { .. } => continue,
                VarDef::Secret { .. } => excluded.secret_values += 1,
                VarDef::Local { .. } => excluded.local_values += 1,
            }
            excluded.withheld_names.push(format!("{}.{key}", doc.name));
        }
        included.environments.push(doc.name.clone());
        envs.push(doc);
    }

    let bundle = Bundle {
        mandalo_bundle: BUNDLE_VERSION,
        collections,
        requests: Vec::new(),
        environments: envs,
    };
    let json =
        serde_json::to_string_pretty(&bundle).map_err(|e| CoreError::Parse(e.to_string()))?;
    let findings = scan::scan_text(Path::new("<bundle>"), &json);
    let token = review::token("export", std::slice::from_ref(&json))?;
    Ok(ExportPlan {
        bytes: json.len(),
        blocked: !findings.is_empty(),
        included,
        excluded,
        findings,
        token,
        format: "bundle".to_string(),
        warnings: Vec::new(),
        json,
    })
}

fn plan_postman(workspace: &Path, selection: &ExportSelection) -> CoreResult<ExportPlan> {
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

    if let Some(wanted) = &selection.collections {
        let missing: Vec<String> = wanted
            .iter()
            .filter(|w| !tree.collections.iter().any(|n| n.slug == w.slug))
            .map(|w| w.slug.clone())
            .collect();
        if !missing.is_empty() {
            return Err(unknown("collection", &missing));
        }
    }
    if let Some(wanted) = &selection.environments {
        let missing: Vec<String> = wanted
            .iter()
            .filter(|name| !environments.items.iter().any(|e| &&e.name == name))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(unknown("environment", &missing));
        }
    }

    let mut included = ExportIncluded::default();
    let mut excluded = ExportExcluded::default();
    let mut warnings = ExportWarnings::default();

    let chosen_collections: Vec<&CollectionNode> = tree
        .collections
        .iter()
        .filter(|node| match &selection.collections {
            None => true,
            Some(list) => list.iter().any(|c| c.slug == node.slug),
        })
        .collect();
    for node in &tree.collections {
        if chosen_collections.iter().any(|c| c.slug == node.slug) {
            continue;
        }
        excluded.collections.push(node.slug.clone());
        excluded.requests += paths(node).len();
    }

    let chosen_envs: Vec<&EnvDoc> = match &selection.environments {
        // Whole-workspace Postman export prefers a single collection file; envs
        // are separate exports. Only include envs when the caller asked for them
        // (and no collections), or when there are no collections at all.
        None if selection.collections.is_none() && chosen_collections.is_empty() => {
            environments.items.iter().collect()
        }
        None => Vec::new(),
        Some(list) => environments
            .items
            .iter()
            .filter(|doc| list.iter().any(|name| name == &doc.name))
            .collect(),
    };
    for doc in &environments.items {
        if chosen_envs.iter().any(|e| e.name == doc.name) {
            continue;
        }
        excluded.environments.push(doc.name.clone());
    }

    let only_collections = !chosen_collections.is_empty() && chosen_envs.is_empty();
    let only_envs = chosen_collections.is_empty() && !chosen_envs.is_empty();
    if !only_collections && !only_envs {
        return Err(CoreError::InvalidName(
            "Postman export writes one file: pick exactly one collection (no environments), or exactly one environment (no collections)"
                .to_string(),
        ));
    }
    if only_collections && chosen_collections.len() != 1 {
        return Err(CoreError::InvalidName(format!(
            "Postman export writes one collection per file — pick a single collection (got {})",
            chosen_collections
                .iter()
                .map(|c| c.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if only_envs && chosen_envs.len() != 1 {
        return Err(CoreError::InvalidName(format!(
            "Postman export writes one environment per file — pick a single environment (got {})",
            chosen_envs
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let json = if only_collections {
        let node = chosen_collections[0];
        let want = selection
            .collections
            .as_ref()
            .and_then(|list| list.iter().find(|c| c.slug == node.slug).cloned())
            .unwrap_or_else(|| CollectionSelection::whole(&node.slug));
        let mut unmatched = Vec::new();
        for folder in &want.folders {
            if !paths(node)
                .iter()
                .any(|p| under(p, folder.trim_matches('/')))
            {
                unmatched.push(format!("{}:{folder}", node.slug));
            }
        }
        for request in &want.requests {
            if !paths(node)
                .iter()
                .any(|p| p == request || collection::split_identity(p).0 == request)
            {
                unmatched.push(format!("{}:{request}", node.slug));
            }
        }
        if !unmatched.is_empty() {
            return Err(unknown("folder or request", &unmatched));
        }
        // Folder/request narrowing: build a filtered tree view by only rendering covered paths.
        // Cheapest path that still honours selection: render full collection when whole;
        // otherwise refuse folder narrowing for Postman (keep cheap).
        if !want.is_whole() {
            return Err(CoreError::InvalidName(
                "Postman export does not narrow by folder or request yet — export the whole collection"
                    .to_string(),
            ));
        }
        let mut listed = Vec::new();
        for path in paths(node) {
            let request = collection::load_request_source(workspace, &node.slug, &path)?;
            listed.push(IncludedRequest {
                path,
                name: request.name,
            });
        }
        included.request_count = listed.len();
        included.collections.push(IncludedCollection {
            slug: node.slug.clone(),
            name: node.name.clone(),
            requests: listed,
        });
        postman::collection_json(workspace, node, &mut warnings)?
    } else {
        let doc = chosen_envs[0];
        for (key, def) in &doc.vars {
            match def {
                VarDef::Shared { .. } => continue,
                VarDef::Secret { .. } => excluded.secret_values += 1,
                VarDef::Local { .. } => excluded.local_values += 1,
            }
            excluded.withheld_names.push(format!("{}.{key}", doc.name));
        }
        included.environments.push(doc.name.clone());
        postman::environment_json(doc)?
    };

    let findings = scan::scan_text(Path::new("<postman>"), &json);
    let token = review::token("export", std::slice::from_ref(&json))?;
    Ok(ExportPlan {
        bytes: json.len(),
        blocked: !findings.is_empty(),
        included,
        excluded,
        findings,
        token,
        format: "postman".to_string(),
        warnings: warnings.warnings,
        json,
    })
}

/// Writes the payload — and only the payload a plan with this exact token
/// described. A finding stops it before the file is created; `force` is the one
/// override and it applies to this call alone.
pub fn run_export(
    workspace: &Path,
    selection: &ExportSelection,
    token: &str,
    dest: &Path,
    force: bool,
) -> CoreResult<ExportReceipt> {
    run_export_as(workspace, selection, None, token, dest, force)
}

pub fn run_export_as(
    workspace: &Path,
    selection: &ExportSelection,
    format: Option<ShareFormat>,
    token: &str,
    dest: &Path,
    force: bool,
) -> CoreResult<ExportReceipt> {
    let plan = plan_export_as(workspace, selection, format)?;
    if plan.token != token {
        return Err(review::stale("export"));
    }
    if plan.blocked && !force {
        return Err(CoreError::Secret(format!(
            "export stopped: {} credential-looking literal(s) would be written to {} — {}",
            plan.findings.len(),
            dest.display(),
            summarize(&plan.findings)
        )));
    }
    std::fs::write(dest, &plan.json).map_err(|e| CoreError::io(dest.display(), e))?;
    Ok(ExportReceipt {
        path: dest.display().to_string(),
        bytes: plan.bytes,
        requests: plan.included.request_count,
        collections: plan.included.collections.len(),
        environments: plan.included.environments.len(),
        forced: force && plan.blocked,
    })
}

fn summarize(findings: &[Finding]) -> String {
    findings
        .iter()
        .take(10)
        .map(|f| format!("line {} ({} · {})", f.line, f.rule, f.excerpt))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The whole workspace as a payload, with no file written. Everything that puts
/// a bundle somewhere goes through `plan_export` + `run_export` instead.
pub fn export(workspace: &Path) -> CoreResult<ExportBundle> {
    let plan = plan_export(workspace, &ExportSelection::default())?;
    Ok(ExportBundle {
        json: plan.json,
        findings: plan.findings,
    })
}

/// One file of the import, already rendered. Nothing is written until every file
/// of the whole bundle has one of these: a bundle that cannot be reproduced in
/// full leaves the workspace exactly as it was.
struct PlannedFile {
    slug: String,
    file: String,
    contents: String,
    requests: usize,
}

struct Plan {
    collections: Vec<BundleCollection>,
    files: Vec<PlannedFile>,
    environments: Vec<workspace::EnvDoc>,
    summary: String,
}

/// The requests of one file, in the order their `#index` addresses put them. The
/// indexes have to be the whole run `0..n`, or the file cannot be rebuilt with the
/// identities the bundle carries.
fn group_by_file(entries: &[BundleRequest]) -> CoreResult<Vec<(String, Vec<SavedRequest>)>> {
    let mut order: Vec<String> = Vec::new();
    let mut by_file: std::collections::BTreeMap<String, Vec<(usize, SavedRequest)>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        let (file, fragment) = collection::split_identity(&entry.path);
        let index = match fragment {
            Some(fragment) => fragment.parse::<usize>().map_err(|_| {
                CoreError::Parse(format!(
                    "bundle request {:?}: a bundle addresses a request by index, not by name",
                    entry.path
                ))
            })?,
            None => by_file.get(file).map(Vec::len).unwrap_or(0),
        };
        if !by_file.contains_key(file) {
            order.push(file.to_string());
        }
        by_file
            .entry(file.to_string())
            .or_default()
            .push((index, entry.request.clone()));
    }
    let mut out = Vec::with_capacity(order.len());
    for file in order {
        let mut found = by_file.remove(&file).unwrap_or_default();
        found.sort_by_key(|(index, _)| *index);
        for (position, (index, _)) in found.iter().enumerate() {
            if *index != position {
                return Err(CoreError::Parse(format!(
                    "bundle file {file} jumps from request {position} to {index} — a bundle carries every request of a file"
                )));
            }
        }
        out.push((file, found.into_iter().map(|(_, r)| r).collect()));
    }
    Ok(out)
}

fn plan(workspace: &Path, bundle: IncomingBundle) -> CoreResult<Plan> {
    let mut environments = Vec::with_capacity(bundle.environments.len());
    for raw in bundle.environments {
        environments.push(raw.validate("bundle environment")?);
    }
    let mut files = Vec::new();
    let collections = match bundle.mandalo_bundle {
        1 => {
            let default = BundleCollection {
                id: uuid::Uuid::new_v4().to_string(),
                slug: "default".to_string(),
                name: "Default".to_string(),
                requests: Vec::new(),
            };
            let mut taken: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for request in &bundle.requests {
                collection::validate_request(request)?;
                let extension = match request.kind.as_str() {
                    "grpc" => "grpc",
                    "websocket" => "ws",
                    "mqtt" => "mqtt",
                    _ => "http",
                };
                let stem = collection::slugify(&request.name);
                let mut file = format!("{stem}.{extension}");
                let mut n = 2;
                while !taken.insert(file.clone()) {
                    file = format!("{stem}-{n}.{extension}");
                    n += 1;
                }
                files.push(PlannedFile {
                    slug: default.slug.clone(),
                    contents: collection::plan_file(
                        workspace,
                        &default.slug,
                        &file,
                        std::slice::from_ref(request),
                    )?,
                    file,
                    requests: 1,
                });
            }
            vec![default]
        }
        2 => {
            for c in &bundle.collections {
                for (file, requests) in group_by_file(&c.requests)? {
                    files.push(PlannedFile {
                        contents: collection::plan_file(workspace, &c.slug, &file, &requests)?,
                        slug: c.slug.clone(),
                        file,
                        requests: requests.len(),
                    });
                }
            }
            bundle.collections
        }
        other => {
            return Err(CoreError::Unsupported(format!(
                "unsupported bundle version: {other} (expected 1 or {BUNDLE_VERSION})"
            )))
        }
    };
    Ok(Plan {
        summary: match bundle.mandalo_bundle {
            1 => "Version 1 bundle imported into the Default collection and upgraded to version 2."
                .to_string(),
            _ => {
                "Mándalo bundle imported; requests keep their original ids and folders.".to_string()
            }
        },
        collections,
        files,
        environments,
    })
}

/// Imports a bundle whole or not at all: every file is rendered and every
/// environment validated first, so a bundle that fails halfway leaves no
/// half-written workspace behind.
pub fn import(workspace: &Path, json: &str) -> CoreResult<ImportReport> {
    let bundle: IncomingBundle = serde_json::from_str(json)
        .map_err(|e| CoreError::Parse(format!("invalid bundle JSON: {e}")))?;
    let plan = plan(workspace, bundle)?;

    let mut report = ImportReport {
        imported: 0,
        collections: plan.collections.len(),
        environments: plan.environments.len(),
        skipped: Vec::new(),
        warnings: Vec::new(),
        summary: plan.summary,
    };
    for c in &plan.collections {
        collection::ensure_collection(workspace, &c.slug, &c.name, Some(&c.id))?;
    }
    for planned in &plan.files {
        collection::write_file(workspace, &planned.slug, &planned.file, &planned.contents)?;
        report.imported += planned.requests;
    }
    for env in &plan.environments {
        workspace::save_env_doc(workspace, env)?;
    }
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
            stream: None,
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
            &crate::capability::RefuseLocalWrites,
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
