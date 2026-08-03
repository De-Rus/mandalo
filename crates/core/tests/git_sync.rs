use mandalo_core::git_sync::{
    self, Auth, PushedBranch, SyncOutcome, SyncStatus, FALLBACK_EMAIL, FALLBACK_NAME,
};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// libgit2 reads the ambient `~/.gitconfig`, which would make the identity
/// fallback test depend on the developer's machine. Point every non-local config
/// level at an empty directory, once, before any repository is opened.
fn isolate() {
    static ONCE: OnceLock<tempfile::TempDir> = OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("config sandbox");
        for level in [
            git2::ConfigLevel::System,
            git2::ConfigLevel::Global,
            git2::ConfigLevel::XDG,
            git2::ConfigLevel::ProgramData,
        ] {
            // SAFETY: this runs once, before any repository in this test binary
            // is opened, so no other thread is reading libgit2's config state.
            unsafe {
                let _ = git2::opts::set_search_path(level, dir.path());
            }
        }
        dir
    });
}

struct Sandbox {
    _root: tempfile::TempDir,
    bare: PathBuf,
    root: PathBuf,
}

/// A bare repository standing in for GitHub, with its HEAD on `main` the way a
/// freshly created remote has it.
fn sandbox() -> Sandbox {
    isolate();
    let root = tempfile::tempdir().expect("sandbox");
    let bare = root.path().join("remote.git");
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true).initial_head("main");
    git2::Repository::init_opts(&bare, &options).expect("bare remote");
    Sandbox {
        root: root.path().to_path_buf(),
        _root: root,
        bare,
    }
}

impl Sandbox {
    fn remote_url(&self) -> String {
        self.bare.to_string_lossy().to_string()
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(&path).expect("workspace dir");
        path
    }

    /// A workspace wired to the bare remote, with a real identity configured.
    fn workspace(&self, name: &str) -> PathBuf {
        let path = self.dir(name);
        git_sync::init(&path, Some(&self.remote_url())).expect("init");
        set_identity(&path, name);
        path
    }

    fn clone(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        git_sync::clone(&self.remote_url(), &path, &Auth::None).expect("clone");
        set_identity(&path, name);
        path
    }
}

fn set_identity(workspace: &Path, who: &str) {
    let repo = git2::Repository::open(workspace).expect("open");
    let mut config = repo.config().expect("config");
    config.set_str("user.name", who).expect("name");
    config
        .set_str("user.email", &format!("{who}@example.test"))
        .expect("email");
}

fn write(workspace: &Path, name: &str, body: &str) {
    let path = workspace.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, body).expect("write");
}

fn read(workspace: &Path, name: &str) -> String {
    std::fs::read_to_string(workspace.join(name)).unwrap_or_default()
}

fn sync(workspace: &Path, message: &str) -> SyncOutcome {
    git_sync::sync(workspace, message, &Auth::None, false).expect("sync")
}

fn status(workspace: &Path) -> SyncStatus {
    git_sync::status(workspace).expect("status")
}

#[test]
fn status_on_a_plain_directory_says_it_is_not_a_repository() {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let out = status(dir.path());
    assert!(!out.is_repo);
    assert_eq!(out.branch, None);
    assert_eq!(out.remote_url, None);
    assert_eq!(out.dirty_total, 0);
    assert!(out.identity.is_none());
}

#[test]
fn status_on_a_missing_directory_fails_loud() {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let err = git_sync::status(&dir.path().join("ghost")).unwrap_err();
    assert_eq!(err.code(), "E_NOT_FOUND");
}

