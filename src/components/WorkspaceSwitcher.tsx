import { useState } from "react";
import { errorMessage } from "../lib/api";
import { basename } from "../lib/backend";
import { pickDirectory } from "../lib/pickDirectory";
import { useCollection } from "../store/collection";
import { useRemote } from "../store/remote";
import { toast } from "../store/toast";
import { useWorkspaces } from "../store/workspace";
import { ConfirmModal } from "./ConfirmModal";
import { Dropdown, MenuItem } from "./Dropdown";
import { Check, ChevronDown, Layers, Plus, Trash } from "./Icons";

function displayPath(path: string): string {
  const shortened = path
    .replace(/^\/Users\/[^/]+/, "~")
    .replace(/^\/home\/[^/]+/, "~")
    .replace(/^\/private\/var\/folders\/[^/]+\/[^/]+\/[^/]+\/T/, "~/tmp");
  if (shortened.length <= 48) return shortened;
  return `${shortened.slice(0, 18)}…${shortened.slice(-26)}`;
}

export function WorkspaceSwitcher() {
  const items = useWorkspaces((s) => s.items);
  const activeId = useWorkspaces((s) => s.activeId);
  const select = useWorkspaces((s) => s.select);
  const createWorkspace = useWorkspaces((s) => s.create);
  const removeWorkspace = useWorkspaces((s) => s.remove);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);
  const ensureStarterCollection = useCollection((s) => s.ensureStarterCollection);
  const origin = useRemote((s) => s.origin);
  const [pendingRemove, setPendingRemove] = useState<{
    id: string;
    name: string;
  } | null>(null);

  const active = items.find((w) => w.id === activeId) ?? null;

  const pick = async (id: string, close: () => void) => {
    close();
    if (id === activeId) return;
    try {
      const path = await select(id);
      if (path) await switchWorkspace(path);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  const doCreate = async (close: () => void) => {
    try {
      const dir = await pickDirectory({
        title: "Create workspace in empty folder",
        closeMenu: close,
      });
      if (!dir) return;
      const path = await createWorkspace(dir, basename(dir));
      if (!path) return;
      await switchWorkspace(path);
      await ensureStarterCollection(basename(path));
      toast("success", `Workspace “${basename(path)}” created`);
    } catch (e) {
      close();
      toast("error", errorMessage(e));
    }
  };

  const confirmRemove = async () => {
    if (pendingRemove === null) return;
    const { id, name } = pendingRemove;
    setPendingRemove(null);
    try {
      const path = await removeWorkspace(id);
      if (path) await switchWorkspace(path);
      toast(
        "success",
        `“${name}” left the list — the folder on disk is untouched`,
      );
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  return (
    <>
      <Dropdown
        align="left"
        menuClassName="ws-menu"
        trigger={({ open: isOpen, toggle }) => (
          <button
            className={`ws-trigger ${isOpen ? "ws-trigger-open" : ""}`}
            onClick={toggle}
            aria-haspopup="menu"
            aria-expanded={isOpen}
            title="Switch workspace"
          >
            <Layers size={14} />
            <span className="ws-name">{active?.name ?? "Workspace"}</span>
            {origin !== null && <span className="ws-readonly">read-only</span>}
            <ChevronDown size={12} className="ws-caret" />
          </button>
        )}
      >
        {(close) => (
          <>
            <div className="menu-head">Workspaces</div>
            {items.length === 0 ? (
              <p className="empty-line ws-empty">No workspaces yet.</p>
            ) : (
              items.map((w) => (
                <div
                  key={w.id}
                  className={`menu-item ws-item ${w.id === activeId ? "menu-item-active" : ""}`}
                  role="menuitem"
                  title={w.path}
                >
                  <button
                    type="button"
                    className="ws-item-pick"
                    onClick={() => void pick(w.id, close)}
                  >
                    <span className="menu-item-icon">
                      {w.id === activeId ? <Check size={13} /> : null}
                    </span>
                    <span className="ws-item-body">
                      <span className="menu-item-label">{w.name}</span>
                      <span className="ws-item-path">{displayPath(w.path)}</span>
                    </span>
                  </button>
                  {w.id !== "browser" && (
                    <button
                      type="button"
                      className="btn-ghost btn-icon btn-icon-sm ws-item-remove"
                      aria-label={`Remove ${w.name}`}
                      title="Remove from list (keeps the folder)"
                      onClick={(e) => {
                        e.stopPropagation();
                        close();
                        setPendingRemove({ id: w.id, name: w.name });
                      }}
                    >
                      <Trash size={12} />
                    </button>
                  )}
                </div>
              ))
            )}
            <div className="menu-sep" />
            <MenuItem
              icon={<Plus size={13} />}
              hint="Empty folder"
              onClick={() => void doCreate(close)}
            >
              New workspace…
            </MenuItem>
          </>
        )}
      </Dropdown>
      {pendingRemove && (
        <ConfirmModal
          title="Remove workspace"
          message={`“${pendingRemove.name}” will leave this list. The folder on disk is not deleted.`}
          confirmLabel="Remove from list"
          onConfirm={() => void confirmRemove()}
          onClose={() => setPendingRemove(null)}
        />
      )}
    </>
  );
}
