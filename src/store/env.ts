import { create } from "zustand";
import {
  defaultWorkspaceDir,
  deleteEnvironment,
  errorMessage,
  listEnvironments,
  saveEnvironment,
  type Environment,
} from "../lib/api";

const SELECTED_KEY = "mandalo.env.selected";

function skippedLine(skipped: string[]): string | null {
  if (skipped.length === 0) return null;
  return `Skipped ${skipped.length} unreadable environment file(s): ${skipped.join("; ")}`;
}

interface EnvState {
  workspace: string | null;
  envs: Environment[];
  selected: string | null;
  error: string | null;
  init: () => Promise<void>;
  select: (name: string | null) => void;
  save: (env: Environment) => Promise<void>;
  remove: (name: string) => Promise<void>;
  dismissError: () => void;
}

export const useEnv = create<EnvState>((set, get) => ({
  workspace: null,
  envs: [],
  selected: localStorage.getItem(SELECTED_KEY),
  error: null,
  init: async () => {
    try {
      const workspace = await defaultWorkspaceDir();
      const { items: envs, skipped } = await listEnvironments(workspace);
      set((s) => ({
        workspace,
        envs,
        error: skippedLine(skipped),
        selected: envs.some((e) => e.name === s.selected) ? s.selected : null,
      }));
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },
  select: (name) => {
    if (name) localStorage.setItem(SELECTED_KEY, name);
    else localStorage.removeItem(SELECTED_KEY);
    set({ selected: name });
  },
  save: async (env) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await saveEnvironment(workspace, env);
      const { items: envs, skipped } = await listEnvironments(workspace);
      set({ envs, error: skippedLine(skipped) });
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  remove: async (name) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await deleteEnvironment(workspace, name);
      const { items: envs, skipped } = await listEnvironments(workspace);
      set((s) => ({
        envs,
        error: skippedLine(skipped),
        selected: s.selected === name ? null : s.selected,
      }));
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
    if (get().selected === null) localStorage.removeItem(SELECTED_KEY);
  },
  dismissError: () => set({ error: null }),
}));

export function useActiveVars(): Record<string, string> {
  return useEnv(
    (s) => s.envs.find((e) => e.name === s.selected)?.vars ?? EMPTY_VARS,
  );
}

const EMPTY_VARS: Record<string, string> = {};