#[test]
fn init_writes_the_gitignore_and_leaves_a_committable_repository() {
    let box_ = sandbox();
    let ws = box_.dir("fresh");
    write(&ws, "collections/api/get.toml", "name = \"get\"\n");
    git_sync::init(&ws, None).expect("init");

    let ignore = read(&ws, ".gitignore");
    assert!(ignore.contains(".mandalo/"), "{ignore}");
    assert!(ignore.contains("*.local.toml"), "{ignore}");

    let out = status(&ws);
    assert!(out.is_repo);
    assert_eq!(out.branch.as_deref(), Some("main"));
    assert_eq!(out.remote_url, None);
    assert!(out.untracked >= 2, "{out:?}");
    assert!(out.dirty_files.iter().any(|p| p == ".gitignore"), "{out:?}");

    set_identity(&ws, "solo");
    match sync(&ws, "first") {
        SyncOutcome::Committed { sha, identity } => {
            assert_eq!(sha.len(), 40);
            assert!(!identity.is_fallback);
        }
        other => panic!("expected Committed without a remote, got {other:?}"),
    }
    assert_eq!(status(&ws).untracked, 0);
}

#[test]
fn init_records_the_remote_and_refuses_to_silently_repoint_it() {
    let box_ = sandbox();
    let ws = box_.dir("wired");
    git_sync::init(&ws, Some(&box_.remote_url())).expect("init");
    assert_eq!(
        status(&ws).remote_url.as_deref(),
        Some(box_.remote_url().as_str())
    );

    git_sync::init(&ws, Some(&box_.remote_url())).expect("same remote is a no-op");
    let err = git_sync::init(&ws, Some("https://github.com/other/repo.git")).unwrap_err();
    assert_eq!(err.code(), "E_CONFLICT");
}

#[test]
fn a_clean_sync_commits_and_pushes_and_a_second_clone_pulls_it() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(
        &alice,
        "collections/api/list.toml",
        "name = \"list orders\"\n",
    );

    let first = sync(&alice, "add list orders");
    let pushed_sha = match &first {
        SyncOutcome::Pushed { sha, ahead, .. } => {
            assert_eq!(*ahead, 1);
            sha.clone()
        }
        other => panic!("expected Pushed, got {other:?}"),
    };

    let after = status(&alice);
    assert_eq!(after.ahead, 0);
    assert_eq!(after.behind, 0);
    assert_eq!(after.dirty_total, 0);
    assert_eq!(
        after.remote_url.as_deref(),
        Some(box_.remote_url().as_str())
    );

    let bob = box_.clone("bob");
    assert_eq!(
        read(&bob, "collections/api/list.toml"),
        "name = \"list orders\"\n"
    );

    write(
        &alice,
        "collections/api/list.toml",
        "name = \"list all orders\"\n",
    );
    let second = sync(&alice, "rename");
    match &second {
        SyncOutcome::Pushed { sha, .. } => assert_ne!(sha, &pushed_sha),
        other => panic!("expected Pushed, got {other:?}"),
    }

    assert_eq!(status(&bob).behind, 0, "bob has not fetched yet");
    match sync(&bob, "nothing of mine") {
        SyncOutcome::Pulled { behind, .. } => assert_eq!(behind, 1),
        other => panic!("expected Pulled, got {other:?}"),
    }
    assert_eq!(
        read(&bob, "collections/api/list.toml"),
        "name = \"list all orders\"\n"
    );
}

#[test]
fn nothing_to_do_when_the_workspace_is_clean_and_in_step() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/one.toml", "name = \"one\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));
    assert_eq!(sync(&alice, "again"), SyncOutcome::NothingToDo);
    assert_eq!(sync(&alice, "and again"), SyncOutcome::NothingToDo);
}

