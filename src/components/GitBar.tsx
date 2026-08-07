import { useEffect, useState } from "react";
import { currentHost } from "../lib/host";
import { useGithubSession } from "../lib/githubSession";
import { useCollection } from "../store/collection";
import { useGit } from "../store/git";
import { toast } from "../store/toast";
import { Branch, Warn } from "./Icons";
import { ConflictDialog } from "./ConflictDialog";
import { GitRemoteDialog } from "./GitRemoteDialog";
import { SyncDialog } from "./SyncDialog";

/**
 * Working-copy state for the active workspace. Git is a desktop capability —
 * the browser has no git2 — so this bar stays off the web shell.
 */
export function GitBar() {
  const workspace = useCollection((s) => s.workspace);
  const status = useGit((s) => s.status);
  const error = useGit((s) => s.error);
  const busy = useGit((s) => s.busy);
  const refresh = useGit((s) => s.refresh);
  const gh = useGithubSession();
  const [remoteMode, setRemoteMode] = useState<"init" | "connect" | null>(null);
  const [preferGithub, setPreferGithub] = useState(true);
  const [syncOpen, setSyncOpen] = useState(false);
  const [resolveOpen, setResolveOpen] = useState(false);

  useEffect(() => {
    if (currentHost() !== "desktop") return;
    void refresh(workspace);
  }, [workspace, refresh]);

  useEffect(() => {
    if (currentHost() !== "desktop") return;
    const onFocus = () => void refresh(workspace);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [workspace, refresh]);

  if (currentHost() !== "desktop" || workspace === null) return null;
  if (status === null && error === null) return null;

  const openGithub = () => {
    setPreferGithub(true);
    setRemoteMode(status?.isRepo ? "connect" : "init");
  };

  const githubChip = (
    <button
      type="button"
      className="btn-ghost btn-sm"
      disabled={gh.loading || busy}
      title={
        gh.login
          ? `Signed in as ${gh.login}. Click to connect a repository.`
          : "Sign in to GitHub to sync private repos"
      }
      onClick={openGithub}
    >
      {gh.loading ? "GitHub…" : gh.login ? `@${gh.login}` : "Sign in to GitHub"}
    </button>
  );

  if (error && status === null) {
    return (
      <div className="git-bar git-bar-error" role="status">
        <Warn size={12} />
        <span className="git-bar-text">{error}</span>
        <button className="btn-ghost btn-sm" onClick={() => void refresh(workspace)}>
          Retry
        </button>
        {githubChip}
      </div>
    );
  }

  if (!status) return null;

  if (!status.isRepo) {
    return (
      <>
        <div className="git-bar" role="status">
          <Branch size={12} />
          <span className="git-bar-text">not a git repository</span>
          <button
            className="btn btn-sm"
            disabled={busy}
            onClick={() => {
              setPreferGithub(false);
              setRemoteMode("init");
            }}
          >
            Initialize
          </button>
          {githubChip}
        </div>
        {remoteMode === "init" && (
          <GitRemoteDialog
            workspace={workspace}
            mode="init"
            preferGithub={preferGithub}
            onClose={() => setRemoteMode(null)}
          />
        )}
      </>
    );
  }

  const branch =
    status.detached
      ? "detached HEAD"
      : (status.branch ?? "no branch");
  const dirty = status.dirtyTotal;
  const conflicts = status.conflicted.length;
  const hasRemote = Boolean(status.remoteUrl);
  const canSync =
    hasRemote &&
    (dirty > 0 || status.ahead > 0 || status.behind > 0 || conflicts > 0);

  return (
    <>
      <div className="git-bar" role="status">
        <Branch size={12} />
        <span className="git-bar-branch" title={status.remoteUrl ?? undefined}>
          {branch}
        </span>
        {!hasRemote && (
          <span className="git-bar-text" title="No origin remote configured">
            local only
          </span>
        )}
        {hasRemote && (status.ahead > 0 || status.behind > 0) && (
          <span className="git-bar-sync" title="Commits ahead / behind remote">
            ↑{status.ahead} ↓{status.behind}
          </span>
        )}
        <span className="git-bar-spacer" />
        {conflicts > 0 && (
          <button
            type="button"
            className="git-bar-conflict"
            title="Local and remote both changed these files"
            onClick={() => setResolveOpen(true)}
          >
            ⚠ {conflicts} conflict{conflicts === 1 ? "" : "s"}
          </button>
        )}
        <span
          className={`git-bar-dirty ${dirty > 0 ? "git-bar-dirty-on" : ""}`}
          title={
            dirty > 0
              ? status.dirtyFiles.slice(0, 12).join("\n") +
                (status.dirtyTotal > status.dirtyFiles.length ? "\n…" : "")
              : "Working tree clean"
          }
        >
          {dirty === 0 ? "clean" : `${dirty} changed`}
        </span>
        {githubChip}
        {!hasRemote && (
          <button
            className="btn btn-sm"
            disabled={busy}
            title="Add an origin remote so Sync can push and pull"
            onClick={() => {
              setPreferGithub(true);
              setRemoteMode("connect");
            }}
          >
            Connect remote…
          </button>
        )}
        {canSync && (
          <button
            className="btn btn-sm"
            title="Review changes and sync with the remote"
            onClick={() => setSyncOpen(true)}
          >
            Sync
          </button>
        )}
        {conflicts > 0 && (
          <button
            className="btn-ghost btn-sm"
            title="Pick a side for each conflicted file"
            onClick={() => setResolveOpen(true)}
          >
            Resolve
          </button>
        )}
        <button
          className="btn-ghost btn-sm"
          title="Refresh git status"
          onClick={() => void refresh(workspace)}
        >
          Refresh
        </button>
      </div>
      {remoteMode !== null && (
        <GitRemoteDialog
          workspace={workspace}
          mode={remoteMode}
          preferGithub={preferGithub}
          onClose={() => setRemoteMode(null)}
        />
      )}
      {resolveOpen && (
        <ConflictDialog
          workspace={workspace}
          files={status.conflicted}
          onClose={() => setResolveOpen(false)}
          onResolved={() => {
            setResolveOpen(false);
            toast("success", "Saved — Sync to finish");
            void refresh(workspace);
          }}
        />
      )}
      {syncOpen && <SyncDialog onClose={() => setSyncOpen(false)} />}
    </>
  );
}
