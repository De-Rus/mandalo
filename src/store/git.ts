import { create } from "zustand";
import { errorMessage, gitInit, gitStatus, type SyncStatus } from "../lib/api";
import { currentHost } from "../lib/host";

interface GitState {
  status: SyncStatus | null;
  error: string | null;
  busy: boolean;
  refresh: (workspace: string | null) => Promise<void>;
  initRepo: (workspace: string, remoteUrl?: string | null) => Promise<void>;
}

export const useGit = create<GitState>((set, get) => ({
  status: null,
  error: null,
  busy: false,
  refresh: async (workspace) => {
    if (currentHost() !== "desktop" || !workspace) {
      set({ status: null, error: null });
      return;
    }
    try {
      const status = await gitStatus(workspace);
      set({ status, error: null });
    } catch (e) {
      set({ status: null, error: errorMessage(e) });
    }
  },
  initRepo: async (workspace, remoteUrl = null) => {
    if (get().busy) return;
    set({ busy: true });
    try {
      await gitInit(workspace, remoteUrl);
      await get().refresh(workspace);
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    } finally {
      set({ busy: false });
    }
  },
}));