#[test]
fn divergent_edits_to_different_files_rebase_cleanly() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/base.toml", "name = \"base\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));

    let bob = box_.clone("bob");
    write(&alice, "collections/api/alice.toml", "name = \"alice\"\n");
    write(&bob, "collections/api/bob.toml", "name = \"bob\"\n");

    assert!(matches!(
        sync(&alice, "alice adds hers"),
        SyncOutcome::Pushed { .. }
    ));
    match sync(&bob, "bob adds his") {
        SyncOutcome::Pushed { ahead, .. } => assert_eq!(ahead, 1),
        other => panic!("expected a clean rebase then Pushed, got {other:?}"),
    }

    assert_eq!(
        read(&bob, "collections/api/alice.toml"),
        "name = \"alice\"\n"
    );
    assert_eq!(read(&bob, "collections/api/bob.toml"), "name = \"bob\"\n");

    match sync(&alice, "catch up") {
        SyncOutcome::Pulled { .. } => {}
        other => panic!("expected Pulled, got {other:?}"),
    }
    assert_eq!(read(&alice, "collections/api/bob.toml"), "name = \"bob\"\n");
    assert_eq!(
        git2::Repository::open(&bob).unwrap().state(),
        git2::RepositoryState::Clean
    );
}

#[test]
fn divergent_edits_to_the_same_file_conflict_and_leave_a_usable_tree() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/shared.toml", "name = \"base\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));

    let bob = box_.clone("bob");
    write(
        &alice,
        "collections/api/shared.toml",
        "name = \"alice wins\"\n",
    );
    write(&bob, "collections/api/shared.toml", "name = \"bob wins\"\n");
    assert!(matches!(sync(&alice, "alice"), SyncOutcome::Pushed { .. }));

    match sync(&bob, "bob") {
        SyncOutcome::Conflicted { files } => {
            assert_eq!(files, vec!["collections/api/shared.toml".to_string()]);
        }
        other => panic!("expected Conflicted, got {other:?}"),
    }

    let repo = git2::Repository::open(&bob).unwrap();
    assert_eq!(
        repo.state(),
        git2::RepositoryState::Clean,
        "the rebase must be aborted, not left half-applied"
    );
    assert!(!repo.index().unwrap().has_conflicts());
    assert_eq!(
        read(&bob, "collections/api/shared.toml"),
        "name = \"bob wins\"\n",
        "bob's own commit must survive the abort"
    );

    let out = status(&bob);
    assert!(out.conflicted.is_empty());
    assert_eq!(out.ahead, 1);
    assert_eq!(out.behind, 1);

    write(
        &bob,
        "collections/api/shared.toml",
        "name = \"alice wins\"\n",
    );
    assert!(
        matches!(sync(&bob, "take alice's"), SyncOutcome::Pushed { .. }),
        "a resolved workspace must sync again"
    );
}

