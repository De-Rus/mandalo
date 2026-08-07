import { useMemo } from "react";
import { create } from "zustand";
import {
  bindSecretHost,
  clearSecret,
  deleteEnvironment,
  deleteVar,
  errorMessage,
  setSecret,
  type Environment,
  type EnvironmentView,
  type VarInfo,
} from "../lib/api";
import { listEnvironments, saveEnvironment } from "../lib/backend";

const SELECTED_KEY = "mandalo.env.selected";

function skippedLine(skipped: string[]): string | null {
  if (skipped.length === 0) return null;
  return `Skipped ${skipped.length} unreadable environment file(s): ${skipped.join("; ")}`;
}

function fail(set: (partial: { error: string }) => void, message: string): never {
  set({ error: message });
  throw new Error(message);
}

export function plainVars(
  env: EnvironmentView | null | undefined,
): Record<string, string> {
  const vars: Record<string, string> = {};
  if (!env) return vars;
  for (const [key, info] of Object.entries(env.vars)) {
    if (!info.secret && info.value !== null) vars[key] = info.value;
  }
  return vars;
}

export function secretNames(env: EnvironmentView | null | undefined): string[] {
  if (!env) return [];
  return Object.entries(env.vars)
    .filter(([, info]) => info.secret)
    .map(([key]) => key);
}

export function varLabel(info: VarInfo): string {
  if (!info.secret) return info.value ?? "";
  return info.set ? "••••••••" : "not set on this machine";
}

interface EnvState {
  workspace: string | null;
  envs: EnvironmentView[];
  selected: string | null;
  error: string | null;
  init: (workspace: string) => Promise<void>;
  refresh: () => Promise<void>;
  select: (name: string | null) => void;
  save: (env: Environment) => Promise<void>;
  createEnv: (name: string, vars?: Record<string, string>) => Promise<void>;
  addVar: (env: string, key: string, value: string) => Promise<void>;
  remove: (name: string) => Promise<void>;
  storeSecret: (env: string, key: string, value: string) => Promise<void>;
  forgetSecret: (env: string, key: string) => Promise<void>;
  bindHost: (env: string, key: string, host: string) => Promise<void>;
  removeVar: (env: string, key: string) => Promise<void>;
  applyVarWrites: (
    sets: Record<string, string>,
    unsets: string[],
  ) => Promise<void>;
  dismissError: () => void;
}

export const useEnv = create<EnvState>((set, get) => ({
  workspace: null,
  envs: [],
  selected: localStorage.getItem(SELECTED_KEY),
  error: null,
  init: async (workspace) => {
    try {
      const { items: envs, skipped } = await listEnvironments(workspace);
      set((s) => ({
        workspace,
        envs,
        error: skippedLine(skipped),
        selected: envs.some((e) => e.name === s.selected) ? s.selected : null,
      }));
    } catch (e) {
      set({ workspace, error: errorMessage(e) });
    }
  },
  refresh: async () => {
    const { workspace } = get();
    if (!workspace) return;
    try {
      const { items: envs, skipped } = await listEnvironments(workspace);
      set({ envs, error: skippedLine(skipped) });
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
      await get().refresh();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  createEnv: async (name, vars = {}) => {
    const trimmed = name.trim();
    if (trimmed === "") fail(set, "Environment name is required");
    if (get().envs.some((e) => e.name === trimmed))
      fail(set, `Environment ${trimmed} already exists`);
    await get().save({ name: trimmed, vars });
    get().select(trimmed);
  },
  addVar: async (envName, key, value) => {
    const name = key.trim();
    if (name === "") fail(set, "Variable name is required");
    const current = get().envs.find((e) => e.name === envName);
    if (!current) fail(set, `Environment ${envName} is not loaded`);
    const info = current.vars[name];
    if (info && info.secret)
      fail(
        set,
        `${envName}.${name} is declared secret — its value never goes in the environment file. Use Set value in the environment editor.`,
      );
    if (info && !info.shared)
      fail(
        set,
        `${envName}.${name} is declared local — its value lives on this machine only. Set it in the environment editor.`,
      );
    await get().save({
      name: envName,
      vars: { ...plainVars(current), [name]: value },
    });
  },
  remove: async (name) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await deleteEnvironment(workspace, name);
      await get().refresh();
      set((s) => ({ selected: s.selected === name ? null : s.selected }));
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
    if (get().selected === null) localStorage.removeItem(SELECTED_KEY);
  },
  storeSecret: async (env, key, value) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await setSecret(workspace, env, key, value);
      await get().refresh();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  forgetSecret: async (env, key) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await clearSecret(workspace, env, key);
      await get().refresh();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  bindHost: async (env, key, host) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await bindSecretHost(workspace, env, key, host);
      await get().refresh();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  removeVar: async (env, key) => {
    const { workspace } = get();
    if (!workspace) throw new Error("Workspace not loaded yet");
    try {
      await deleteVar(workspace, env, key);
      await get().refresh();
    } catch (e) {
      set({ error: errorMessage(e) });
      throw e;
    }
  },
  applyVarWrites: async (sets, unsets) => {
    const { selected, envs } = get();
    if (!selected) return;
    if (Object.keys(sets).length === 0 && unsets.length === 0) return;
    const current = envs.find((e) => e.name === selected);
    if (!current) return;
    const secret = new Set(secretNames(current));
    const refused = Object.keys(sets).filter((key) => secret.has(key));
    if (refused.length > 0) {
      set({
        error: `${selected}.${refused[0]} is declared secret — a script cannot write its value into the environment file. Use Set value in the environment editor.`,
      });
      return;
    }
    const vars = { ...plainVars(current), ...sets };
    for (const key of unsets) delete vars[key];
    await get().save({ name: selected, vars });
  },
  dismissError: () => set({ error: null }),
}));

const EMPTY_VARS: Record<string, string> = {};

export function useActiveEnv(): EnvironmentView | null {
  return useEnv((s) => s.envs.find((e) => e.name === s.selected) ?? null);
}

export function useActiveVars(): Record<string, string> {
  const env = useActiveEnv();
  return useMemo(() => (env ? plainVars(env) : EMPTY_VARS), [env]);
}
