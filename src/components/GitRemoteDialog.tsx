import { useEffect, useState } from "react";
import {
  errorMessage,
  githubCreateRepo,
  githubListRepos,
  type GithubRepo,
} from "../lib/api";
import { useGithubSession } from "../lib/githubSession";
import { useGit } from "../store/git";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { Branch, Check, Close, Warn } from "./Icons";

type Mode = "init" | "connect";
type Source = "github" | "url";
type RepoTab = "select" | "create";

interface GitRemoteDialogProps {
  workspace: string;
  mode: Mode;
  /** Prefer the GitHub tab when opening Connect remote. */
  preferGithub?: boolean;
  onClose: () => void;
}

/**
 * Initialize a local repo (optional URL) or attach `origin` to the current
 * workspace — either by pasting an HTTPS URL or by signing in to GitHub and
 * picking / creating a repository.
 */
export function GitRemoteDialog({
  workspace,
  mode,
  preferGithub = true,
  onClose,
}: GitRemoteDialogProps) {
  useModalGuard();
  const initRepo = useGit((s) => s.initRepo);
  const busyGit = useGit((s) => s.busy);
  const gh = useGithubSession();

  const [source, setSource] = useState<Source>(
    mode === "connect" && preferGithub ? "github" : "url",
  );
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const [repoTab, setRepoTab] = useState<RepoTab>("select");
  const [repos, setRepos] = useState<GithubRepo[]>([]);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<GithubRepo | null>(null);
  const [loadingRepos, setLoadingRepos] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPrivate, setNewPrivate] = useState(true);

  useEffect(() => {
    if (!gh.login || source !== "github") {
      setRepos([]);
      return;
    }
    setLoadingRepos(true);
    void githubListRepos()
      .then((list) => {
        setRepos(list);
        setLoadingRepos(false);
      })
      .catch((e) => {
        setError(errorMessage(e));
        setLoadingRepos(false);
      });
  }, [gh.login, source]);

  const filtered = repos.filter((r) => {
    if (query.trim() === "") return true;
    const q = query.trim().toLowerCase();
    return (
      r.fullName.toLowerCase().includes(q) || r.name.toLowerCase().includes(q)
    );
  });

  const attach = async (cloneUrl: string, label: string) => {
    setBusy(label);
    setError(null);
    try {
      await initRepo(workspace, cloneUrl);
      toast("success", `Connected ${cloneUrl}`);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const submitUrl = async () => {
    const trimmed = url.trim();
    if (mode === "connect" && trimmed === "") return;
    setBusy(mode === "init" ? "Initializing…" : "Connecting…");
    setError(null);
    try {
      await initRepo(workspace, trimmed === "" ? null : trimmed);
      toast(
        "success",
        mode === "init"
          ? trimmed === ""
            ? "Initialized a git repository here"
            : `Initialized and connected ${trimmed}`
          : `Remote connected — ${trimmed}`,
      );
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const connectSelected = async () => {
    if (!selected) return;
    await attach(selected.cloneUrl, `Connecting ${selected.fullName}…`);
  };

  const createAndConnect = async () => {
    if (newName.trim() === "") return;
    setBusy(`Creating ${newName.trim()} on GitHub…`);
    setError(null);
    try {
      const repo = await githubCreateRepo(newName.trim(), newPrivate);
      await attach(repo.cloneUrl, `Connecting ${repo.fullName}…`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(null);
    }
  };

  const title = mode === "init" ? "Initialize git" : "Connect remote";
  const blocked = busy !== null || busyGit;
  const showGithub = mode === "connect" || mode === "init";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className={`modal ${source === "github" ? "modal-wide" : ""}`}
        style={source === "github" ? undefined : { width: 460 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{title}</h2>
          <button
            className="btn-ghost btn-icon"
            aria-label="Close"
            onClick={onClose}
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {showGithub && (
            <div className="segmented import-sources" role="tablist">
              <button
                type="button"
                role="tab"
                className={`segment ${source === "github" ? "segment-active" : ""}`}
                onClick={() => setSource("github")}
              >
                GitHub
              </button>
              <button
                type="button"
                role="tab"
                className={`segment ${source === "url" ? "segment-active" : ""}`}
                onClick={() => setSource("url")}
              >
                URL
              </button>
            </div>
          )}

          {source === "url" && (
            <>
              <p className="empty-line">
                {mode === "init"
                  ? "Creates a repository in this folder. Attach a remote now or leave it blank."
                  : "Adds origin so Sync can push and pull. Prefer a clean https:// URL — sign in to GitHub for private repos."}
              </p>
              <label className="field">
                <span className="field-label">
                  Remote URL{mode === "init" ? " (optional)" : ""}
                </span>
                <input
                  className="input mono"
                  placeholder="https://github.com/org/repo.git"
                  value={url}
                  autoFocus
                  onChange={(e) => setUrl(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void submitUrl();
                    }
                  }}
                />
              </label>
              <div className="modal-actions">
                <button className="btn-ghost" onClick={onClose} disabled={blocked}>
                  Cancel
                </button>
                <button
                  className="btn btn-primary"
                  disabled={
                    blocked || (mode === "connect" && url.trim() === "")
                  }
                  onClick={() => void submitUrl()}
                >
                  {mode === "init" ? "Initialize" : "Connect"}
                </button>
              </div>
            </>
          )}

          {source === "github" && (
            <>
              {gh.loading && <p className="import-status">Checking GitHub…</p>}

              {!gh.loading && !gh.login && (
                <>
                  <p className="import-hint">
                    Sign in once. Mándalo opens GitHub in your browser. The token
                    stays on this machine and is used when you Sync.
                  </p>
                  {gh.userCode && (
                    <div className="notice notice-wrap">
                      <Branch size={13} />
                      <span className="notice-text">
                        Enter code{" "}
                        <strong className="mono">{gh.userCode}</strong> at{" "}
                        {gh.verifyUri ? (
                          <a href={gh.verifyUri} target="_blank" rel="noreferrer">
                            {gh.verifyUri}
                          </a>
                        ) : (
                          "github.com/login/device"
                        )}
                        , then approve access. Waiting for GitHub…
                      </span>
                    </div>
                  )}
                  <div className="modal-actions">
                    <button className="btn-ghost" onClick={onClose}>
                      Cancel
                    </button>
                    <button
                      className="btn btn-primary"
                      disabled={gh.signingIn}
                      onClick={() => void gh.signIn()}
                    >
                      {gh.signingIn ? "Waiting…" : "Sign in with GitHub"}
                    </button>
                  </div>
                </>
              )}

              {!gh.loading && gh.login && (
                <>
                  <div className="github-account">
                    <span>
                      Signed in as <strong>{gh.login}</strong>
                    </span>
                    <button
                      className="btn-ghost btn-sm"
                      onClick={() => void gh.signOut()}
                    >
                      Sign out
                    </button>
                  </div>

                  <div
                    className="body-types"
                    role="tablist"
                    aria-label="Repository action"
                  >
                    <button
                      type="button"
                      role="tab"
                      aria-selected={repoTab === "select"}
                      className={`body-type ${repoTab === "select" ? "body-type-active" : ""}`}
                      onClick={() => setRepoTab("select")}
                    >
                      Select repository
                    </button>
                    <button
                      type="button"
                      role="tab"
                      aria-selected={repoTab === "create"}
                      className={`body-type ${repoTab === "create" ? "body-type-active" : ""}`}
                      onClick={() => setRepoTab("create")}
                    >
                      Create repository
                    </button>
                  </div>

                  {repoTab === "select" && (
                    <>
                      <label className="field">
                        <span className="field-label">Your repositories</span>
                        <input
                          className="input"
                          placeholder="Filter by name…"
                          value={query}
                          onChange={(e) => setQuery(e.target.value)}
                        />
                      </label>
                      {loadingRepos ? (
                        <p className="import-status">Loading repositories…</p>
                      ) : (
                        <div
                          className="github-repo-list"
                          role="listbox"
                          aria-label="Repositories"
                        >
                          {filtered.length === 0 ? (
                            <p className="empty-line">No repositories match.</p>
                          ) : (
                            filtered.map((repo) => (
                              <button
                                key={repo.fullName}
                                type="button"
                                role="option"
                                aria-selected={
                                  selected?.fullName === repo.fullName
                                }
                                className={`github-repo-row ${
                                  selected?.fullName === repo.fullName
                                    ? "github-repo-row-active"
                                    : ""
                                }`}
                                onClick={() => setSelected(repo)}
                              >
                                <span className="github-repo-name mono">
                                  {repo.fullName}
                                </span>
                                <span className="github-repo-meta">
                                  {repo.private ? "private" : "public"}
                                </span>
                              </button>
                            ))
                          )}
                        </div>
                      )}
                    </>
                  )}

                  {repoTab === "create" && (
                    <>
                      <label className="field">
                        <span className="field-label">Repository name</span>
                        <input
                          className="input mono"
                          placeholder="my-api-collection"
                          value={newName}
                          onChange={(e) => setNewName(e.target.value)}
                        />
                      </label>
                      <label className="export-pick-row">
                        <input
                          type="checkbox"
                          className="checkbox"
                          checked={newPrivate}
                          onChange={(e) => setNewPrivate(e.target.checked)}
                        />
                        <span>Private repository</span>
                      </label>
                      <p className="import-hint">
                        Creates the repo under <strong>{gh.login}</strong> and
                        sets it as <span className="mono">origin</span> on this
                        workspace — nothing is pushed until you Sync.
                      </p>
                    </>
                  )}

                  {busy && <p className="import-status">{busy}</p>}

                  <div className="modal-actions">
                    <button
                      className="btn-ghost"
                      onClick={onClose}
                      disabled={blocked}
                    >
                      Cancel
                    </button>
                    {repoTab === "select" ? (
                      <button
                        className="btn btn-primary"
                        disabled={blocked || !selected}
                        onClick={() => void connectSelected()}
                      >
                        <Check size={13} />
                        {mode === "init" ? "Initialize and connect" : "Connect"}
                      </button>
                    ) : (
                      <button
                        className="btn btn-primary"
                        disabled={blocked || newName.trim() === ""}
                        onClick={() => void createAndConnect()}
                      >
                        <Branch size={13} />
                        {mode === "init" ? "Create and initialize" : "Create and connect"}
                      </button>
                    )}
                  </div>
                </>
              )}
            </>
          )}

          {(error || gh.error) && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error ?? gh.error}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