#[test]
fn the_scanner_blocks_a_commit_that_carries_a_token_and_force_overrides_it() {
    let box_ = sandbox();
    let ws = box_.workspace("leaky");
    write(&ws, "collections/api/base.toml", "name = \"base\"\n");
    assert!(matches!(sync(&ws, "seed"), SyncOutcome::Pushed { .. }));

    write(
        &ws,
        "environments/prod.toml",
        "token = \"ghp_16C7e42F292c6912E7710c838347Ae178B4a\"\n",
    );
    let err = git_sync::sync(&ws, "oops", &Auth::None, false).unwrap_err();
    assert_eq!(err.code(), "E_SECRET");
    assert!(err.to_string().contains("github-token"), "{err}");
    assert!(err.to_string().contains("environments/prod.toml"), "{err}");
    assert!(
        !err.to_string()
            .contains("ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
        "the error must never echo the credential back"
    );
    assert_eq!(status(&ws).untracked, 1, "nothing was staged or committed");

    let forced = git_sync::sync(&ws, "I know what I am doing", &Auth::None, true).expect("forced");
    assert!(matches!(forced, SyncOutcome::Pushed { .. }), "{forced:?}");
}

#[test]
fn a_gitignored_secret_file_never_reaches_the_scanner_or_the_commit() {
    let box_ = sandbox();
    let ws = box_.workspace("safe");
    write(
        &ws,
        "environments/prod.local.toml",
        "token = \"ghp_16C7e42F292c6912E7710c838347Ae178B4a\"\n",
    );
    write(&ws, "collections/api/base.toml", "name = \"base\"\n");
    let out =
        git_sync::sync(&ws, "seed", &Auth::None, false).expect("the ignored file is invisible");
    assert!(matches!(out, SyncOutcome::Pushed { .. }), "{out:?}");

    let bob = box_.clone("bob");
    assert!(!bob.join("environments/prod.local.toml").exists());
}

#[test]
fn a_repository_without_an_identity_commits_as_the_labelled_fallback() {
    let box_ = sandbox();
    let ws = box_.dir("anon");
    git_sync::init(&ws, None).expect("init");
    write(&ws, "collections/api/one.toml", "name = \"one\"\n");

    let reported = status(&ws).identity.expect("identity");
    assert!(reported.is_fallback);
    assert_eq!(reported.name, FALLBACK_NAME);
    assert_eq!(reported.email, FALLBACK_EMAIL);

    match sync(&ws, "first") {
        SyncOutcome::Committed { identity, .. } => assert!(identity.is_fallback),
        other => panic!("expected Committed, got {other:?}"),
    }
    let repo = git2::Repository::open(&ws).unwrap();
    let author = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(author.author().email(), Ok(FALLBACK_EMAIL));
}

#[test]
fn a_detached_head_is_rejected_not_swallowed() {
    let box_ = sandbox();
    let ws = box_.workspace("detached");
    write(&ws, "collections/api/one.toml", "name = \"one\"\n");
    assert!(matches!(sync(&ws, "seed"), SyncOutcome::Pushed { .. }));

    let repo = git2::Repository::open(&ws).unwrap();
    let oid = repo.head().unwrap().target().unwrap();
    repo.set_head_detached(oid).unwrap();

    let out = status(&ws);
    assert!(out.detached);
    assert_eq!(out.branch, None);
    match sync(&ws, "nope") {
        SyncOutcome::Rejected { reason } => assert!(reason.contains("detached"), "{reason}"),
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[test]
fn a_bad_credential_never_leaks_the_token_into_the_error() {
    isolate();
    let secret = "ghp_leakcanary00112233445566778899aa";
    let auth = Auth::token(secret);
    let dir = tempfile::tempdir().unwrap();
    let err = git_sync::clone(
        "https://github.com/mandalo-does-not-exist/nope.git",
        &dir.path().join("dest"),
        &auth,
    )
    .unwrap_err();
    let rendered = format!("{err} [{}]", err.code());
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(!err.message().contains(secret), "{}", err.message());
}

#[test]
fn clone_refuses_a_destination_that_already_holds_files() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/one.toml", "name = \"one\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));

    let dest = box_.dir("occupied");
    write(&dest, "keep.txt", "mine\n");
    let err = git_sync::clone(&box_.remote_url(), &dest, &Auth::None).unwrap_err();
    assert_eq!(err.code(), "E_CONFLICT");
    assert_eq!(read(&dest, "keep.txt"), "mine\n");
}

#[test]
fn pushing_a_branch_commits_it_and_never_touches_the_original() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/base.toml", "name = \"base\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));
    let main_tip = status(&alice);
    assert_eq!(main_tip.branch.as_deref(), Some("main"));

    write(
        &alice,
        "collections/api/proposal.toml",
        "name = \"proposal\"\n",
    );
    let PushedBranch {
        branch, sha, url, ..
    } = git_sync::create_branch_and_push(&alice, "add-proposal", "propose", &Auth::None, false)
        .expect("push branch");
    assert_eq!(branch, "add-proposal");
    assert_eq!(sha.len(), 40);
    assert_eq!(url, None, "a filesystem remote is not github");

    let after = status(&alice);
    assert_eq!(after.branch.as_deref(), Some("add-proposal"));
    assert_eq!(after.ahead, 0);
    assert_eq!(after.dirty_total, 0);

    let remote = git2::Repository::open_bare(box_.remote_url()).unwrap();
    assert!(remote.find_reference("refs/heads/add-proposal").is_ok());
    let main = remote
        .find_reference("refs/heads/main")
        .unwrap()
        .target()
        .unwrap();
    assert_ne!(main.to_string(), sha, "main must not have moved");

    let err = git_sync::create_branch_and_push(&alice, "add-proposal", "again", &Auth::None, false)
        .unwrap_err();
    assert_eq!(err.code(), "E_CONFLICT");
}

#[test]
fn pushing_a_branch_is_gated_by_the_scanner_too() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/base.toml", "name = \"base\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));
    write(
        &alice,
        "environments/stage.toml",
        "k = \"AKIAIOSFODNN7EXAMPLE\"\n",
    );

    let err =
        git_sync::create_branch_and_push(&alice, "leak", "nope", &Auth::None, false).unwrap_err();
    assert_eq!(err.code(), "E_SECRET");
    assert_eq!(
        status(&alice).branch.as_deref(),
        Some("main"),
        "a blocked push must not switch branches"
    );

    let err =
        git_sync::create_branch_and_push(&alice, "..bad..", "nope", &Auth::None, true).unwrap_err();
    assert_eq!(err.code(), "E_INVALID_NAME");
}

