import { useEffect } from "react";
import { useCollection } from "../../store/collection";
import { useEnv } from "../../store/env";
import { subscribe, type WorkspaceChange } from "./sync";

/**
 * A save fires on every pause in typing, so the sidebar — which only shows names and
 * methods — is refreshed on a trailing timer while the open request itself reloads at
 * once. The tab the user is looking at stays live; the tree does not re-walk per keystroke.
 */
const TREE_SETTLE_MS = 800;

export function applyChange(
  change: WorkspaceChange,
  scheduleTree: () => void,
): void {
  const { workspace } = useCollection.getState();
  if (workspace === null || change.workspace !== workspace) return;

  if (change.scope === "environments") {
    void useEnv.getState().refresh();
    return;
  }
  if (change.scope === "tree") {
    void useCollection.getState().applyRemoteTree();
    return;
  }
  if (change.collection && change.path) {
    void useCollection
      .getState()
      .applyRemoteRequest(change.collection, change.path);
    scheduleTree();
  }
}

export function useWorkspaceSync(): void {
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const scheduleTree = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        void useCollection.getState().refreshTree();
      }, TREE_SETTLE_MS);
    };
    const stop = subscribe((change) => applyChange(change, scheduleTree));
    return () => {
      if (timer) clearTimeout(timer);
      stop();
    };
  }, []);
}
