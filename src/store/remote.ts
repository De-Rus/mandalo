import { create } from "zustand";
import { remoteOrigin, type RemoteOrigin } from "../lib/api";

interface RemoteState {
  origin: RemoteOrigin | null;
  dialogOpen: boolean;
  prefill: string;
  open: (prefill?: string) => void;
  close: () => void;
  refresh: (workspace: string | null) => Promise<void>;
}

/**
 * Whether the workspace on screen is somebody else's. Every write the shell
 * offers is gated on this, and the engine refuses one anyway — this is what
 * stops the UI from offering an action that can only fail.
 */
export const useRemote = create<RemoteState>((set) => ({
  origin: null,
  dialogOpen: false,
  prefill: "",
  open: (prefill) => set({ dialogOpen: true, prefill: prefill ?? "" }),
  close: () => set({ dialogOpen: false, prefill: "" }),
  refresh: async (workspace) => {
    if (workspace === null) {
      set({ origin: null });
      return;
    }
    try {
      set({ origin: await remoteOrigin(workspace) });
    } catch {
      set({ origin: null });
    }
  },
}));

export function useIsReadOnly(): boolean {
  return useRemote((s) => s.origin !== null);
}