#[test]
fn the_github_compare_url_is_derived_from_both_remote_shapes() {
    for remote in [
        "https://github.com/o/r.git",
        "https://github.com/o/r",
        "git@github.com:o/r.git",
        "ssh://git@github.com/o/r.git",
        "https://ghp_token00112233445566@github.com/o/r.git",
        "https://GitHub.com/o/r.git",
    ] {
        assert_eq!(
            git_sync::github_compare_url(remote, "add-proposal").as_deref(),
            Some("https://github.com/o/r/compare/add-proposal?expand=1"),
            "{remote}"
        );
    }
    for remote in [
        "https://gitlab.com/o/r.git",
        "git@bitbucket.org:o/r.git",
        "https://github.com/o",
        "https://github.com/o/r/extra.git",
        "/tmp/plain/path",
    ] {
        assert_eq!(git_sync::github_compare_url(remote, "b"), None, "{remote}");
    }
}

#[test]
fn sync_refuses_a_subdirectory_of_a_repository() {
    let box_ = sandbox();
    let ws = box_.workspace("alice");
    let nested = ws.join("collections");
    std::fs::create_dir_all(&nested).unwrap();
    let out = status(&nested).is_repo;
    assert!(!out, "a subdirectory is not itself a workspace repository");
}

/// The honest edge of the "abort, never stay mid-rebase" contract: after a
/// conflict the retry is a three-way merge, so a resolution that invents a THIRD
/// version of the same lines still conflicts. Converging means agreeing with the
/// remote on the conflicting lines first.
#[test]
fn a_third_version_of_the_conflicting_lines_conflicts_again() {
    let box_ = sandbox();
    let alice = box_.workspace("alice");
    write(&alice, "collections/api/shared.toml", "name = \"base\"\n");
    assert!(matches!(sync(&alice, "seed"), SyncOutcome::Pushed { .. }));

    let bob = box_.clone("bob");
    write(&alice, "collections/api/shared.toml", "name = \"alice\"\n");
    write(&bob, "collections/api/shared.toml", "name = \"bob\"\n");
    assert!(matches!(sync(&alice, "alice"), SyncOutcome::Pushed { .. }));
    assert!(matches!(sync(&bob, "bob"), SyncOutcome::Conflicted { .. }));

    write(&bob, "collections/api/shared.toml", "name = \"neither\"\n");
    assert!(
        matches!(sync(&bob, "third way"), SyncOutcome::Conflicted { .. }),
        "a third version of the same lines cannot be merged for the user"
    );
    assert_eq!(
        git2::Repository::open(&bob).unwrap().state(),
        git2::RepositoryState::Clean
    );
}
