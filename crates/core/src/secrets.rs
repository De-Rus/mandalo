use crate::capability::{SecretStore, SecretWriter, VarSource};
use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

/// Points the store somewhere else, for CI and for tests that must not touch
/// `$HOME`.
pub const PATH_ENV: &str = "MANDALO_SECRETS_FILE";

/// The one file that holds the values this machine keeps to itself.
///
/// It lives outside every workspace on purpose. A file inside a repository can
/// only be kept out of git by a rule, and a rule can be deleted, bypassed with
/// `git add -f`, or missed entirely when someone zips the folder to a colleague.
/// A file that was never inside the repository cannot be committed at all.
pub fn secrets_path() -> CoreResult<PathBuf> {
    if let Some(raw) = std::env::var_os(PATH_ENV) {
        let path = PathBuf::from(raw);
        if path.as_os_str().is_empty() {
            return Err(CoreError::Io(format!(
                "{PATH_ENV} is set but empty; unset it or point it at a file"
            )));
        }
        return Ok(path);
    }
    Ok(crate::workspace::config_dir()?.join("secrets.toml"))
}

type Section = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Serialize, Deserialize, Default)]
struct SecretsFile {
    #[serde(default)]
    schema_version: Option<u32>,
    /// Identities that belong to the person, not to any one workspace.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    auth: Section,
    /// Workspace **id** → environment → variable. Keying by id, not by path,
    /// means moving or re-cloning a workspace keeps its values attached.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workspaces: BTreeMap<String, Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    Auth,
    Workspace(String),
}

/// Reads and writes the machine-local values of one scope.
#[derive(Debug, Clone)]
pub struct LocalStore {
    /// `Err` carries *why* there is nowhere to keep values — no `$HOME`, or a
    /// directory that is not a workspace. Reading answers "nothing here", so a
    /// run that needs no local value works in a bare container; writing fails
    /// with that reason, because a write that goes nowhere is a lie.
    location: Result<PathBuf, String>,
    scope: Scope,
}

impl LocalStore {
    pub fn for_workspace(workspace_id: impl Into<String>) -> Self {
        LocalStore {
            location: secrets_path().map_err(|e| e.to_string()),
            scope: Scope::Workspace(workspace_id.into()),
        }
    }

    pub fn for_auth() -> Self {
        LocalStore {
            location: secrets_path().map_err(|e| e.to_string()),
            scope: Scope::Auth,
        }
    }

