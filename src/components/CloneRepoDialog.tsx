import { useEffect, useState } from "react";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";
import {
  defaultWorkspaceDir,
  errorMessage,
  gitClone,
  githubStatus,
} from "../lib/api";
import { basename } from "../lib/backend";
import { currentHost } from "../lib/host";
import { repoFolderName } from "../lib/gitUrl";
import { useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { useWorkspaces } from "../store/workspace";
import { Branch, Close, Warn } from "./Icons";

interface CloneRepoDialogProps {
  onClose: () => void;
}

/**
 * A normal working copy you own — not the read-only shared-collection preview.
 * Private GitHub repos use the token from `mandalo login` when one is stored.
 */
export function CloneRepoDialog({ onClose }: CloneRepoDialogProps) {
  useModalGuard();
  const openWorkspace = useWorkspaces((s) => s.open);
  const loadWorkspaces = useWorkspaces((s) => s.load);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);

  const [url, setUrl] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const [githubLogin, setGithubLogin] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const folder = repoFolderName(url);
  const dest = parent ? `${parent.replace(/\/+$/, "")}/${folder}` : null;

  useEffect(() => {
    void defaultWorkspaceDir()
      .then((dir) => setParent(dir))
      .catch(() => setParent(null));
    if (currentHost() !== "desktop") return;
    void githubStatus()
      .then((s) => setGithubLogin(s.connected ? s.login : null))
      .catch(() => setGithubLogin(null));
  }, []);

  const pickParent = async () => {
    const picked = await pickDirectory({ directory: true, multiple: false });
    if (typeof picked === "string") setParent(picked);
  };

  const clone = async () => {
    if (dest === null || url.trim() === "") return;
    setBusy(`Cloning into ${dest}…`);
    setError(null);
    try {
      await gitClone(url.trim(), dest);
      const result = await openWorkspace(dest);
      if (!result) throw new Error("The clone finished, but the workspace did not open.");
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

  if (currentHost() !== "desktop") {
    return (
      <div className="modal-backdrop" onClick={onClose}>
        <div
          className="modal"
          style={{ width: 460 }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="modal-head">
            <h2>Clone repository</h2>
            <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
              <Close size={13} />
            </button>
          </div>
          <div className="modal-body">
            <p className="empty-line">
              Cloning a git repository needs the Mándalo desktop app. In the
              browser, use <strong>Browse shared collection…</strong> for a
              public GitHub repo (read-only).
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
        className="modal"
        style={{ width: 520 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Clone repository</h2>
          <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Repository URL</span>
            <input
              className="input mono"
              autoFocus
              placeholder="https://github.com/owner/name.git"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && dest && url.trim() !== "") {
                  e.preventDefault();
                  void clone();
                }
              }}
            />
          </label>

          <label className="field">
            <span className="field-label">Parent folder</span>
            <div className="import-url-row">
              <input
                className="input mono"
                readOnly
                value={parent ?? ""}
                placeholder="Choose where the clone lands"
                aria-label="Parent folder"
              />
              <button className="btn" type="button" onClick={() => void pickParent()}>
                Choose…
              </button>
            </div>
            {parent && url.trim() !== "" && (
              <p className="import-hint">
                Clones into <code className="mono">{dest}</code>
              </p>
            )}
          </label>

          <p className="import-hint">
            You get a normal workspace you can edit and push back to. Public
            HTTPS clones need no sign-in.
            {githubLogin
              ? ` Signed in to GitHub as ${githubLogin} — private repos use that token.`
              : " For a private GitHub repo, run `mandalo login` once in a terminal first."}
          </p>

          {busy && <p className="import-status">{busy}</p>}
          {error && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error}</span>
            </div>
          )}

          <div className="modal-actions">
            <button className="btn-ghost" onClick={onClose} disabled={busy !== null}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={busy !== null || !dest || url.trim() === ""}
              onClick={() => void clone()}
            >
              <Branch size={13} />
              Clone and open
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
