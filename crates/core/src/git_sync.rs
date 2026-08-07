use crate::error::{CoreError, CoreResult};
use crate::{git, redact, review, scan};
use git2::{
    AnnotatedCommit, BranchType, Cred, CredentialType, ErrorClass, ErrorCode, FetchOptions, Oid,
    PushOptions, RemoteCallbacks, Repository, RepositoryInitOptions, Signature,
};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const REMOTE: &str = "origin";
pub const DIRTY_FILE_CAP: usize = 100;
pub const FALLBACK_NAME: &str = "Mándalo";
pub const FALLBACK_EMAIL: &str = "noreply@mandalo.dev";

/// Who a commit will be attributed to. `is_fallback` means the repository had no
/// `user.name`/`user.email`, so the UI should offer to set a real one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub name: String,
    pub email: String,
    pub is_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub detached: bool,
    pub remote_url: Option<String>,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    /// Index conflicts plus paths where local and remote both edited — the bar
    /// can open Resolve before Sync ever runs.
    pub conflicted: Vec<String>,
    pub dirty_files: Vec<String>,
    pub dirty_total: usize,
    pub identity: Option<Identity>,
}

impl SyncStatus {
    fn not_a_repo() -> Self {
        SyncStatus {
            is_repo: false,
            branch: None,
            detached: false,
            remote_url: None,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            conflicted: Vec::new(),
            dirty_files: Vec::new(),
            dirty_total: 0,
            identity: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncOutcome {
    /// Clean tree, nothing to send, nothing to receive.
    NothingToDo,
    /// Committed locally, but the workspace has no remote to push to.
    Committed { sha: String, identity: Identity },
    /// Local work is now on the remote.
    Pushed {
        sha: String,
        ahead: usize,
        identity: Identity,
    },
    /// Remote work was rebased in; there was nothing of ours to send.
    Pulled { sha: String, behind: usize },
    /// Local and remote edits touch the same lines. Neither the rebase nor the
    /// merge was left half-applied — the working tree is exactly as the user left
    /// it. `items` carries a visual preview of each side so the UI can pick
    /// without reading conflict markers.
    Conflicted {
        files: Vec<String>,
        #[serde(default)]
        items: Vec<ConflictItem>,
    },
    /// The remote or the repository state refused the operation.
    Rejected { reason: String },
}

/// One side of a conflict, with full text so the UI can render every difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictSidePreview {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Entire file contents (UTF-8 lossy). Absent when the side is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl ConflictSidePreview {
    fn missing() -> Self {
        ConflictSidePreview {
            exists: false,
            kind: None,
            method: None,
            name: None,
            detail: Some("Removed".to_string()),
            text: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    pub path: String,
    pub ours: ConflictSidePreview,
    pub theirs: ConflictSidePreview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictChoice {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDecision {
    pub path: String,
    pub choice: ConflictChoice,
    /// When set, written as-is (request-by-request merges from the UI).
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedBranch {
    pub branch: String,
    pub sha: String,
    pub url: Option<String>,
    pub identity: Identity,
}

/// How to authenticate against a remote. A token is never read ambiently — the
/// caller resolves it (the machine-local file, a CI variable) and hands it in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Auth {
    #[default]
    None,
    Token(String),
    SshAgent,
}

impl Auth {
    /// Registers the token with the process redactor before it can travel
    /// anywhere, so it cannot ride out inside an error message.
    pub fn token(value: &str) -> Self {
        redact::register("git", "token", value);
        Auth::Token(value.to_string())
    }

    /// Prefers a stored token (HTTPS) so Sync works on desktop and web. SSH
    /// remotes are rewritten to HTTPS when talking to GitHub with a token.
    pub fn for_url(url: &str, token: Option<&str>) -> Self {
        match token.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => Auth::token(t),
            None if is_ssh_url(url) => Auth::SshAgent,
            None => Auth::None,
        }
    }
}

fn is_ssh_url(url: &str) -> bool {
    url.starts_with("ssh://") || (url.contains('@') && url.contains(':') && !url.contains("://"))
}

/// `https://github.com/owner/repo.git` for any GitHub remote shape (HTTPS or SSH).
fn github_https_clone_url(url: &str) -> Option<String> {
    let parts = parse_remote(url)?;
    let host = parts.host.to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    Some(format!(
        "https://github.com/{}/{}.git",
        parts.owner, parts.repo
    ))
}

/// URL used on the wire for this auth. Token auth always speaks HTTPS to GitHub,
/// even when `origin` is stored as `git@…`.
fn transport_url(configured: &str, auth: &Auth) -> String {
    if matches!(auth, Auth::Token(_)) {
        if let Some(https) = github_https_clone_url(configured) {
            return https;
        }
    }
    configured.to_string()
}

fn map_git(context: &str, e: git2::Error) -> CoreError {
    let message = redact::scrub(&format!("{context}: {}", e.message()));
    match (e.code(), e.class()) {
        (ErrorCode::NotFound, _) => CoreError::NotFound(message),
        (ErrorCode::Exists, _) => CoreError::Conflict(message),
        (ErrorCode::Conflict | ErrorCode::MergeConflict | ErrorCode::Unmerged, _) => {
            CoreError::Conflict(message)
        }
        (ErrorCode::Auth | ErrorCode::Certificate, _) => CoreError::Network(message),
        (_, ErrorClass::Net | ErrorClass::Http | ErrorClass::Ssl | ErrorClass::Ssh) => {
            CoreError::Network(message)
        }
        _ => CoreError::Io(message),
    }
}

/// Drops any `user:password@` embedded in an HTTP(S) remote so a token that a
/// user pasted into their remote URL never reaches a UI, a log or a report.
pub fn sanitize_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    match rest.split_once('@') {
        Some((_, host)) if !host.is_empty() => format!("{scheme}://{host}"),
        _ => url.to_string(),
    }
}

struct RemoteParts {
    host: String,
    owner: String,
    repo: String,
}

fn parse_remote(url: &str) -> Option<RemoteParts> {
    let url = url.trim().trim_end_matches('/');
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let rest = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?.to_string(), path.to_string())
    } else {
        let (userhost, path) = url.split_once(':')?;
        let host = userhost
            .rsplit_once('@')
            .map(|(_, h)| h)
            .unwrap_or(userhost);
        (host.to_string(), path.to_string())
    };
    let path = path.trim_start_matches('/').trim_end_matches(".git");
    let mut segments = path.split('/');
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.to_string();
    if segments.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(RemoteParts { host, owner, repo })
}

/// The "open a pull request" link for a branch, when the remote is GitHub.
/// Returns `None` for any other host — we do not guess another forge's URL shape.
pub fn github_compare_url(remote_url: &str, branch: &str) -> Option<String> {
    let parts = parse_remote(remote_url)?;
    let host = parts.host.to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    Some(format!(
        "https://github.com/{}/{}/compare/{branch}?expand=1",
        parts.owner, parts.repo
    ))
}

fn callbacks(auth: &Auth) -> RemoteCallbacks<'static> {
    let auth = auth.clone();
    let attempts = Cell::new(0u32);
    let mut cb = RemoteCallbacks::new();
    cb.credentials(move |_url, username_from_url, allowed| {
        let n = attempts.get();
        attempts.set(n + 1);
        if n >= 4 {
            return Err(git2::Error::from_str(
                "the remote rejected every credential offered",
            ));
        }
        let user = username_from_url.unwrap_or("git");
        // libgit2 asks for the SSH username on its own turn, before the key.
        if allowed.contains(CredentialType::USERNAME) {
            return Cred::username(user);
        }
        match &auth {
            Auth::Token(token) if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) => {
                // Works for classic PATs, fine-grained tokens, and OAuth (gho_).
                Cred::userpass_plaintext("x-access-token", token)
            }
            Auth::Token(_) => Err(git2::Error::from_str(
                "the remote asked for a credential this token cannot satisfy — use an https GitHub remote",
            )),
            Auth::SshAgent | Auth::None if allowed.contains(CredentialType::SSH_KEY) => {
                Cred::ssh_key_from_agent(user)
            }
            Auth::SshAgent => Err(git2::Error::from_str(
                "the remote asked for a password, but only the SSH agent was offered",
            )),
            Auth::None => Err(git2::Error::from_str(
                "the remote requires authentication — sign in to GitHub or pass a token",
            )),
        }
    });
    cb
}

