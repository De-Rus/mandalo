use crate::error::{CoreError, CoreResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

const BEGIN: &str = "# >>> mandalo — managed block, edits here are overwritten";
const END: &str = "# <<< mandalo";

/// Everything that could hold a secret value next to a committed workspace.
pub const IGNORED: &[&str] = &[
    "*.local.toml",
    ".mandalo/",
    "secrets.toml",
    "*.secret.toml",
    ".env",
    ".env.*",
];

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GitHygiene {
    pub gitignore_written: bool,
    pub hook_installed: bool,
}

pub fn precommit_hook_script() -> String {
    format!(
        "#!/bin/sh\n{BEGIN}\n# Refuses a commit that stages a credential-looking literal.\nif ! mandalo scan --staged; then\n  echo \"mandalo: staged files look like they carry credentials — commit stopped\" >&2\n  exit 1\nfi\n{END}\n"
    )
}

fn gitignore_path(workspace: &Path) -> PathBuf {
    workspace.join(".gitignore")
}

fn hook_path(workspace: &Path) -> PathBuf {
    workspace.join(".git").join("hooks").join("pre-commit")
}

fn managed_block() -> String {
    let mut block = String::from(BEGIN);
    block.push('\n');
    for pattern in IGNORED {
        block.push_str(pattern);
        block.push('\n');
    }
    block.push_str(END);
    block
}

/// Replaces the managed block, leaving every other line of the user's
/// `.gitignore` untouched. Returns the file contents it wants on disk.
pub fn render_gitignore(existing: &str) -> String {
    let block = managed_block();
    let (before, after) = match (existing.find(BEGIN), existing.find(END)) {
        (Some(start), Some(end)) if end > start => {
            let tail = existing[end + END.len()..].trim_start_matches('\n');
            (existing[..start].to_string(), tail.to_string())
        }
        _ => {
            let head = if existing.is_empty() {
                String::new()
            } else {
                format!("{}\n\n", existing.trim_end_matches('\n'))
            };
            (head, String::new())
        }
    };
    let mut out = before;
    out.push_str(&block);
    out.push('\n');
    if !after.is_empty() {
        out.push_str(&after);
    }
    out
}

pub fn hook_is_installed(workspace: &Path) -> bool {
    std::fs::read_to_string(hook_path(workspace))
        .map(|s| s.contains("mandalo scan --staged"))
        .unwrap_or(false)
}

/// Writes the managed `.gitignore` block. Never installs the hook — that needs
/// its own call, because a hook is a program the user did not ask us to run.
pub fn ensure_git_hygiene(workspace: &Path) -> CoreResult<GitHygiene> {
    if !workspace.is_dir() {
        return Err(CoreError::NotFound(format!(
            "workspace directory does not exist: {}",
            workspace.display()
        )));
    }
    let path = gitignore_path(workspace);
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(CoreError::io(path.display(), e)),
    };
    let wanted = render_gitignore(&existing);
    let gitignore_written = wanted != existing;
    if gitignore_written {
        crate::workspace::atomic_write(&path, &wanted)?;
    }
    Ok(GitHygiene {
        gitignore_written,
        hook_installed: hook_is_installed(workspace),
    })
}

pub fn install_precommit_hook(workspace: &Path) -> CoreResult<PathBuf> {
    let hooks = workspace.join(".git").join("hooks");
    if !workspace.join(".git").is_dir() {
        return Err(CoreError::NotFound(format!(
            "{} is not a git repository",
            workspace.display()
        )));
    }
    let path = hook_path(workspace);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if !existing.contains("mandalo scan --staged") {
            return Err(CoreError::Conflict(format!(
                "{} already has a pre-commit hook; add `mandalo scan --staged` to it by hand",
                path.display()
            )));
        }
    }
    std::fs::create_dir_all(&hooks).map_err(|e| CoreError::io(hooks.display(), e))?;
    crate::workspace::atomic_write(&path, &precommit_hook_script())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| CoreError::io(path.display(), e))?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_managed_block_into_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let result = ensure_git_hygiene(dir.path()).unwrap();
        assert!(result.gitignore_written);
        assert!(!result.hook_installed);
        let raw = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        for pattern in IGNORED {
            assert!(raw.contains(pattern), "{raw}");
        }
    }

    #[test]
    fn is_idempotent_and_preserves_user_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n*.log\n").unwrap();
        assert!(ensure_git_hygiene(dir.path()).unwrap().gitignore_written);
        let first = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(first.starts_with("node_modules/\n*.log\n"));
        assert!(!ensure_git_hygiene(dir.path()).unwrap().gitignore_written);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            first
        );
    }

    #[test]
    fn user_lines_after_the_block_survive_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        ensure_git_hygiene(dir.path()).unwrap();
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("dist/\n");
        std::fs::write(&path, &raw).unwrap();
        std::fs::write(
            &path,
            std::fs::read_to_string(&path).unwrap().replace("*.env", ""),
        )
        .unwrap();
        ensure_git_hygiene(dir.path()).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.ends_with("dist/\n"), "{after}");
        assert_eq!(after.matches(BEGIN).count(), 1);
        assert!(after.contains(".mandalo/"));
    }

    #[test]
    fn the_hook_is_never_installed_by_hygiene() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("hooks")).unwrap();
        ensure_git_hygiene(dir.path()).unwrap();
        assert!(!hook_path(dir.path()).exists());
    }

    #[test]
    fn installing_the_hook_is_explicit_and_executable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let path = install_precommit_hook(dir.path()).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("mandalo scan --staged"));
        assert!(hook_is_installed(dir.path()));
        assert!(ensure_git_hygiene(dir.path()).unwrap().hook_installed);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        install_precommit_hook(dir.path()).unwrap();
    }

    #[test]
    fn a_foreign_hook_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("hooks")).unwrap();
        std::fs::write(hook_path(dir.path()), "#!/bin/sh\nmake lint\n").unwrap();
        let err = install_precommit_hook(dir.path()).unwrap_err();
        assert_eq!(err.code(), "E_CONFLICT");
        assert_eq!(
            std::fs::read_to_string(hook_path(dir.path())).unwrap(),
            "#!/bin/sh\nmake lint\n"
        );
    }

    #[test]
    fn hygiene_fails_loud_on_a_missing_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_git_hygiene(&dir.path().join("ghost")).unwrap_err();
        assert_eq!(err.code(), "E_NOT_FOUND");
    }
}
