import { open } from "@tauri-apps/plugin-dialog";
import { errorMessage } from "../lib/api";
import { basename } from "../lib/backend";
import { useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { useWorkspaces } from "../store/workspace";
import { Dropdown, MenuItem } from "./Dropdown";
import { Check, ChevronDown, Layers, Plus } from "./Icons";

export function WorkspaceSwitcher() {
  const items = useWorkspaces((s) => s.items);
  const activeId = useWorkspaces((s) => s.activeId);
  const select = useWorkspaces((s) => s.select);
  const openWorkspace = useWorkspaces((s) => s.open);
  const createWorkspace = useWorkspaces((s) => s.create);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);

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

  const chooseDir = async (): Promise<string | null> => {
    const dir = await open({ directory: true, multiple: false });
    return typeof dir === "string" ? dir : null;
  };

  const doOpen = async (close: () => void) => {
    close();
    try {
      const dir = await chooseDir();
      if (!dir) return;
      const result = await openWorkspace(dir);
      if (!result) return;
      await switchWorkspace(result.path);
      if (result.migrated.length > 0)
        toast(
          "success",
          `Moved ${result.migrated.length} legacy request file(s) into the “default” collection`,
        );
      else toast("success", `Workspace “${basename(result.path)}” opened`);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  const doCreate = async (close: () => void) => {
    close();
    try {
      const dir = await chooseDir();
      if (!dir) return;
      const path = await createWorkspace(dir, basename(dir));
      if (!path) return;
      await switchWorkspace(path);
      toast("success", `Workspace “${basename(path)}” created`);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  return (
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
          <ChevronDown size={12} className="ws-caret" />
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="menu-head">Workspaces</div>
          {items.map((w) => (
            <button
              key={w.id}
              className={`menu-item ws-item ${w.id === activeId ? "menu-item-active" : ""}`}
              role="menuitem"
              onClick={() => void pick(w.id, close)}
            >
              <span className="menu-item-icon">
                {w.id === activeId ? <Check size={13} /> : null}
              </span>
              <span className="ws-item-body">
                <span className="menu-item-label">{w.name}</span>
                <span className="ws-item-path">{w.path}</span>
              </span>
            </button>
          ))}
          <div className="menu-sep" />
          <MenuItem icon={<Plus size={13} />} onClick={() => void doCreate(close)}>
            Create workspace…
          </MenuItem>
          <MenuItem icon={<Layers size={13} />} onClick={() => void doOpen(close)}>
            Open workspace…
          </MenuItem>
        </>
      )}
    </Dropdown>
  );
}