fn open(workspace: &Path) -> CoreResult<Option<Repository>> {
    if !workspace.is_dir() {
        return Err(CoreError::NotFound(format!(
            "workspace directory does not exist: {}",
            workspace.display()
        )));
    }
    match Repository::open(workspace) {
        Ok(repo) => {
            confine(&repo, workspace)?;
            Ok(Some(repo))
        }
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(map_git("cannot open the workspace repository", e)),
    }
}

fn open_required(workspace: &Path) -> CoreResult<Repository> {
    open(workspace)?.ok_or_else(|| {
        CoreError::NotFound(format!(
            "{} is not a git repository yet — connect it first",
            workspace.display()
        ))
    })
}

/// A commit must never reach outside the workspace the user asked us to sync.
fn confine(repo: &Repository, workspace: &Path) -> CoreResult<()> {
    let workdir = repo.workdir().ok_or_else(|| {
        CoreError::Unsupported(format!(
            "{} is a bare repository — there is nothing to sync",
            workspace.display()
        ))
    })?;
    let same = match (workdir.canonicalize(), workspace.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => workdir == workspace,
    };
    if !same {
        return Err(CoreError::PathEscape(format!(
            "{} is inside the repository at {} — sync only ever touches a workspace root",
            workspace.display(),
            workdir.display()
        )));
    }
    Ok(())
}

fn identity(repo: &Repository) -> Identity {
    let config = repo.config().ok();
    let read = |key: &str| {
        config
            .as_ref()
            .and_then(|c| c.get_string(key).ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    match (read("user.name"), read("user.email")) {
        (Some(name), Some(email)) => Identity {
            name,
            email,
            is_fallback: false,
        },
        _ => Identity {
            name: FALLBACK_NAME.to_string(),
            email: FALLBACK_EMAIL.to_string(),
            is_fallback: true,
        },
    }
}

fn signature(id: &Identity) -> CoreResult<Signature<'static>> {
    Signature::now(&id.name, &id.email).map_err(|e| map_git("cannot build a commit signature", e))
}

struct Head {
    branch: Option<String>,
    detached: bool,
    unborn: bool,
    oid: Option<Oid>,
}

fn head(repo: &Repository) -> CoreResult<Head> {
    match repo.head() {
        Ok(reference) => {
            let detached = repo.head_detached().unwrap_or(false);
            Ok(Head {
                branch: if detached {
                    None
                } else {
                    reference.shorthand().ok().map(str::to_string)
                },
                detached,
                unborn: false,
                oid: reference.target(),
            })
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
            let branch = repo.find_reference("HEAD").ok().and_then(|r| {
                r.symbolic_target()
                    .ok()
                    .flatten()
                    .map(|t| t.trim_start_matches("refs/heads/").to_string())
            });
            Ok(Head {
                branch,
                detached: false,
                unborn: true,
                oid: None,
            })
        }
        Err(e) => Err(map_git("cannot read HEAD", e)),
    }
}

fn remote_url(repo: &Repository) -> Option<String> {
    repo.find_remote(REMOTE)
        .ok()
        .and_then(|r| r.url().ok().map(sanitize_url))
}

fn upstream_oid(repo: &Repository, branch: &str) -> Option<Oid> {
    if let Ok(local) = repo.find_branch(branch, BranchType::Local) {
        if let Ok(up) = local.upstream() {
            if let Some(oid) = up.get().target() {
                return Some(oid);
            }
        }
    }
    repo.find_branch(&format!("{REMOTE}/{branch}"), BranchType::Remote)
        .ok()
        .and_then(|b| b.get().target())
}

fn status_options() -> git2::StatusOptions {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .include_unmodified(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    options
}

pub fn status(workspace: &Path) -> CoreResult<SyncStatus> {
    let Some(repo) = open(workspace)? else {
        return Ok(SyncStatus::not_a_repo());
    };
    let mut out = SyncStatus::not_a_repo();
    out.is_repo = true;
    out.identity = Some(identity(&repo));
    out.remote_url = remote_url(&repo);

    let head_info = head(&repo)?;
    out.branch = head_info.branch.clone();
    out.detached = head_info.detached;

    let statuses = repo
        .statuses(Some(&mut status_options()))
        .map_err(|e| map_git("cannot read the workspace status", e))?;
    let mut dirty = Vec::new();
    for entry in statuses.iter() {
        let bits = entry.status();
        let path = entry.path().unwrap_or_default().to_string();
        if bits.is_conflicted() {
            out.conflicted.push(path.clone());
        }
        if bits.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            out.staged += 1;
        }
        if bits.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            out.unstaged += 1;
        }
        if bits.contains(git2::Status::WT_NEW) {
            out.untracked += 1;
        }
        if !path.is_empty() {
            dirty.push(path);
        }
    }
    dirty.sort();
    dirty.dedup();
    out.dirty_total = dirty.len();
    dirty.truncate(DIRTY_FILE_CAP);
    out.dirty_files = dirty;

    if let (Some(branch), Some(local)) = (head_info.branch.as_deref(), head_info.oid) {
        if let Some(remote) = upstream_oid(&repo, branch) {
            let (ahead, behind) = repo
                .graph_ahead_behind(local, remote)
                .map_err(|e| map_git("cannot compare with the remote", e))?;
            out.ahead = ahead;
            out.behind = behind;
            if behind > 0 {
                let pending = pending_conflict_paths(
                    &repo,
                    workspace,
                    local,
                    remote,
                    ahead,
                    &out.dirty_files,
                )?;
                out.conflicted.extend(pending);
                out.conflicted.sort();
                out.conflicted.dedup();
            }
        }
    }
    Ok(out)
}

pub fn init(workspace: &Path, remote_url: Option<&str>) -> CoreResult<()> {
    if !workspace.is_dir() {
        return Err(CoreError::NotFound(format!(
            "workspace directory does not exist: {}",
            workspace.display()
        )));
    }
    let repo = match open(workspace)? {
        Some(repo) => repo,
        None => {
            let mut options = RepositoryInitOptions::new();
            options.initial_head("main").no_reinit(true);
            Repository::init_opts(workspace, &options)
                .map_err(|e| map_git("cannot create the repository", e))?
        }
    };
    git::ensure_git_hygiene(workspace)?;
    if let Some(url) = remote_url.map(str::trim).filter(|u| !u.is_empty()) {
        match repo.find_remote(REMOTE) {
            Ok(existing) => {
                let current = existing.url().unwrap_or_default();
                if current != url {
                    return Err(CoreError::Conflict(format!(
                        "{REMOTE} already points at {} — remove it before connecting a different remote",
                        sanitize_url(current)
                    )));
                }
            }
            Err(_) => {
                repo.remote(REMOTE, url)
                    .map_err(|e| map_git("cannot add the remote", e))?;
            }
        }
    }
    Ok(())
}

