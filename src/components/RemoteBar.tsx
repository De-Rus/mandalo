import { useState } from "react";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";
import { errorMessage, saveWorkspaceCopy } from "../lib/api";
import { currentHost } from "../lib/host";
import { useCollection } from "../store/collection";
import { useRemote } from "../store/remote";
import { toast } from "../store/toast";
import { useWorkspaces } from "../store/workspace";
import { Copy, Eye } from "./Icons";

/**
 * A workspace opened from a link is somebody else's, and has to look like it
 * every moment it is on screen — not only in the dialog that opened it. This is
 * also the one place the user can turn it into a workspace they own.
 */
export function RemoteBar() {
  const origin = useRemote((s) => s.origin);
  const workspace = useCollection((s) => s.workspace);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);
  const loadWorkspaces = useWorkspaces((s) => s.load);
  const refreshOrigin = useRemote((s) => s.refresh);
  const [busy, setBusy] = useState(false);

  if (origin === null || workspace === null) return null;

  const saveCopy = async () => {
    setBusy(true);
    try {
      const name = origin.label.split("/").pop() ?? "Collection";
      let dest = name;
      if (currentHost() === "desktop") {
        const picked = await pickDirectory({ directory: true, multiple: false });
        if (typeof picked !== "string") return;
        dest = `${picked}/${name}`;
      }
      const info = await saveWorkspaceCopy(workspace, dest, name);
      await switchWorkspace(info.path);
      await loadWorkspaces();
      await refreshOrigin(info.path);
      toast("success", `Saved a copy you own at ${info.path}`);
    } catch (e) {
      toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="remote-bar">
      <Eye size={13} />
      <span className="remote-bar-text">
        Read-only copy of <strong>{origin.label}</strong>
        {origin.commit ? ` at ${origin.commit.slice(0, 8)}` : ""}. You can send
        these requests; you cannot change them, and nothing here ran on its own.
      </span>
      <button className="btn" disabled={busy} onClick={() => void saveCopy()}>
        <Copy size={13} />
        Save a copy locally
      </button>
    </div>
  );
}
