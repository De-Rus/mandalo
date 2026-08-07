import { useState } from "react";
import { addSampleCollection, errorMessage } from "../lib/api";
import { basename } from "../lib/backend";
import { pickDirectory } from "../lib/pickDirectory";
import {
  checkForUpdate,
  installAndRelaunch,
  updateSummary,
  updaterAvailable,
  type Update,
} from "../lib/updater";
import { useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { useRemote } from "../store/remote";
import { useWorkspaces } from "../store/workspace";
import { CloneRepoDialog } from "./CloneRepoDialog";
import { ConfirmModal } from "./ConfirmModal";
import { GithubReposDialog } from "./GithubReposDialog";
import { Dropdown, MenuItem } from "./Dropdown";
import {
  ArrowDown,
  Branch,
  Doc,
  Dots,
  Eye,
  FolderOpen,
} from "./Icons";

export function WorkspaceActions() {
  const openWorkspace = useWorkspaces((s) => s.open);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);
  const ensureStarterCollection = useCollection((s) => s.ensureStarterCollection);
  const workspace = useCollection((s) => s.workspace);
  const refreshTree = useCollection((s) => s.refreshTree);
  const openRemote = useRemote((s) => s.open);
  const [cloneOpen, setCloneOpen] = useState(false);
  const [githubOpen, setGithubOpen] = useState(false);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);

  const doOpen = async (close: () => void) => {
    try {
      const dir = await pickDirectory({
        title: "Open workspace folder",
        closeMenu: close,
      });
      if (!dir) return;
      const result = await openWorkspace(dir);
      if (!result) return;
      await switchWorkspace(result.path);
      await ensureStarterCollection(basename(result.path));
      const moved = result.migrated.length;
      if (moved > 0)
        toast(
          "success",
          `Moved ${moved} legacy request file(s) into the “default” collection`,
        );
      else toast("success", `Workspace “${basename(result.path)}” opened`);
    } catch (e) {
      close();
      toast("error", errorMessage(e));
    }
  };

  const doAddSample = async (close: () => void) => {
    close();
    if (workspace === null) return;
    try {
      const slug = await addSampleCollection(workspace);
      await refreshTree();
      toast("success", `Sample collection added as “${slug}”`);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  const doCheckUpdates = async (close: () => void) => {
    close();
    try {
      const update = await checkForUpdate();
      if (!update) {
        toast("success", "You’re on the latest version");
        return;
      }
      setPendingUpdate(update);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  return (
    <>
      <Dropdown
        align="left"
        menuClassName="ws-actions-menu"
        trigger={({ open: isOpen, toggle }) => (
          <button
            className={`btn-ghost btn-icon ws-actions-btn ${isOpen ? "menu-item-active" : ""}`}
            title="Workspace actions"
            aria-label="Workspace actions"
            aria-haspopup="menu"
            aria-expanded={isOpen}
            onClick={toggle}
          >
            <Dots size={14} />
          </button>
        )}
      >
        {(close) => (
          <>
            <div className="menu-head">This machine</div>
            <MenuItem
              icon={<FolderOpen size={13} />}
              hint="Existing folder"
              onClick={() => void doOpen(close)}
            >
              Open folder…
            </MenuItem>
            {updaterAvailable() && (
              <MenuItem
                icon={<ArrowDown size={13} />}
                hint="GitHub Releases"
                onClick={() => void doCheckUpdates(close)}
              >
                Check for updates…
              </MenuItem>
            )}

            <div className="menu-sep" />
            <div className="menu-head">GitHub</div>
            <MenuItem
              icon={<Branch size={13} />}
              hint="Sign in & pick a repo"
              onClick={() => {
                close();
                setGithubOpen(true);
              }}
            >
              Clone from GitHub…
            </MenuItem>
            <MenuItem
              icon={<ArrowDown size={13} />}
              hint="Any git URL"
              onClick={() => {
                close();
                setCloneOpen(true);
              }}
            >
              Clone from URL…
            </MenuItem>
            <MenuItem
              icon={<Eye size={13} />}
              hint="Read-only preview"
              onClick={() => {
                close();
                openRemote();
              }}
            >
              Browse shared collection…
            </MenuItem>

            <div className="menu-sep" />
            <MenuItem
              icon={<Doc size={13} />}
              hint="Into the active workspace"
              onClick={() => void doAddSample(close)}
            >
              Add sample collection
            </MenuItem>
          </>
        )}
      </Dropdown>
      {githubOpen && <GithubReposDialog onClose={() => setGithubOpen(false)} />}
      {cloneOpen && <CloneRepoDialog onClose={() => setCloneOpen(false)} />}
      {pendingUpdate !== null && (
        <ConfirmModal
          title="Update available"
          message={updateSummary(pendingUpdate)}
          confirmLabel="Install & restart"
          tone="primary"
          onConfirm={() => {
            const update = pendingUpdate;
            setPendingUpdate(null);
            toast("info", "Downloading update…");
            void installAndRelaunch(update).catch((e) =>
              toast("error", errorMessage(e)),
            );
          }}
          onClose={() => setPendingUpdate(null)}
        />
      )}
    </>
  );
}