pub fn clone(url: &str, dest: &Path, auth: &Auth) -> CoreResult<()> {
    if dest.exists() {
        let empty = std::fs::read_dir(dest)
            .map_err(|e| CoreError::io(dest.display(), e))?
            .next()
            .is_none();
        if !empty {
            return Err(CoreError::Conflict(format!(
                "{} already exists and is not empty",
                dest.display()
            )));
        }
    }
    let url = transport_url(url, auth);
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(auth));
    git2::build::RepoBuilder::new()
        .fetch_options(fetch)
        .clone(&url, dest)
        .map_err(|e| map_git(&format!("cannot clone {}", sanitize_url(&url)), e))?;
    Ok(())
}

/// Every path git considers changed, with the one word that describes it.
fn changed_files(repo: &Repository) -> CoreResult<Vec<(String, FileChange)>> {
    let statuses = repo
        .statuses(Some(&mut status_options()))
        .map_err(|e| map_git("cannot read the workspace status", e))?;
    let mut files: Vec<(String, FileChange)> = Vec::new();
    for entry in statuses.iter() {
        let Ok(path) = entry.path() else { continue };
        files.push((path.to_string(), FileChange::of(entry.status())));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);
    Ok(files)
}

fn guard_secrets(workspace: &Path, files: &[String], force: bool) -> CoreResult<()> {
    if force {
        return Ok(());
    }
    let findings = scan_selected(workspace, files)?;
    if findings.is_empty() {
        return Ok(());
    }
    let detail = findings
        .iter()
        .take(10)
        .map(|f| {
            let shown = f
                .path
                .strip_prefix(workspace)
                .unwrap_or(&f.path)
                .display()
                .to_string();
            format!("{shown}:{} ({} · {})", f.line, f.rule, f.excerpt)
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(CoreError::Secret(format!(
        "sync stopped: {} credential-looking literal(s) would be committed — {detail}",
        findings.len()
    )))
}

fn scan_selected(workspace: &Path, files: &[String]) -> CoreResult<Vec<scan::Finding>> {
    let present: Vec<PathBuf> = files
        .iter()
        .map(|p| workspace.join(p))
        .filter(|p| p.is_file())
        .collect();
    scan::scan_files(&present)
}

/// Stages exactly the chosen paths. The index is first put back in step with
/// HEAD so that a file the user had already `git add`ed but left out of this
/// commit cannot ride along; its edit stays in the working tree.
fn stage_selected(repo: &Repository, workspace: &Path, files: &[String]) -> CoreResult<()> {
    if let Ok(head) = repo.head().and_then(|h| h.peel(git2::ObjectType::Commit)) {
        repo.reset_default(Some(&head), ["*"].iter())
            .map_err(|e| map_git("cannot reset the index", e))?;
    }
    let mut index = repo
        .index()
        .map_err(|e| map_git("cannot open the index", e))?;
    // Pathspecs are relative to the work tree, so nothing outside it can be staged.
    for file in files {
        let rel = Path::new(file);
        if workspace.join(rel).is_file() {
            index
                .add_path(rel)
                .map_err(|e| map_git("cannot stage changes", e))?;
        } else {
            index
                .remove_path(rel)
                .map_err(|e| map_git("cannot stage a deletion", e))?;
        }
    }
    unstage_local_values(&mut index)?;
    index
        .write()
        .map_err(|e| map_git("cannot write the index", e))?;
    Ok(())
}

/// `.gitignore` already keeps these out, but an ignore rule can be deleted and
/// `git add -f` bypasses it. This is the one file whose accidental commit is
/// catastrophic, so the staging path drops it whatever the rules say.
fn unstage_local_values(index: &mut git2::Index) -> CoreResult<()> {
    let doomed: Vec<PathBuf> = index
        .iter()
        .map(|entry| PathBuf::from(String::from_utf8_lossy(&entry.path).into_owned()))
        .filter(|path| scan::is_local_values_file(path))
        .collect();
    for path in doomed {
        index
            .remove_path(&path)
            .map_err(|e| map_git("cannot keep the local values file out of the commit", e))?;
    }
    Ok(())
}

fn commit_selected(
    repo: &Repository,
    workspace: &Path,
    files: &[String],
    message: &str,
    id: &Identity,
    head_info: &Head,
) -> CoreResult<Option<Oid>> {
    stage_selected(repo, workspace, files)?;
    let mut index = repo
        .index()
        .map_err(|e| map_git("cannot open the index", e))?;
    if index.has_conflicts() {
        return Err(CoreError::Conflict(
            "the workspace has unresolved conflicts — resolve them, then sync again".to_string(),
        ));
    }
    let tree_oid = index
        .write_tree()
        .map_err(|e| map_git("cannot write the tree", e))?;
    let parent = match head_info.oid {
        Some(oid) => Some(
            repo.find_commit(oid)
                .map_err(|e| map_git("cannot read the last commit", e))?,
        ),
        None => None,
    };
    if let Some(parent) = &parent {
        if parent.tree_id() == tree_oid {
            return Ok(None);
        }
    }
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| map_git("cannot read the tree", e))?;
    let sig = signature(id)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let message = if message.trim().is_empty() {
        "Update workspace"
    } else {
        message.trim()
    };
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(|e| map_git("cannot commit", e))?;
    Ok(Some(oid))
}

fn configured_remote_url(repo: &Repository) -> CoreResult<String> {
    let remote = repo
        .find_remote(REMOTE)
        .map_err(|e| map_git("cannot read the remote", e))?;
    Ok(remote.url().unwrap_or_default().to_string())
}

/// Opens `origin`, or a detached HTTPS remote when auth is a token against GitHub SSH.
fn open_transport_remote<'a>(repo: &'a Repository, auth: &Auth) -> CoreResult<git2::Remote<'a>> {
    let configured = configured_remote_url(repo)?;
    let url = transport_url(&configured, auth);
    if url != configured {
        return repo
            .remote_anonymous(&url)
            .map_err(|e| map_git("cannot open the remote over https", e));
    }
    repo.find_remote(REMOTE)
        .map_err(|e| map_git("cannot read the remote", e))
}

fn fetch(repo: &Repository, branch: &str, auth: &Auth) -> CoreResult<()> {
    let mut remote = open_transport_remote(repo, auth)?;
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(auth));
    let refspec = format!("+refs/heads/{branch}:refs/remotes/{REMOTE}/{branch}");
    match remote.fetch(&[refspec.as_str()], Some(&mut options), None) {
        Ok(()) => Ok(()),
        // A branch that does not exist on the remote yet is the first-push case.
        Err(e) if e.code() == ErrorCode::NotFound => Ok(()),
        Err(e) => Err(map_git("cannot fetch from the remote", e)),
    }
}