    /// A store with nowhere to keep anything, and the reason to say so.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        LocalStore {
            location: Err(reason.into()),
            scope: Scope::Auth,
        }
    }

    pub fn at(path: impl Into<PathBuf>, workspace_id: impl Into<String>) -> Self {
        LocalStore {
            location: Ok(path.into()),
            scope: Scope::Workspace(workspace_id.into()),
        }
    }

    pub fn auth_at(path: impl Into<PathBuf>) -> Self {
        LocalStore {
            location: Ok(path.into()),
            scope: Scope::Auth,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.location.as_deref().ok()
    }

    fn writable_path(&self) -> CoreResult<&Path> {
        self.location
            .as_deref()
            .map_err(|reason| CoreError::Secret(format!("nowhere to keep this value: {reason}")))
    }

    fn section<'a>(&self, file: &'a SecretsFile) -> Option<&'a Section> {
        match &self.scope {
            Scope::Auth => Some(&file.auth),
            Scope::Workspace(id) => file.workspaces.get(id),
        }
    }

    fn section_mut<'a>(&self, file: &'a mut SecretsFile) -> &'a mut Section {
        match &self.scope {
            Scope::Auth => &mut file.auth,
            Scope::Workspace(id) => file.workspaces.entry(id.clone()).or_default(),
        }
    }

    fn prune(&self, file: &mut SecretsFile) {
        if let Scope::Workspace(id) = &self.scope {
            if file
                .workspaces
                .get(id)
                .map(BTreeMap::is_empty)
                .unwrap_or(false)
            {
                file.workspaces.remove(id);
            }
        }
    }

    /// Which environments of this scope hold a value here. A workspace copied
    /// from a colleague answers "none", which is the correct onboarding state.
    pub fn envs(&self) -> CoreResult<Vec<String>> {
        let file = self.read()?;
        Ok(self
            .section(&file)
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default())
    }

    /// Every variable of `env` this machine holds a value for.
    pub fn keys(&self, env: &str) -> CoreResult<Vec<String>> {
        let file = self.read()?;
        Ok(self
            .section(&file)
            .and_then(|s| s.get(env))
            .map(|vars| vars.keys().cloned().collect())
            .unwrap_or_default())
    }

    fn read(&self) -> CoreResult<SecretsFile> {
        let Ok(path) = self.location.as_deref() else {
            return Ok(SecretsFile::default());
        };
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SecretsFile::default()),
            Err(e) => return Err(CoreError::io(path.display(), e)),
        };
        check_permissions(path)?;
        let file: SecretsFile =
            toml::from_str(&raw).map_err(|e| CoreError::parse(path.display(), e))?;
        if let Some(version) = file.schema_version {
            if version != SCHEMA_VERSION {
                return Err(CoreError::Schema(format!(
                    "{}: unsupported schema_version {version} (expected {SCHEMA_VERSION})",
                    path.display()
                )));
            }
        }
        self.reject_empty_values(&file)?;
        Ok(file)
    }

    /// An empty value is never a value. Catching it here means a hand-edited
    /// `token = ""` fails by name instead of being sent as an empty credential.
    fn reject_empty_values(&self, file: &SecretsFile) -> CoreResult<()> {
        let Some(section) = self.section(file) else {
            return Ok(());
        };
        for (env, vars) in section {
            for (key, value) in vars {
                if value.is_empty() {
                    return Err(CoreError::Schema(format!(
                        "{}: {env}.{key} is empty; give it a value or delete the line",
                        self.location.as_deref().unwrap_or(Path::new("?")).display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn write(&self, file: &SecretsFile) -> CoreResult<()> {
        let path = self.writable_path()?.to_path_buf();
        let raw = toml::to_string_pretty(file).map_err(|e| CoreError::Parse(e.to_string()))?;
        write_private(&path, &format!("{HEADER}{raw}"))
    }
}

const HEADER: &str =
    "# Mándalo — the values this machine keeps to itself. Never commit or share this file.\n";

impl SecretStore for LocalStore {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>> {
        let file = self.read()?;
        Ok(self
            .section(&file)
            .and_then(|s| s.get(env))
            .and_then(|vars| vars.get(key))
            .cloned())
    }

    fn source(&self, env: &str, key: &str) -> CoreResult<Option<VarSource>> {
        Ok(self.get(env, key)?.map(|_| VarSource::Local))
    }
}

impl SecretWriter for LocalStore {
    fn set(&self, env: &str, key: &str, value: &str) -> CoreResult<()> {
        if value.is_empty() {
            return Err(CoreError::Secret(format!(
                "{env}.{key} was given an empty value; clear it instead of storing nothing"
            )));
        }
        self.writable_path()?;
        let mut file = self.read()?;
        file.schema_version = Some(SCHEMA_VERSION);
        self.section_mut(&mut file)
            .entry(env.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        self.write(&file)
    }

    fn delete(&self, env: &str, key: &str) -> CoreResult<()> {
        let mut file = self.read()?;
        file.schema_version = Some(SCHEMA_VERSION);
        {
            let section = self.section_mut(&mut file);
            let empty = match section.get_mut(env) {
                Some(vars) => {
                    vars.remove(key);
                    vars.is_empty()
                }
                None => return Ok(()),
            };
            if empty {
                section.remove(env);
            }
        }
        self.prune(&mut file);
        self.write(&file)
    }
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> CoreResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| CoreError::io(path.display(), e))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(CoreError::Secret(format!(
            "{} is readable by other users (mode {mode:04o}) — run: chmod 600 {}",
            path.display(),
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> CoreResult<()> {
    Ok(())
}

/// Writes so that the contents are never, even briefly, readable by another
/// user: the temporary file is created `0600` before anything is written to it.
fn write_private(path: &Path, contents: &str) -> CoreResult<()> {
    use std::io::Write;

    let parent = path.parent().ok_or_else(|| {
        CoreError::Io(format!("path has no parent directory: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| CoreError::io(parent.display(), e))?;
    }
    let name = path.file_name().and_then(|f| f.to_str()).ok_or_else(|| {
        CoreError::InvalidName(format!("path has no file name: {}", path.display()))
    })?;
    let tmp = parent.join(format!(".{name}.tmp"));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options
        .open(&tmp)
        .map_err(|e| CoreError::io(tmp.display(), e))?;
    handle
        .write_all(contents.as_bytes())
        .map_err(|e| CoreError::io(tmp.display(), e))?;
    handle
        .sync_all()
        .map_err(|e| CoreError::io(tmp.display(), e))?;
    drop(handle);
    std::fs::rename(&tmp, path).map_err(|e| CoreError::io(path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn store(dir: &tempfile::TempDir) -> LocalStore {
        LocalStore::at(dir.path().join("config").join("secrets.toml"), WS)
    }

    #[test]
    fn a_missing_file_is_an_empty_store_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(store(&dir).get("prod", "token").unwrap(), None);
        assert!(store(&dir).envs().unwrap().is_empty());
    }

    #[test]
    fn round_trips_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        s.set("prod", "token", "t0p").unwrap();
        s.set("staging", "baseUrl", "http://localhost:3000")
            .unwrap();
        assert_eq!(s.get("prod", "token").unwrap(), Some("t0p".to_string()));
        assert_eq!(s.envs().unwrap(), vec!["prod", "staging"]);
        assert_eq!(s.keys("prod").unwrap(), vec!["token"]);

        s.delete("prod", "token").unwrap();
        assert_eq!(s.get("prod", "token").unwrap(), None);
        assert_eq!(s.envs().unwrap(), vec!["staging"]);
        s.delete("prod", "token").unwrap();
    }

    #[test]
    fn scopes_cannot_read_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let mine = LocalStore::at(&path, WS);
        let theirs = LocalStore::at(&path, "11111111-2222-4333-8444-555555555555");
        let auth = LocalStore::auth_at(&path);

        mine.set("prod", "token", "mine").unwrap();
        theirs.set("prod", "token", "theirs").unwrap();
        auth.set("github", "token", "gho_x").unwrap();

        assert_eq!(mine.get("prod", "token").unwrap(), Some("mine".to_string()));
        assert_eq!(
            theirs.get("prod", "token").unwrap(),
            Some("theirs".to_string())
        );
        assert_eq!(auth.get("prod", "token").unwrap(), None);
        assert_eq!(mine.get("github", "token").unwrap(), None);
    }

    #[test]
    fn the_file_is_written_in_the_documented_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        LocalStore::auth_at(&path)
            .set("github", "token", "gho_fixture")
            .unwrap();
        LocalStore::at(&path, WS)
            .set("prod", "token", "t0p")
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("# Mándalo"), "{raw}");
        assert!(raw.contains("schema_version = 1"), "{raw}");
        assert!(raw.contains("[auth.github]"), "{raw}");
        // A workspace id is a UUID, so TOML writes it as a bare key.
        assert!(raw.contains(&format!("[workspaces.{WS}.prod]")), "{raw}");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("secrets.toml");
        LocalStore::at(&path, WS)
            .set("prod", "token", "t0p")
            .unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_file_fails_loud_instead_of_being_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let s = LocalStore::at(&path, WS);
        s.set("prod", "token", "t0p").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = s.get("prod", "token").unwrap_err();
        assert_eq!(err.code(), "E_SECRET");
        assert!(err.to_string().contains("chmod 600"), "{err}");
    }

    #[test]
    fn an_empty_value_is_refused_on_the_way_in_and_on_the_way_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let s = LocalStore::at(&path, WS);
        assert_eq!(s.set("prod", "token", "").unwrap_err().code(), "E_SECRET");

        s.set("prod", "token", "t0p").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap().replace("t0p", "");
        std::fs::write(&path, raw).unwrap();
        let err = s.get("prod", "token").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(err.to_string().contains("prod.token is empty"), "{err}");
    }

    #[test]
    fn a_future_schema_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        write_private(&path, "schema_version = 99\n").unwrap();
        let err = LocalStore::at(&path, WS).get("prod", "token").unwrap_err();
        assert_eq!(err.code(), "E_SCHEMA");
        assert!(err.to_string().contains("schema_version 99"), "{err}");
    }

    #[test]
    fn writes_leave_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("secrets.toml");
        LocalStore::at(&path, WS)
            .set("prod", "token", "t0p")
            .unwrap();
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["secrets.toml"]);
    }
}
