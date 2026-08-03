use crate::error::{CoreError, CoreResult};
use crate::{git, redact, scan};
use git2::{
    AnnotatedCommit, BranchType, Cred, CredentialType, ErrorClass, ErrorCode, FetchOptions,
    IndexAddOption, Oid, PushOptions, RemoteCallbacks, Repository, RepositoryInitOptions,
    Signature,
};
use serde::Serialize;
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
    /// it. They edit the listed files to what they want and sync again.
    Conflicted { files: Vec<String> },
    /// The remote or the repository state refused the operation.
    Rejected { reason: String },
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
/// caller resolves it (keychain, CI variable) and hands it in.
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

    /// Picks the mechanism a URL implies: an explicit token wins, an `ssh://` or
    /// `git@host:path` remote falls back to the system SSH agent.
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
                // GitHub's documented basic scheme: the token IS the username.
                Cred::userpass_plaintext(token, "x-oauth-basic")
            }
            Auth::Token(_) => Err(git2::Error::from_str(
                "the remote asked for a credential this token cannot satisfy",
            )),
            Auth::SshAgent | Auth::None if allowed.contains(CredentialType::SSH_KEY) => {
                Cred::ssh_key_from_agent(user)
            }
            Auth::SshAgent => Err(git2::Error::from_str(
                "the remote asked for a password, but only the SSH agent was offered",
            )),
            Auth::None => Err(git2::Error::from_str(
                "the remote requires authentication — pass a token or use an ssh remote",
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
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(auth));
    git2::build::RepoBuilder::new()
        .fetch_options(fetch)
        .clone(url, dest)
        .map_err(|e| map_git(&format!("cannot clone {}", sanitize_url(url)), e))?;
    Ok(())
}

