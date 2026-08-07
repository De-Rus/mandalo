use crate::collection;
use crate::error::{CoreError, CoreResult};
use crate::postman::{self, ExportWarnings};
use crate::workspace::{self, ShareConfig, ShareFormat};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeReport {
    pub paths: Vec<String>,
    pub warnings: Vec<String>,
    pub dir: String,
}

/// Resolve the active share config. Absent stanza → native (no materialize).
pub fn active(workspace: &Path) -> CoreResult<Option<ShareConfig>> {
    Ok(workspace::share_config(workspace)?.filter(|c| c.is_postman()))
}

fn ensure_relative_dir(name: &str) -> CoreResult<&str> {
    let name = name.trim().trim_matches('/');
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('/')
        || Path::new(name).is_absolute()
    {
        return Err(CoreError::PathEscape(format!(
            "share.dir {name:?} must be a relative directory under the workspace"
        )));
    }
    Ok(name)
}

/// Write Postman JSON mirrors under `share.dir` when `[share] format = "postman"`.
/// Native / absent config is a no-op. Returns the workspace-relative paths written.
pub fn materialize(workspace: &Path) -> CoreResult<MaterializeReport> {
    let Some(share) = active(workspace)? else {
        return Ok(MaterializeReport::default());
    };
    let dir_name = ensure_relative_dir(share.dir_name())?;
    let root = workspace.join(dir_name);
    std::fs::create_dir_all(root.join("environments"))
        .map_err(|e| CoreError::io(root.display(), e))?;

    let tree = collection::list_tree(workspace)?;
    let envs = workspace::list_env_docs(workspace)?;

    let mut warnings = ExportWarnings::default();
    for skip in tree.skipped.iter().chain(envs.skipped.iter()) {
        warnings.push(format!("skipped while building Postman share: {skip}"));
    }

    let mut paths = Vec::new();

    for node in &tree.collections {
        let json = postman::collection_json(workspace, node, &mut warnings)?;
        let rel = format!("{dir_name}/{}.json", node.slug);
        let dest = workspace.join(&rel);
        std::fs::write(&dest, json).map_err(|e| CoreError::io(dest.display(), e))?;
        paths.push(rel);
    }

    for doc in &envs.items {
        let json = postman::environment_json(doc)?;
        let rel = format!("{dir_name}/environments/{}.json", doc.name);
        let dest = workspace.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;
        }
        std::fs::write(&dest, json).map_err(|e| CoreError::io(dest.display(), e))?;
        paths.push(rel);
    }

    // Drop stale generated files that no longer have a source collection/env.
    prune_stale(&root, dir_name, &paths)?;

    Ok(MaterializeReport {
        paths,
        warnings: warnings.warnings,
        dir: dir_name.to_string(),
    })
}

fn prune_stale(root: &Path, dir_name: &str, keep: &[String]) -> CoreResult<()> {
    if !root.exists() {
        return Ok(());
    }
    let keep: std::collections::BTreeSet<&str> = keep.iter().map(String::as_str).collect();
    for entry in walkdir(root)? {
        let rel_inside = entry
            .strip_prefix(root)
            .map_err(|_| {
                CoreError::PathEscape(format!(
                    "{} is outside {}",
                    entry.display(),
                    root.display()
                ))
            })?
            .display()
            .to_string()
            .replace('\\', "/");
        let rel = format!("{dir_name}/{rel_inside}");
        if keep.contains(rel.as_str()) || !rel.ends_with(".json") {
            continue;
        }
        let _ = std::fs::remove_file(&entry);
    }
    Ok(())
}

fn walkdir(root: &Path) -> CoreResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> CoreResult<()> {
        for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir.display(), e))? {
            let entry = entry.map_err(|e| CoreError::io(dir.display(), e))?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    walk(root, &mut out)?;
    Ok(out)
}

pub fn format_label(format: ShareFormat) -> &'static str {
    match format {
        ShareFormat::Native => "native",
        ShareFormat::Postman => "postman",
    }
}
