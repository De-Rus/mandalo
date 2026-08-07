import { useEffect, useRef, useState } from "react";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  defaultWorkspaceDir,
  errorMessage,
  gitClone,
  githubCreateRepo,
  githubListRepos,
  githubLogout,
  githubPollLogin,
  githubStartLogin,
  githubStatus,
  type GithubRepo,
} from "../lib/api";
import { basename } from "../lib/backend";
import { currentHost } from "../lib/host";
import { repoFolderName } from "../lib/gitUrl";
import { useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { useWorkspaces } from "../store/workspace";
import { Branch, Check, Close, Warn } from "./Icons";

type Tab = "select" | "create";

interface GithubReposDialogProps {
  onClose: () => void;
}

/**
 * Sign in → pick or create a GitHub repository → clone into a local workspace.
 * The token never crosses into React; only the device code and the clone URL do.
 */
export function GithubReposDialog({ onClose }: GithubReposDialogProps) {
  useModalGuard();
  const openWorkspace = useWorkspaces((s) => s.open);
  const loadWorkspaces = useWorkspaces((s) => s.load);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);

  const [login, setLogin] = useState<string | null>(null);
  const [loadingAuth, setLoadingAuth] = useState(true);
  const [userCode, setUserCode] = useState<string | null>(null);
  const [verifyUri, setVerifyUri] = useState<string | null>(null);
  const [signingIn, setSigningIn] = useState(false);

  const [tab, setTab] = useState<Tab>("select");
  const [repos, setRepos] = useState<GithubRepo[]>([]);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<GithubRepo | null>(null);
  const [loadingRepos, setLoadingRepos] = useState(false);

  const [newName, setNewName] = useState("");
  const [newPrivate, setNewPrivate] = useState(true);

  const [parent, setParent] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const folder =
    tab === "select" && selected
      ? selected.name
      : tab === "create"
        ? repoFolderName(newName || "workspace")
        : null;
  const dest =
    parent && folder ? `${parent.replace(/\/+$/, "")}/${folder}` : null;

  useEffect(() => {
    void defaultWorkspaceDir()
      .then((dir) => setParent(dir))
      .catch(() => setParent(null));
    if (currentHost() !== "desktop") {
      setLoadingAuth(false);
      return;
    }
    void githubStatus()
      .then((s) => {
        setLogin(s.connected ? s.login : null);
        setLoadingAuth(false);
      })
      .catch((e) => {
        setError(errorMessage(e));
        setLoadingAuth(false);
      });
    return () => {
      if (pollRef.current) clearTimeout(pollRef.current);
    };
  }, []);

  useEffect(() => {
    if (!login) {
      setRepos([]);
      return;
    }
    setLoadingRepos(true);
    setError(null);
    void githubListRepos()
      .then((list) => {
        setRepos(list);
        setLoadingRepos(false);
      })
      .catch((e) => {
        setError(errorMessage(e));
        setLoadingRepos(false);
      });
  }, [login]);

  const filtered = repos.filter((r) => {
    if (query.trim() === "") return true;
    const q = query.trim().toLowerCase();
    return (
      r.fullName.toLowerCase().includes(q) || r.name.toLowerCase().includes(q)
    );
  });

  const startLogin = async () => {
    setSigningIn(true);
    setError(null);
    try {
      const start = await githubStartLogin(false);
      setUserCode(start.userCode);
      setVerifyUri(start.verificationUri);
      try {
        await openUrl(start.verificationUri);
      } catch {
        /* user can open the link by hand */
      }
      const tick = async () => {
        try {
          const poll = await githubPollLogin(start.handle);
          if (poll.pending) {
            pollRef.current = setTimeout(() => void tick(), start.interval * 1000);
            return;
          }
          setLogin(poll.user?.login ?? null);
          setUserCode(null);
          setVerifyUri(null);
          setSigningIn(false);
          toast("success", `Signed in as ${poll.user?.login ?? "GitHub"}`);
        } catch (e) {
          setError(errorMessage(e));
          setSigningIn(false);
          setUserCode(null);
        }
      };
      pollRef.current = setTimeout(() => void tick(), start.interval * 1000);
    } catch (e) {
      setError(errorMessage(e));
      setSigningIn(false);
    }
  };

  const logout = async () => {
    try {
      await githubLogout();
      setLogin(null);
      setSelected(null);
      setRepos([]);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const pickParent = async () => {
    const picked = await pickDirectory({ directory: true, multiple: false });
    if (typeof picked === "string") setParent(picked);
  };

  const openClone = async (cloneUrl: string, into: string) => {
    setBusy(`Cloning into ${into}…`);
    setError(null);
    try {
      await gitClone(cloneUrl, into);
      const result = await openWorkspace(into);
      if (!result)
        throw new Error("The clone finished, but the workspace did not open.");
      await switchWorkspace(result.path);
      await loadWorkspaces();
      await useCollection.getState().ensureStarterCollection(basename(result.path));
      toast("success", `Opened “${basename(result.path)}”`);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const cloneSelected = async () => {
    if (!selected || !dest) return;
    await openClone(selected.cloneUrl, dest);
  };

  const createAndClone = async () => {
    if (!dest || newName.trim() === "") return;
    setBusy(`Creating ${newName.trim()} on GitHub…`);
    setError(null);
    try {
      const repo = await githubCreateRepo(newName.trim(), newPrivate);
      setBusy(`Cloning into ${dest}…`);
      await openClone(repo.cloneUrl, dest);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(null);
    }
  };

  if (currentHost() !== "desktop") {
    return (
      <div className="modal-backdrop" onClick={onClose}>
        <div
          className="modal"
          style={{ width: 460 }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="modal-head">
            <h2>Clone from GitHub</h2>
            <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
              <Close size={13} />
            </button>
          </div>
          <div className="modal-body">
            <p className="empty-line">
              Signing in to GitHub and cloning repositories needs the Mándalo
              desktop app. In the browser, use{" "}
              <strong>Browse shared collection…</strong> for a public repo
              (read-only).
            </p>
            <div className="modal-actions">
              <button className="btn" onClick={onClose}>
                Close
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-wide"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Clone from GitHub</h2>
          <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {loadingAuth && <p className="import-status">Checking GitHub…</p>}

          {!loadingAuth && !login && (
            <>
              <p className="import-hint">
                Sign in once. Mándalo opens GitHub in your browser, never in an
                embedded window. The token stays on this machine.
              </p>
              {userCode && (
                <div className="notice notice-wrap">
                  <Branch size={13} />
                  <span className="notice-text">
                    Enter code{" "}
                    <strong className="mono">{userCode}</strong> at{" "}
                    {verifyUri ? (
                      <a href={verifyUri} target="_blank" rel="noreferrer">
                        {verifyUri}
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
                  disabled={signingIn}
                  onClick={() => void startLogin()}
                >
                  {signingIn ? "Waiting…" : "Sign in with GitHub"}
                </button>
              </div>
            </>
          )}

          {!loadingAuth && login && (
            <>
              <div className="github-account">
                <span>
                  Signed in as <strong>{login}</strong>
                </span>
                <button className="btn-ghost btn-sm" onClick={() => void logout()}>
                  Sign out
                </button>
              </div>

              <div className="body-types" role="tablist" aria-label="Repository action">
                <button
                  role="tab"
                  aria-selected={tab === "select"}
                  className={`body-type ${tab === "select" ? "body-type-active" : ""}`}
                  onClick={() => setTab("select")}
                >
                  Select repository
                </button>
                <button
                  role="tab"
                  aria-selected={tab === "create"}
                  className={`body-type ${tab === "create" ? "body-type-active" : ""}`}
                  onClick={() => setTab("create")}
                >
                  Create repository
                </button>
              </div>

              {tab === "select" && (
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
                    <div className="github-repo-list" role="listbox" aria-label="Repositories">
                      {filtered.length === 0 ? (
                        <p className="empty-line">No repositories match.</p>
                      ) : (
                        filtered.map((repo) => (
                          <button
                            key={repo.fullName}
                            type="button"
                            role="option"
                            aria-selected={selected?.fullName === repo.fullName}
                            className={`github-repo-row ${
                              selected?.fullName === repo.fullName
                                ? "github-repo-row-active"
                                : ""
                            }`}
                            onClick={() => setSelected(repo)}
                          >
                            <span className="github-repo-name mono">{repo.fullName}</span>
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

              {tab === "create" && (
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
                    Creates the repo under <strong>{login}</strong> with a README
                    so it can be cloned immediately.
                  </p>
                </>
              )}

              <label className="field">
                <span className="field-label">Clone into</span>
                <div className="import-url-row">
                  <input
                    className="input mono"
                    readOnly
                    value={dest ?? parent ?? ""}
                    aria-label="Clone destination"
                  />
                  <button className="btn" type="button" onClick={() => void pickParent()}>
                    Choose…
                  </button>
                </div>
              </label>

              {busy && <p className="import-status">{busy}</p>}

              <div className="modal-actions">
                <button className="btn-ghost" onClick={onClose} disabled={busy !== null}>
                  Cancel
                </button>
                {tab === "select" ? (
                  <button
                    className="btn btn-primary"
                    disabled={busy !== null || !selected || !dest}
                    onClick={() => void cloneSelected()}
                  >
                    <Check size={13} />
                    Clone and open
                  </button>
                ) : (
                  <button
                    className="btn btn-primary"
                    disabled={busy !== null || newName.trim() === "" || !dest}
                    onClick={() => void createAndClone()}
                  >
                    <Branch size={13} />
                    Create and open
                  </button>
                )}
              </div>
            </>
          )}

          {error && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