/// The files a commit would carry, as absolute paths inside the workspace.
fn pending_files(repo: &Repository, workspace: &Path) -> CoreResult<Vec<PathBuf>> {
    let statuses = repo
        .statuses(Some(&mut status_options()))
        .map_err(|e| map_git("cannot read the workspace status", e))?;
    let mut files = Vec::new();
    for entry in statuses.iter() {
        let Ok(path) = entry.path() else { continue };
        let full = workspace.join(path);
        if full.is_file() {
            files.push(full);
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn guard_secrets(repo: &Repository, workspace: &Path, force: bool) -> CoreResult<()> {
    if force {
        return Ok(());
    }
    let findings = scan::scan_files(&pending_files(repo, workspace)?)?;
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

fn stage_all(repo: &Repository) -> CoreResult<()> {
    let mut index = repo
        .index()
        .map_err(|e| map_git("cannot open the index", e))?;
    // Pathspecs are relative to the work tree, so nothing outside it can be staged.
    index
        .update_all(["*"].iter(), None)
        .map_err(|e| map_git("cannot stage changes", e))?;
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(|e| map_git("cannot stage changes", e))?;
    index
        .write()
        .map_err(|e| map_git("cannot write the index", e))?;
    Ok(())
}

fn commit_all(
    repo: &Repository,
    message: &str,
    id: &Identity,
    head_info: &Head,
) -> CoreResult<Option<Oid>> {
    stage_all(repo)?;
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

fn fetch(repo: &Repository, branch: &str, auth: &Auth) -> CoreResult<()> {
    let mut remote = repo
        .find_remote(REMOTE)
        .map_err(|e| map_git("cannot read the remote", e))?;
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

/// Brings the working tree and the index back in step with HEAD. Everything the
/// user had is already committed by the time this runs, so there is nothing to
/// lose; untracked and ignored files are left alone.
fn checkout_head(repo: &Repository) -> CoreResult<()> {
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout
        .force()
        .remove_untracked(false)
        .remove_ignored(false);
    repo.checkout_head(Some(&mut checkout))
        .map_err(|e| map_git("cannot update the working tree", e))
}

fn fast_forward(repo: &Repository, branch: &str, target: Oid) -> CoreResult<()> {
    let name = format!("refs/heads/{branch}");
    repo.reference(&name, target, true, "mandalo sync: fast-forward")
        .map_err(|e| map_git("cannot move the branch", e))?;
    repo.set_head(&name)
        .map_err(|e| map_git("cannot move HEAD", e))?;
    checkout_head(repo)
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

/// Replays local commits on top of the remote. On conflict the rebase is aborted
/// so the working tree stays usable — we never resolve anyone's edits for them.
fn rebase_onto(repo: &Repository, upstream: Oid, id: &Identity) -> CoreResult<Option<Vec<String>>> {
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
    checkout_head(repo)?;
    Ok(None)
}

/// A second chance after a rebase conflict. The rebase replays the user's old
/// commits, so it keeps conflicting even once they have fixed the file; a merge
/// looks at what the two sides *are* now, so a resolved workspace converges.
/// Built entirely in memory: on conflict nothing on disk has moved.
fn merge_onto(
    repo: &Repository,
    local: Oid,
    upstream: Oid,
    branch: &str,
    id: &Identity,
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
    checkout_head(repo)?;
    Ok(None)
}

fn push(repo: &Repository, branch: &str, auth: &Auth) -> CoreResult<Option<String>> {
    let mut remote = repo
        .find_remote(REMOTE)
        .map_err(|e| map_git("cannot read the remote", e))?;
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

/// Stage everything, commit, rebase on top of the remote, push. `force` skips the
/// credential scanner and nothing else — it never forces a push.
pub fn sync(workspace: &Path, message: &str, auth: &Auth, force: bool) -> CoreResult<SyncOutcome> {
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
        guard_secrets(&repo, workspace, force)?;
    }

    let committed = commit_all(&repo, message, &id, &head_info)?;
    let local = match committed {
        Some(oid) => Some(oid),
        None => head_info.oid,
    };

    if repo.find_remote(REMOTE).is_err() {
        return Ok(match committed {
            Some(oid) => SyncOutcome::Committed {
                sha: oid.to_string(),
                identity: id,
            },
            None => SyncOutcome::NothingToDo,
        });
    }
    let Some(local) = local else {
        return Ok(SyncOutcome::Rejected {
            reason: "the branch has no commits yet — there is nothing to sync".to_string(),
        });
    };

    fetch(&repo, &branch, auth)?;

    let Some(remote_oid) = upstream_oid(&repo, &branch) else {
        return match push(&repo, &branch, auth)? {
            Some(reason) => Ok(SyncOutcome::Rejected { reason }),
            None => Ok(SyncOutcome::Pushed {
                sha: local.to_string(),
                ahead: 1,
                identity: id,
            }),
        };
    };

    let (ahead, behind) = repo
        .graph_ahead_behind(local, remote_oid)
        .map_err(|e| map_git("cannot compare with the remote", e))?;

    if behind > 0 {
        if ahead == 0 {
            fast_forward(&repo, &branch, remote_oid)?;
            return Ok(SyncOutcome::Pulled {
                sha: remote_oid.to_string(),
                behind,
            });
        }
        if rebase_onto(&repo, remote_oid, &id)?.is_some() {
            if let Some(files) = merge_onto(&repo, local, remote_oid, &branch, &id)? {
                return Ok(SyncOutcome::Conflicted { files });
            }
        }
    }

    let tip = head(&repo)?
        .oid
        .ok_or_else(|| CoreError::Io("the branch lost its tip during the rebase".to_string()))?;
    let (ahead, _) = repo
        .graph_ahead_behind(tip, remote_oid)
        .map_err(|e| map_git("cannot compare with the remote", e))?;
    if ahead == 0 {
        return Ok(if behind > 0 {
            SyncOutcome::Pulled {
                sha: tip.to_string(),
                behind,
            }
        } else {
            SyncOutcome::NothingToDo
        });
    }
    match push(&repo, &branch, auth)? {
        Some(reason) => Ok(SyncOutcome::Rejected { reason }),
        None => Ok(SyncOutcome::Pushed {
            sha: tip.to_string(),
            ahead,
            identity: id,
        }),
    }
}

/// Commits the working tree onto a NEW branch and pushes it. Existing branches
/// are never moved and never deleted — that is the whole "open a PR" flow.
pub fn create_branch_and_push(
    workspace: &Path,
    branch: &str,
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
        guard_secrets(&repo, workspace, force)?;
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
    let committed = commit_all(&repo, message, &id, &on_branch)?;
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