/// Brings the working tree and the index back in step with HEAD. Untracked and
/// ignored files are left alone. `keep_edits` is on whenever the user left files
/// out of this commit: their edits are NOT in any commit, so a forced checkout
/// would destroy them — a safe checkout stops instead.
fn checkout_head(repo: &Repository, keep_edits: bool) -> CoreResult<()> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Io("the repository has no working tree".to_string()))?;
    let preserved: Vec<(PathBuf, Vec<u8>)> = if keep_edits {
        changed_files(repo)?
            .into_iter()
            .filter_map(|(path, _)| {
                let abs = workdir.join(&path);
                std::fs::read(&abs).ok().map(|bytes| (abs, bytes))
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout
        .force()
        .remove_untracked(false)
        .remove_ignored(false);
    repo.checkout_head(Some(&mut checkout))
        .map_err(|e| map_git("cannot update the working tree", e))?;
    for (path, bytes) in preserved {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, bytes).map_err(|e| CoreError::io(path.display(), e))?;
    }
    Ok(())
}

fn fast_forward(repo: &Repository, branch: &str, target: Oid, keep_edits: bool) -> CoreResult<()> {
    let name = format!("refs/heads/{branch}");
    repo.reference(&name, target, true, "mandalo sync: fast-forward")
        .map_err(|e| map_git("cannot move the branch", e))?;
    repo.set_head(&name)
        .map_err(|e| map_git("cannot move HEAD", e))?;
    checkout_head(repo, keep_edits)
}

fn conflict_paths(repo: &Repository) -> Vec<String> {
    let Ok(index) = repo.index() else {
        return Vec::new();
    };
    let Ok(conflicts) = index.conflicts() else {
        return Vec::new();
    };
    let mut out: Vec<String> = conflicts
        .filter_map(|c| c.ok())
        .filter_map(|c| {
            c.our
                .or(c.their)
                .or(c.ancestor)
                .map(|e| String::from_utf8_lossy(&e.path).to_string())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn paths_changed_between(repo: &Repository, from: Oid, to: Oid) -> CoreResult<Vec<String>> {
    let from_tree = repo
        .find_commit(from)
        .map_err(|e| map_git("cannot read commit", e))?
        .tree()
        .map_err(|e| map_git("cannot read tree", e))?;
    let to_tree = repo
        .find_commit(to)
        .map_err(|e| map_git("cannot read commit", e))?
        .tree()
        .map_err(|e| map_git("cannot read tree", e))?;
    let diff = repo
        .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
        .map_err(|e| map_git("cannot diff commits", e))?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(|p| p.to_str())
            .unwrap_or_default();
        if !path.is_empty() {
            paths.push(path.to_string());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Paths both sides touched with different contents. Mandalo diffs these itself
/// (request-by-request in the UI) — not a git merge/pull conflict.
fn pending_conflict_paths(
    repo: &Repository,
    workspace: &Path,
    local: Oid,
    remote: Oid,
    ahead: usize,
    dirty: &[String],
) -> CoreResult<Vec<String>> {
    let base = repo
        .merge_base(local, remote)
        .map_err(|e| map_git("cannot find merge base with the remote", e))?;
    let remote_changed = paths_changed_between(repo, base, remote)?;
    if remote_changed.is_empty() {
        return Ok(Vec::new());
    }
    let mut local_touched: std::collections::BTreeSet<String> = dirty.iter().cloned().collect();
    if ahead > 0 {
        for path in paths_changed_between(repo, base, local)? {
            local_touched.insert(path);
        }
    }
    let dirty_set: std::collections::BTreeSet<&str> = dirty.iter().map(String::as_str).collect();
    let marked = read_resolved(repo);
    Ok(remote_changed
        .into_iter()
        .filter(|path| {
            if !local_touched.contains(path) || marked.contains(path) {
                return false;
            }
            let committed = blob_at(repo, local, path);
            let disk = {
                let p = workspace.join(path);
                if p.is_file() {
                    std::fs::read(&p).ok()
                } else if dirty_set.contains(path.as_str()) {
                    None
                } else {
                    committed.clone()
                }
            };
            let theirs = blob_at(repo, remote, path);
            disk != theirs
        })
        .collect())
}

fn blob_at(repo: &Repository, commit: Oid, path: &str) -> Option<Vec<u8>> {
    let commit = repo.find_commit(commit).ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}

fn preview_bytes(path: &str, bytes: Option<&[u8]>) -> ConflictSidePreview {
    let Some(bytes) = bytes else {
        return ConflictSidePreview::missing();
    };
    let text = String::from_utf8_lossy(bytes).into_owned();
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    if path.ends_with(".http") || path.ends_with(".rest") {
        let mut name: Option<String> = None;
        let mut method: Option<String> = None;
        let mut url: Option<String> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("###") {
                if name.is_none() {
                    let n = rest.trim();
                    if !n.is_empty() {
                        name = Some(n.to_string());
                    }
                }
                continue;
            }
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            if let (Some(m), Some(u)) = (parts.next(), parts.next()) {
                let m = m.to_ascii_uppercase();
                if matches!(
                    m.as_str(),
                    "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
                ) {
                    method = Some(m);
                    url = Some(u.trim().to_string());
                    break;
                }
            }
        }
        if method.is_some() || name.is_some() {
            return ConflictSidePreview {
                exists: true,
                kind: Some("http".into()),
                method,
                name: name.or_else(|| Some(stem.to_string())),
                detail: url,
                text: Some(text),
            };
        }
    }
    if path.starts_with("environments/") && path.ends_with(".toml") {
        let name = stem.to_string();
        let vars = text.matches('[').count().saturating_sub(1);
        return ConflictSidePreview {
            exists: true,
            kind: Some("environment".into()),
            method: None,
            name: Some(name),
            detail: Some(if vars == 0 {
                "environment".into()
            } else {
                format!("{vars} entries")
            }),
            text: Some(text),
        };
    }
    if path.starts_with("postman/") && path.ends_with(".json") {
        return ConflictSidePreview {
            exists: true,
            kind: Some("postman".into()),
            method: None,
            name: Some(stem.to_string()),
            detail: Some("Postman mirror".into()),
            text: Some(text),
        };
    }
    ConflictSidePreview {
        exists: true,
        kind: Some("file".into()),
        method: None,
        name: Some(
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
                .to_string(),
        ),
        detail: None,
        text: Some(text),
    }
}

fn conflict_items(
    repo: &Repository,
    workspace: &Path,
    local: Oid,
    remote: Oid,
    files: &[String],
) -> CoreResult<Vec<ConflictItem>> {
    let mut items = Vec::with_capacity(files.len());
    for path in files {
        let ours_disk = {
            let p = workspace.join(path);
            if p.is_file() {
                std::fs::read(&p).ok()
            } else {
                None
            }
        };
        let ours = ours_disk.or_else(|| blob_at(repo, local, path));
        let theirs = blob_at(repo, remote, path);
        items.push(ConflictItem {
            path: path.clone(),
            ours: preview_bytes(path, ours.as_deref()),
            theirs: preview_bytes(path, theirs.as_deref()),
        });
    }
    Ok(items)
}

/// Visual previews for conflicted paths: HEAD/working tree vs the upstream tip.
pub fn conflict_previews(workspace: &Path, files: &[String]) -> CoreResult<Vec<ConflictItem>> {
    let repo = open_required(workspace)?;
    let head = head(&repo)?;
    let Some(local) = head.oid else {
        return Err(CoreError::Conflict(
            "the repository has no commits yet".to_string(),
        ));
    };
    let branch = head.branch.as_deref().unwrap_or("main");
    let Some(remote) = upstream_oid(&repo, branch) else {
        return Err(CoreError::Conflict(
            "no upstream to compare against — fetch first".to_string(),
        ));
    };
    conflict_items(&repo, workspace, local, remote, files)
}

/// Write the Mandalo-resolved file bodies into the working tree. This is not a
/// git merge — the UI already diffed requests/config; Sync picks the result up
/// from disk afterwards.
pub fn apply_conflict_choices(workspace: &Path, decisions: &[ConflictDecision]) -> CoreResult<()> {
    let repo = open_required(workspace)?;
    let head_info = head(&repo)?;
    let Some(local) = head_info.oid else {
        return Err(CoreError::Conflict(
            "the repository has no commits yet".to_string(),
        ));
    };
    let branch = head_info.branch.as_deref().unwrap_or("main");
    let Some(remote) = upstream_oid(&repo, branch) else {
        return Err(CoreError::Conflict(
            "no upstream to compare against — fetch first".to_string(),
        ));
    };
    let mut marked = Vec::new();
    for decision in decisions {
        let path = decision.path.trim_matches('/');
        if path.is_empty() || path.contains("..") {
            return Err(CoreError::PathEscape(format!(
                "refusing conflict path {}",
                decision.path
            )));
        }
        let bytes = if let Some(content) = &decision.content {
            Some(content.as_bytes().to_vec())
        } else {
            match decision.choice {
                ConflictChoice::Ours => {
                    let disk = workspace.join(path);
                    if disk.is_file() {
                        Some(std::fs::read(&disk).map_err(|e| CoreError::io(disk.display(), e))?)
                    } else {
                        blob_at(&repo, local, path)
                    }
                }
                ConflictChoice::Theirs => blob_at(&repo, remote, path),
            }
        };
        let dest = workspace.join(path);
        match bytes {
            Some(content) => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CoreError::io(parent.display(), e))?;
                }
                std::fs::write(&dest, content).map_err(|e| CoreError::io(dest.display(), e))?;
            }
            None => {
                if dest.exists() {
                    std::fs::remove_file(&dest).map_err(|e| CoreError::io(dest.display(), e))?;
                }
            }
        }
        marked.push(path.to_string());
    }
    mark_resolved(&repo, &marked)?;
    Ok(())
}

fn resolved_marker(repo: &Repository) -> PathBuf {
    repo.path().join("mandalo-resolved")
}

fn mark_resolved(repo: &Repository, paths: &[String]) -> CoreResult<()> {
    let path = resolved_marker(repo);
    let mut set: std::collections::BTreeSet<String> = read_resolved(repo);
    set.extend(paths.iter().cloned());
    let body = set.into_iter().collect::<Vec<_>>().join("\n");
    std::fs::write(&path, body).map_err(|e| CoreError::io(path.display(), e))
}

fn read_resolved(repo: &Repository) -> std::collections::BTreeSet<String> {
    std::fs::read_to_string(resolved_marker(repo))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn clear_resolved(repo: &Repository) {
    let _ = std::fs::remove_file(resolved_marker(repo));
}

/// When commits have diverged, build a merge commit that prefers Mandalo's
/// already-written working-tree resolutions for each conflicted path.
fn finish_merge_with_workdir(
    repo: &Repository,
    workspace: &Path,
    local: Oid,
    upstream: Oid,
    branch: &str,
    id: &Identity,
    paths: &[String],
) -> CoreResult<()> {
    let mut resolved = std::collections::BTreeMap::new();
    for path in paths {
        let disk = workspace.join(path);
        if disk.is_file() {
            resolved.insert(
                path.clone(),
                std::fs::read(&disk).map_err(|e| CoreError::io(disk.display(), e))?,
            );
        } else if !disk.exists() {
            resolved.insert(path.clone(), Vec::new());
        }
    }
    finish_merge_with_resolutions(repo, local, upstream, branch, id, &resolved)?;
    clear_resolved(repo);
    Ok(())
}

/// Completes a three-way merge using the paths the user already picked in the UI.
fn finish_merge_with_resolutions(
    repo: &Repository,
    local: Oid,
    upstream: Oid,
    branch: &str,
    id: &Identity,
    resolved: &std::collections::BTreeMap<String, Vec<u8>>,
) -> CoreResult<()> {
    let ours = repo
        .find_commit(local)
        .map_err(|e| map_git("cannot read the local commit", e))?;
    let theirs = repo
        .find_commit(upstream)
        .map_err(|e| map_git("cannot read the remote commit", e))?;
    let mut index = repo
        .merge_commits(&ours, &theirs, None)
        .map_err(|e| map_git("cannot merge the remote changes", e))?;

    if index.has_conflicts() {
        let mut files: Vec<String> = index
            .conflicts()
            .map(|c| {
                c.filter_map(|entry| entry.ok())
                    .filter_map(|c| {
                        c.our
                            .or(c.their)
                            .or(c.ancestor)
                            .map(|e| String::from_utf8_lossy(&e.path).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files.dedup();
        for path in &files {
            let bytes = resolved.get(path).cloned().or_else(|| {
                // Prefer whatever is already on disk from the dialog.
                let disk = repo
                    .workdir()
                    .map(|w| w.join(path))
                    .filter(|p| p.is_file())
                    .and_then(|p| std::fs::read(p).ok());
                disk.or_else(|| blob_at(repo, local, path))
            });
            put_blob_in_index(repo, &mut index, path, bytes.as_deref())?;
        }
    }

    for (path, content) in resolved {
        if content.is_empty() {
            let _ = index.remove_path(Path::new(path));
            put_blob_in_index(repo, &mut index, path, None)?;
        } else {
            put_blob_in_index(repo, &mut index, path, Some(content))?;
        }
    }

    let tree_oid = index
        .write_tree_to(repo)
        .map_err(|e| map_git("cannot write the merged tree", e))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| map_git("cannot read the merged tree", e))?;
    let sig = signature(id)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &format!("Merge remote changes into {branch}"),
        &tree,
        &[&ours, &theirs],
    )
    .map_err(|e| map_git("cannot record the merge", e))?;
    checkout_head(repo, false)?;
    Ok(())
}

fn put_blob_in_index(
    repo: &Repository,
    index: &mut git2::Index,
    path: &str,
    bytes: Option<&[u8]>,
) -> CoreResult<()> {
    let _ = index.conflict_remove(Path::new(path));
    match bytes {
        None | Some([]) => {
            let _ = index.remove_path(Path::new(path));
        }
        Some(content) => {
            // merge_commits returns an in-memory index with no repo backing, so
            // add_frombuffer fails. Store the blob in the ODB, then add the entry.
            let oid = repo
                .blob(content)
                .map_err(|e| map_git("cannot store resolved content", e))?;
            let entry = git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: content.len() as u32,
                id: oid,
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            };
            index
                .add(&entry)
                .map_err(|e| map_git("cannot stage resolved content", e))?;
        }
    }
    Ok(())
}

/// Replays local commits on top of the remote. On conflict the rebase is aborted
/// so the working tree stays usable — we never resolve anyone's edits for them.
fn rebase_onto(
    repo: &Repository,
    upstream: Oid,
    id: &Identity,
    keep_edits: bool,
) -> CoreResult<Option<Vec<String>>> {
    let annotated: AnnotatedCommit = repo
        .find_annotated_commit(upstream)
        .map_err(|e| map_git("cannot read the remote commit", e))?;
    let mut rebase = repo
        .rebase(None, Some(&annotated), None, None)
        .map_err(|e| map_git("cannot start the rebase", e))?;
    let sig = signature(id)?;
    while let Some(operation) = rebase.next() {
        if let Err(e) = operation {
            let files = conflict_paths(repo);
            let _ = rebase.abort();
            if files.is_empty() {
                return Err(map_git("the rebase failed", e));
            }
            return Ok(Some(files));
        }
        let has_conflicts = repo.index().map(|i| i.has_conflicts()).unwrap_or(false);
        if has_conflicts {
            let files = conflict_paths(repo);
            let _ = rebase.abort();
            return Ok(Some(if files.is_empty() {
                vec!["(unknown path)".to_string()]
            } else {
                files
            }));
        }
        match rebase.commit(None, &sig, None) {
            Ok(_) => {}
            // The change is already upstream — nothing to replay, keep going.
            Err(e) if e.code() == ErrorCode::Applied => {}
            Err(e) => {
                let files = conflict_paths(repo);
                let _ = rebase.abort();
                if files.is_empty() {
                    return Err(map_git("the rebase failed", e));
                }
                return Ok(Some(files));
            }
        }
    }
    rebase
        .finish(None)
        .map_err(|e| map_git("cannot finish the rebase", e))?;
    checkout_head(repo, keep_edits)?;
    Ok(None)
}

/// A second chance after a rebase conflict. The rebase replays the user's old
/// commits, so it keeps conflicting even once they have fixed the file; a merge
/// looks at what the two sides *are* now, so a resolved workspace converges.
/// Built entirely in memory: on conflict nothing on disk has moved.
///
/// Any path git cannot merge (requests, config, …) is returned for a full-text
/// visual pick. Clean merges (different files / different lines) never land here.
fn merge_onto(
    repo: &Repository,
    local: Oid,
    upstream: Oid,
    branch: &str,
    id: &Identity,
    keep_edits: bool,
) -> CoreResult<Option<Vec<String>>> {
    let ours = repo
        .find_commit(local)
        .map_err(|e| map_git("cannot read the local commit", e))?;
    let theirs = repo
        .find_commit(upstream)
        .map_err(|e| map_git("cannot read the remote commit", e))?;
    let mut index = repo
        .merge_commits(&ours, &theirs, None)
        .map_err(|e| map_git("cannot merge the remote changes", e))?;
    if index.has_conflicts() {
        let mut files: Vec<String> = index
            .conflicts()
            .map(|c| {
                c.filter_map(|entry| entry.ok())
                    .filter_map(|c| {
                        c.our
                            .or(c.their)
                            .or(c.ancestor)
                            .map(|e| String::from_utf8_lossy(&e.path).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files.dedup();
        return Ok(Some(if files.is_empty() {
            vec!["(unknown path)".to_string()]
        } else {
            files
        }));
    }
    let tree_oid = index
        .write_tree_to(repo)
        .map_err(|e| map_git("cannot write the merged tree", e))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| map_git("cannot read the merged tree", e))?;
    let sig = signature(id)?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &format!("Merge remote changes into {branch}"),
        &tree,
        &[&ours, &theirs],
    )
    .map_err(|e| map_git("cannot record the merge", e))?;
    checkout_head(repo, keep_edits)?;
    Ok(None)
}

fn push(repo: &Repository, branch: &str, auth: &Auth) -> CoreResult<Option<String>> {
    let mut remote = open_transport_remote(repo, auth)?;
    let rejected: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let sink = Rc::clone(&rejected);
    let mut cb = callbacks(auth);
    cb.push_update_reference(move |reference, status| {
        if let Some(message) = status {
            *sink.borrow_mut() = Some(format!("{reference}: {message}"));
        }
        Ok(())
    });
    let mut options = PushOptions::new();
    options.remote_callbacks(cb);
    // No leading `+`: a sync can never force-push and can never rewrite history.
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    remote
        .push(&[refspec.as_str()], Some(&mut options))
        .map_err(|e| map_git("cannot push to the remote", e))?;
    let outcome = rejected.borrow().clone();
    if outcome.is_none() {
        set_upstream(repo, branch);
    }
    Ok(outcome)
}

fn set_upstream(repo: &Repository, branch: &str) {
    if let Ok(mut local) = repo.find_branch(branch, BranchType::Local) {
        if local.upstream().is_err() {
            let _ = local.set_upstream(Some(&format!("{REMOTE}/{branch}")));
        }
    }
}

/// Which of the changed files this commit carries. `only: None` means every one
/// of them; `except` takes files back out. What is left out is not lost — it
/// stays modified in the working tree, ready for the next sync.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncSelection {
    pub only: Option<Vec<String>>,
    pub except: Vec<String>,
}

fn path_matches(entry: &str, path: &str) -> bool {
    let entry = entry.trim_matches('/');
    !entry.is_empty()
        && (path == entry || (path.starts_with(entry) && path[entry.len()..].starts_with('/')))
}

impl SyncSelection {
    pub fn only(paths: &[&str]) -> Self {
        SyncSelection {
            only: Some(paths.iter().map(|p| p.to_string()).collect()),
            except: Vec::new(),
        }
    }

    pub fn except(paths: &[&str]) -> Self {
        SyncSelection {
            only: None,
            except: paths.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn covers(&self, path: &str) -> bool {
        let wanted = match &self.only {
            None => true,
            Some(list) => list.iter().any(|entry| path_matches(entry, path)),
        };
        wanted && !self.except.iter().any(|entry| path_matches(entry, path))
    }

    fn unknown(&self, changed: &[(String, FileChange)]) -> Vec<String> {
        let known = |entry: &String| changed.iter().any(|(path, _)| path_matches(entry, path));
        self.only
            .iter()
            .flatten()
            .chain(self.except.iter())
            .filter(|entry| !known(entry))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileChange {
    New,
    Modified,
    Deleted,
    Renamed,
    TypeChange,
    Conflicted,
}

impl FileChange {
    fn of(bits: git2::Status) -> Self {
        use git2::Status as S;
        if bits.is_conflicted() {
            FileChange::Conflicted
        } else if bits.intersects(S::WT_DELETED | S::INDEX_DELETED) {
            FileChange::Deleted
        } else if bits.intersects(S::WT_NEW | S::INDEX_NEW) {
            FileChange::New
        } else if bits.intersects(S::WT_RENAMED | S::INDEX_RENAMED) {
            FileChange::Renamed
        } else if bits.intersects(S::WT_TYPECHANGE | S::INDEX_TYPECHANGE) {
            FileChange::TypeChange
        } else {
            FileChange::Modified
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub path: String,
    pub change: FileChange,
    pub included: bool,
}

/// What the sync would actually do. `CommitAndPush` is the only one that puts
/// anything on a remote; the UI says which one before the user agrees to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncAction {
    Nothing,
    Commit,
    Push,
    CommitAndPush,
    Pull,
    BranchAndPush,
}

/// What a sync is about to send, and where. `ahead`/`behind` are as of the last
/// fetch — planning is deliberately offline, so a preview never touches the
/// network on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub action: SyncAction,
    pub files: Vec<PlannedFile>,
    pub included: usize,
    pub excluded: usize,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub target_branch: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub conflicted: Vec<String>,
    /// Card previews for `conflicted` — empty when there is nothing to pick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_items: Vec<ConflictItem>,
    pub findings: Vec<scan::Finding>,
    pub blocked: bool,
    pub identity: Identity,
    pub token: String,
    /// Set when `[share] format = "postman"` so the UI can say the mirror was refreshed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_dir: Option<String>,
}

impl SyncPlan {
    fn selected(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.included)
            .map(|f| f.path.clone())
            .collect()
    }
}

fn blob_id(workspace: &Path, path: &str) -> String {
    git2::Oid::hash_file(git2::ObjectType::Blob, workspace.join(path))
        .map(|oid| oid.to_string())
        .unwrap_or_else(|_| "gone".to_string())
}

/// Preview what a sync would do. When `[share] format = "postman"`, regenerates
/// the Postman mirror first so those files appear in the plan.
pub fn plan_sync(
    workspace: &Path,
    selection: &SyncSelection,
    branch_name: Option<&str>,
) -> CoreResult<SyncPlan> {
    let share = crate::share::materialize(workspace)?;
    let repo = open_required(workspace)?;
    let id = identity(&repo);
    let head_info = head(&repo)?;
    let remote = remote_url(&repo);
    let changed = changed_files(&repo)?;

    let missing = selection.unknown(&changed);
    if !missing.is_empty() {
        return Err(CoreError::NotFound(format!(
            "cannot sync: nothing changed at {} — check the path",
            missing.join(", ")
        )));
    }

    let files: Vec<PlannedFile> = changed
        .into_iter()
        .map(|(path, change)| PlannedFile {
            included: selection.covers(&path),
            path,
            change,
        })
        .collect();
    let included = files.iter().filter(|f| f.included).count();
    let excluded = files.len() - included;
    let selected: Vec<String> = files
        .iter()
        .filter(|f| f.included)
        .map(|f| f.path.clone())
        .collect();

    let (mut ahead, mut behind) = (0, 0);
    let mut conflicted = conflict_paths(&repo);
    if let (Some(branch), Some(local)) = (head_info.branch.as_deref(), head_info.oid) {
        if let Some(upstream) = upstream_oid(&repo, branch) {
            let counts = repo
                .graph_ahead_behind(local, upstream)
                .map_err(|e| map_git("cannot compare with the remote", e))?;
            ahead = counts.0;
            behind = counts.1;
            if behind > 0 {
                let dirty: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
                let pending =
                    pending_conflict_paths(&repo, workspace, local, upstream, ahead, &dirty)?;
                conflicted.extend(pending);
                conflicted.sort();
                conflicted.dedup();
            }
        }
    }

    let has_remote = repo.find_remote(REMOTE).is_ok();
    let action = match branch_name {
        Some(_) => SyncAction::BranchAndPush,
        None if included > 0 && has_remote => SyncAction::CommitAndPush,
        None if included > 0 => SyncAction::Commit,
        None if ahead > 0 && has_remote => SyncAction::Push,
        None if behind > 0 => SyncAction::Pull,
        None => SyncAction::Nothing,
    };

    let findings = scan_selected(workspace, &selected)?;
    let mut parts = vec![
        format!("{action:?}"),
        branch_name.unwrap_or_default().to_string(),
        head_info.branch.clone().unwrap_or_default(),
        remote.clone().unwrap_or_default(),
    ];
    parts.extend(
        selected
            .iter()
            .map(|path| format!("{path}\u{1f}{}", blob_id(workspace, path))),
    );
    let token = review::token("sync", &parts)?;

    let conflict_items = if conflicted.is_empty() {
        Vec::new()
    } else if let (Some(local), Some(branch)) = (head_info.oid, head_info.branch.as_deref()) {
        if let Some(remote_oid) = upstream_oid(&repo, branch) {
            conflict_items(&repo, workspace, local, remote_oid, &conflicted)?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    Ok(SyncPlan {
        action,
        files,
        included,
        excluded,
        remote,
        branch: head_info.branch,
        target_branch: branch_name.map(str::to_string),
        ahead,
        behind,
        conflicted,
        conflict_items,
        blocked: !findings.is_empty(),
        findings,
        identity: id,
        token,
        share_dir: if share.dir.is_empty() {
            None
        } else {
            Some(share.dir)
        },
    })
}

fn check_token(plan: &SyncPlan, token: &str) -> CoreResult<()> {
    if plan.token != token {
        return Err(review::stale("sync"));
    }
    Ok(())
}

/// Pull remote first (when behind), then commit local work, then push.
/// `force` skips the credential scanner and nothing else — it never forces a push.
pub fn run_sync(
    workspace: &Path,
    selection: &SyncSelection,
    token: &str,
    message: &str,
    auth: &Auth,
    force: bool,
) -> CoreResult<SyncOutcome> {
    let plan = plan_sync(workspace, selection, None)?;
    check_token(&plan, token)?;
    let files = plan.selected();
    let keep_edits = plan.excluded > 0;

    let repo = open_required(workspace)?;
    let id = identity(&repo);
    let head_info = head(&repo)?;
    if head_info.detached {
        return Ok(SyncOutcome::Rejected {
            reason: "HEAD is detached — check out a branch before syncing".to_string(),
        });
    }
    let Some(branch) = head_info.branch.clone() else {
        return Ok(SyncOutcome::Rejected {
            reason: "the repository has no current branch".to_string(),
        });
    };
    if !repo.index().map(|i| i.has_conflicts()).unwrap_or(false) {
        guard_secrets(workspace, &files, force)?;
    }

    if repo.find_remote(REMOTE).is_err() {
        let committed = commit_selected(&repo, workspace, &files, message, &id, &head_info)?;
        return Ok(match committed {
            Some(oid) => SyncOutcome::Committed {
                sha: oid.to_string(),
                identity: id,
            },
            None => SyncOutcome::NothingToDo,
        });
    }

    fetch(&repo, &branch, auth)?;

    let remote_oid = upstream_oid(&repo, &branch);
    let head_oid = head(&repo)?.oid;

    // First push: no upstream tip yet — commit then push.
    let Some(remote_oid) = remote_oid else {
        let committed = commit_selected(&repo, workspace, &files, message, &id, &head(&repo)?)?;
        let tip = committed.or(head_oid).ok_or_else(|| {
            CoreError::Conflict(
                "the branch has no commits yet — there is nothing to sync".to_string(),
            )
        })?;
        return match push(&repo, &branch, auth)? {
            Some(reason) => Ok(SyncOutcome::Rejected { reason }),
            None => Ok(SyncOutcome::Pushed {
                sha: tip.to_string(),
                ahead: 1,
                identity: id,
            }),
        };
    };

    let mut local = match head_oid {
        Some(oid) => oid,
        None => {
            let committed = commit_selected(&repo, workspace, &files, message, &id, &head(&repo)?)?;
            committed.ok_or_else(|| {
                CoreError::Conflict(
                    "the branch has no commits yet — there is nothing to sync".to_string(),
                )
            })?
        }
    };

    let (ahead, behind) = repo
        .graph_ahead_behind(local, remote_oid)
        .map_err(|e| map_git("cannot compare with the remote", e))?;

    // Integrate remote first. Overlaps are Mandalo diffs (Resolve), not git pulls.
    if behind > 0 {
        let dirty_paths: Vec<String> = changed_files(&repo)?.into_iter().map(|(p, _)| p).collect();
        let pending =
            pending_conflict_paths(&repo, workspace, local, remote_oid, ahead, &dirty_paths)?;
        if !pending.is_empty() {
            let items = conflict_items(&repo, workspace, local, remote_oid, &pending)?;
            return Ok(SyncOutcome::Conflicted {
                files: pending,
                items,
            });
        }

        let dirty = !dirty_paths.is_empty();
        let marked = read_resolved(&repo);
        let preserve = keep_edits || dirty || !marked.is_empty();

        if ahead == 0 {
            fast_forward(&repo, &branch, remote_oid, preserve)?;
            local = remote_oid;
        } else {
            // libgit2 refuses rebase with a dirty workdir — merge instead.
            let needs_merge = dirty || rebase_onto(&repo, remote_oid, &id, preserve)?.is_some();
            if needs_merge {
                if let Some(files) = merge_onto(&repo, local, remote_oid, &branch, &id, preserve)? {
                    if files.iter().all(|path| marked.contains(path)) {
                        finish_merge_with_workdir(
                            &repo, workspace, local, remote_oid, &branch, &id, &files,
                        )?;
                    } else {
                        let items = conflict_items(&repo, workspace, local, remote_oid, &files)?;
                        return Ok(SyncOutcome::Conflicted { files, items });
                    }
                }
            }
            local = head(&repo)?
                .oid
                .ok_or_else(|| CoreError::Io("the branch lost its tip during the merge".into()))?;
        }
    }

    let head_now = head(&repo)?;
    let committed = commit_selected(&repo, workspace, &files, message, &id, &head_now)?;
    let tip = committed.unwrap_or_else(|| head(&repo).ok().and_then(|h| h.oid).unwrap_or(local));

    let remote_now = upstream_oid(&repo, &branch).unwrap_or(remote_oid);
    let (ahead, behind_left) = repo
        .graph_ahead_behind(tip, remote_now)
        .map_err(|e| map_git("cannot compare with the remote", e))?;
    if ahead == 0 {
        return Ok(if behind > 0 || behind_left > 0 {
            SyncOutcome::Pulled {
                sha: tip.to_string(),
                behind: behind.max(behind_left),
            }
        } else if committed.is_some() {
            SyncOutcome::Committed {
                sha: tip.to_string(),
                identity: id,
            }
        } else {
            SyncOutcome::NothingToDo
        });
    }
    match push(&repo, &branch, auth)? {
        Some(reason) => Ok(SyncOutcome::Rejected { reason }),
        None => {
            clear_resolved(&repo);
            Ok(SyncOutcome::Pushed {
                sha: tip.to_string(),
                ahead,
                identity: id,
            })
        }
    }
}

/// Commits the reviewed files onto a NEW branch and pushes it. Existing branches
/// are never moved and never deleted — that is the whole "open a PR" flow.
pub fn run_branch_push(
    workspace: &Path,
    selection: &SyncSelection,
    branch: &str,
    token: &str,
    message: &str,
    auth: &Auth,
    force: bool,
) -> CoreResult<PushedBranch> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(CoreError::InvalidName("a branch needs a name".to_string()));
    }
    if !git2::Branch::name_is_valid(branch).unwrap_or(false) {
        return Err(CoreError::InvalidName(format!(
            "{branch} is not a valid branch name"
        )));
    }
    let plan = plan_sync(workspace, selection, Some(branch))?;
    check_token(&plan, token)?;
    let files = plan.selected();
    let repo = open_required(workspace)?;
    let id = identity(&repo);
    let head_info = head(&repo)?;
    if head_info.unborn || head_info.oid.is_none() {
        return Err(CoreError::Conflict(
            "the repository has no commits yet — sync once before opening a branch".to_string(),
        ));
    }
    if repo.find_branch(branch, BranchType::Local).is_ok() {
        return Err(CoreError::Conflict(format!(
            "the branch {branch} already exists"
        )));
    }
    if !repo.index().map(|i| i.has_conflicts()).unwrap_or(false) {
        guard_secrets(workspace, &files, force)?;
    }

    let base = head_info
        .oid
        .ok_or_else(|| CoreError::Conflict("the repository has no commits yet".to_string()))?;
    let tip = repo
        .find_commit(base)
        .map_err(|e| map_git("cannot read the last commit", e))?;
    repo.branch(branch, &tip, false)
        .map_err(|e| map_git("cannot create the branch", e))?;
    // The new branch starts at HEAD, so the work tree already matches it —
    // moving HEAD alone keeps every uncommitted edit exactly where it is.
    repo.set_head(&format!("refs/heads/{branch}"))
        .map_err(|e| map_git("cannot switch to the new branch", e))?;

    let on_branch = head(&repo)?;
    let committed = commit_selected(&repo, workspace, &files, message, &id, &on_branch)?;
    let sha = committed.unwrap_or(base);

    if repo.find_remote(REMOTE).is_err() {
        return Err(CoreError::NotFound(
            "the workspace has no remote to push the branch to".to_string(),
        ));
    }
    if let Some(reason) = push(&repo, branch, auth)? {
        return Err(CoreError::Conflict(format!(
            "the remote refused the branch — {reason}"
        )));
    }
    let url = remote_url(&repo).and_then(|u| github_compare_url(&u, branch));
    Ok(PushedBranch {
        branch: branch.to_string(),
        sha: sha.to_string(),
        url,
        identity: id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_credentials_from_a_remote_url() {
        assert_eq!(
            sanitize_url("https://ghp_deadbeefdeadbeef@github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            sanitize_url("https://user:tok@github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            sanitize_url("https://github.com/o/r.git"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            sanitize_url("git@github.com:o/r.git"),
            "git@github.com:o/r.git"
        );
    }

    #[test]
    fn recognises_ssh_remotes() {
        assert!(is_ssh_url("git@github.com:o/r.git"));
        assert!(is_ssh_url("ssh://git@github.com/o/r.git"));
        assert!(!is_ssh_url("https://github.com/o/r.git"));
        assert!(!is_ssh_url("/tmp/local/repo"));
    }

    #[test]
    fn token_auth_rewrites_github_ssh_to_https() {
        assert!(matches!(
            Auth::for_url("git@github.com:o/r.git", Some("ghp_deadbeef")),
            Auth::Token(_)
        ));
        assert_eq!(
            transport_url(
                "git@github.com:o/r.git",
                &Auth::Token("ghp_deadbeef".into())
            ),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            transport_url(
                "ssh://git@github.com/o/r.git",
                &Auth::Token("ghp_deadbeef".into())
            ),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            Auth::for_url("git@github.com:o/r.git", None),
            Auth::SshAgent
        );
    }

    #[test]
    fn a_token_is_registered_with_the_redactor_the_moment_it_is_accepted() {
        let auth = Auth::token("ghp_redactorfixture0123456789abcd");
        assert_eq!(
            auth,
            Auth::Token("ghp_redactorfixture0123456789abcd".to_string())
        );
        assert_eq!(
            redact::scrub("remote said ghp_redactorfixture0123456789abcd"),
            "remote said [redacted:git.token]"
        );
    }
}
