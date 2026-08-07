use crate::collection;
use crate::error::{CoreError, CoreResult};
use crate::remote;
use crate::workspace;
use std::path::Path;

include!(concat!(env!("OUT_DIR"), "/sample_files.rs"));

/// The shipped sample workspace, path and contents, sorted by path. The very
/// files `examples/mock-workspace` holds — the browser inlines the same set at
/// build time, so neither host has a copy of its own to drift.
pub fn files() -> &'static [(&'static str, &'static str)] {
    SAMPLE_FILES
}

const SAMPLE_SLUG: &str = "mock";

fn taken_slugs(workspace: &Path) -> CoreResult<Vec<String>> {
    let dir = collection::collections_dir(workspace);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| CoreError::io(dir.display(), e))?;
        if let Some(name) = entry.file_name().to_str() {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

/// Copies the shipped sample collection into a workspace under a free slug,
/// touching nothing already there — a user who deleted it, or who started from
/// an empty workspace, can always get a collection back to send from. Supporting
/// files the sample's requests point at (`protos/`, `files/`) come along; an
/// environment is written only when the workspace has none by that name.
///
/// Returns the slug it landed under, which is `mock` the first time and
/// `mock-2`, `mock-3`… after that.
pub fn add_sample_collection(workspace: &Path) -> CoreResult<String> {
    remote::ensure_writable(workspace)?;
    if workspace::read_manifest(workspace)?.is_none() {
        return Err(CoreError::NotFound(format!(
            "{} is not a Mándalo workspace yet — open it first",
            workspace.display()
        )));
    }
    let taken = taken_slugs(workspace)?;
    let mut slug = SAMPLE_SLUG.to_string();
    let mut n = 2;
    while taken.contains(&slug) {
        slug = format!("{SAMPLE_SLUG}-{n}");
        n += 1;
    }

    let prefix = format!("collections/{SAMPLE_SLUG}/");
    let mut copied = 0usize;
    for (path, text) in files() {
        if let Some(rest) = path.strip_prefix(&prefix) {
            let target =
                collection::resolve_within(workspace, &format!("collections/{slug}/{rest}"))?;
            write_new(&target, text)?;
            copied += 1;
            continue;
        }
        if path.starts_with("collections/") || *path == "mandalo.toml" {
            continue;
        }
        let target = collection::resolve_within(workspace, path)?;
        if !target.exists() {
            write_new(&target, text)?;
        }
    }
    if copied == 0 {
        return Err(CoreError::NotFound(
            "the sample collection is missing from this build: examples/mock-workspace shipped no requests".to_string(),
        ));
    }
    Ok(slug)
}

fn write_new(target: &Path, text: &str) -> CoreResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;
    }
    workspace::atomic_write(target, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_carries_the_sample_collection_and_its_supporting_files() {
        let paths: Vec<&str> = files().iter().map(|(p, _)| *p).collect();
        assert!(paths.contains(&"mandalo.toml"));
        assert!(paths.iter().any(|p| p.starts_with("collections/mock/")));
        assert!(paths.iter().any(|p| p.ends_with(".http")));
        assert!(
            paths.windows(2).all(|w| w[0] < w[1]),
            "the table has to be sorted so both hosts write the same tree"
        );
    }

    #[test]
    fn no_binary_fixture_rides_along() {
        assert!(files().iter().all(|(p, _)| !p.ends_with(".pdf")));
    }
}
