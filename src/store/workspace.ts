import { create } from "zustand";
import {
  createWorkspace,
  errorMessage,
  openWorkspace,
  removeWorkspace,
  setActiveWorkspace,
  type WorkspaceInfo,
} from "../lib/api";
import { listWorkspaces } from "../lib/backend";

interface WorkspaceState {
  items: WorkspaceInfo[];
  activeId: string | null;
  error: string | null;
  load: () => Promise<string | null>;
  open: (path: string) => Promise<{ path: string; migrated: string[] } | null>;
  create: (path: string, name: string) => Promise<string | null>;
  select: (id: string) => Promise<string | null>;
  remove: (id: string) => Promise<string | null>;
}

function pathOf(items: WorkspaceInfo[], id: string | null): string | null {
  return items.find((w) => w.id === id)?.path ?? null;
}

export const useWorkspaces = create<WorkspaceState>((set, get) => ({
  items: [],
  activeId: null,
  error: null,
  load: async () => {
    try {
      const { items, active } = await listWorkspaces();
      set({ items, activeId: active, error: null });
      return pathOf(items, active);
    } catch (e) {
      set({ error: errorMessage(e) });
      return null;
    }
  },
  open: async (path) => {
    const { workspace, migrated } = await openWorkspace(path);
    await setActiveWorkspace(workspace.id);
    await get().load();
    return { path: workspace.path, migrated };
  },
  create: async (path, name) => {
    const workspace = await createWorkspace(path, name);
    await setActiveWorkspace(workspace.id);
    await get().load();
    return workspace.path;
  },
  select: async (id) => {
    await setActiveWorkspace(id);
    return get().load();
  },
  remove: async (id) => {
    await removeWorkspace(id);
    return get().load();
  },
}));

export function useActiveWorkspace(): WorkspaceInfo | null {
  return useWorkspaces((s) => s.items.find((w) => w.id === s.activeId) ?? null);
}
