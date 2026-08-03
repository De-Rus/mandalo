use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

fn environments_dir(workspace: &Path) -> PathBuf {
    workspace.join("environments")
}

pub fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("invalid environment name: {name:?}"));
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

pub fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent directory: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| format!("path has no valid file name: {}", path.display()))?
    ));
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn list_environments(workspace: &Path) -> Result<EnvironmentList, String> {
    let dir = environments_dir(workspace);
    let mut list = EnvironmentList {
        items: Vec::new(),
        skipped: Vec::new(),
    };
    if !dir.exists() {
        return Ok(list);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|raw| toml::from_str::<Environment>(&raw).map_err(|e| e.to_string()));
        match parsed {
            Ok(env) => list.items.push(env),
            Err(e) => list.skipped.push(format!("{}: {e}", path.display())),
        }
    }
    list.items.sort_by(|a, b| a.name.cmp(&b.name));
    list.skipped.sort();
    Ok(list)
}

pub fn save_environment(workspace: &Path, env: &Environment) -> Result<PathBuf, String> {
    validate_env_name(&env.name)?;
    let dir = environments_dir(workspace);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = env_path(workspace, &env.name);
    let raw = toml::to_string_pretty(env).map_err(|e| e.to_string())?;
    atomic_write(&path, &raw)?;
    Ok(path)
}

pub fn delete_environment(workspace: &Path, name: &str) -> Result<(), String> {
    validate_env_name(name)?;
    std::fs::remove_file(env_path(workspace, name)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let staging = env("staging", &[("base", "https://staging.x.dev"), ("token", "t1")]);
        let prod = env("prod", &[("base", "https://x.dev")]);
        save_environment(dir.path(), &staging).unwrap();
        save_environment(dir.path(), &prod).unwrap();
        assert_eq!(
            list_environments(dir.path()).unwrap().items,
            vec![prod, staging]
        );
    }

    #[test]
    fn save_writes_readable_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = save_environment(dir.path(), &env("staging", &[("base", "b")])).unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("name = \"staging\""));
        assert!(raw.contains("base = \"b\""));
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &env("staging", &[])).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path().join("environments"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["staging.toml"]);
    }

    #[test]
    fn rejects_path_traversal_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_environment(dir.path(), &env("../evil", &[])).is_err());
        assert!(save_environment(dir.path(), &env("", &[])).is_err());
    }

    #[test]
    fn rejects_non_ascii_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_environment(dir.path(), &env("prodücción", &[])).is_err());
    }

    #[test]
    fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &env("tmp", &[])).unwrap();
        delete_environment(dir.path(), "tmp").unwrap();
        assert!(list_environments(dir.path()).unwrap().items.is_empty());
    }

    #[test]
    fn delete_rejects_traversal_and_absolute_names() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.toml");
        std::fs::write(&outside, "name = \"outside\"").unwrap();
        let err = delete_environment(dir.path(), "../outside").unwrap_err();
        assert!(err.contains("invalid environment name"));
        assert!(outside.exists());

        let abs = dir.path().join("abs.toml");
        std::fs::write(&abs, "name = \"abs\"").unwrap();
        let abs_name = abs.with_extension("");
        let err = delete_environment(dir.path(), abs_name.to_str().unwrap()).unwrap_err();
        assert!(err.contains("invalid environment name"));
        assert!(abs.exists());
    }

    #[test]
    fn corrupt_env_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        save_environment(dir.path(), &env("good", &[])).unwrap();
        let bad = dir.path().join("environments").join("bad.toml");
        std::fs::write(&bad, "not [ valid toml").unwrap();
        let list = list_environments(dir.path()).unwrap();
        assert_eq!(list.items, vec![env("good", &[])]);
        assert_eq!(list.skipped.len(), 1);
        assert!(list.skipped[0].contains("bad.toml"));
    }

    #[test]
    fn sanitize_maps_invalid_chars_and_dedupes_collisions() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sanitize_env_name(dir.path(), "Staging (EU)"), "Staging--EU-");
        save_environment(dir.path(), &env("Staging--EU-", &[])).unwrap();
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging (EU)"),
            "Staging--EU--2"
        );
        save_environment(dir.path(), &env("Staging--EU--2", &[])).unwrap();
        assert_eq!(
            sanitize_env_name(dir.path(), "Staging (EU)"),
            "Staging--EU--3"
        );
        assert_eq!(sanitize_env_name(dir.path(), "Staging--EU-"), "Staging--EU-");
        assert_eq!(sanitize_env_name(dir.path(), "···"), "---");
        assert_eq!(sanitize_env_name(dir.path(), ""), "imported");
    }
}
