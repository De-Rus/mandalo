import { create } from "zustand";

const KEY = "mandalo.tabs.v1";

interface TabsState {
  openIds: string[];
  dirtyIds: string[];
  open: (id: string) => void;
  close: (id: string, activeId: string | null) => string | null;
  closeMany: (ids: string[], activeId: string | null) => string | null;
  closeAll: () => void;
  closeOthers: (id: string) => void;
  markDirty: (id: string) => void;
  markClean: (id: string) => void;
  prune: (existing: string[]) => void;
}

function read(): string[] {
  const raw = localStorage.getItem(KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed) ? parsed.filter((v) => typeof v === "string") : [];
  } catch {
    return [];
  }
}

function persist(openIds: string[]): void {
  localStorage.setItem(KEY, JSON.stringify(openIds));
}

export const useTabs = create<TabsState>((set, get) => ({
  openIds: read(),
  dirtyIds: [],
  open: (id) => {
    if (get().openIds.includes(id)) return;
    const openIds = [...get().openIds, id];
    persist(openIds);
    set({ openIds });
  },
  close: (id, activeId) => {
    const { openIds } = get();
    const index = openIds.indexOf(id);
    const next = openIds.filter((v) => v !== id);
    persist(next);
    set({ openIds: next, dirtyIds: get().dirtyIds.filter((v) => v !== id) });
    if (activeId !== id) return activeId;
    if (next.length === 0) return null;
    return next[Math.min(index, next.length - 1)];
  },
  closeMany: (ids, activeId) => {
    const closing = new Set(ids);
    const { openIds } = get();
    const next = openIds.filter((id) => !closing.has(id));
    persist(next);
    set({ openIds: next, dirtyIds: get().dirtyIds.filter((id) => !closing.has(id)) });
    if (activeId === null || !closing.has(activeId)) return activeId;
    if (next.length === 0) return null;
    const from = openIds.indexOf(activeId);
    return (
      openIds.slice(from + 1).find((id) => !closing.has(id)) ??
      next[next.length - 1]
    );
  },
  closeAll: () => {
    persist([]);
    set({ openIds: [], dirtyIds: [] });
  },
  closeOthers: (id) => {
    const openIds = get().openIds.includes(id) ? [id] : [];
    persist(openIds);
    set({ openIds, dirtyIds: get().dirtyIds.filter((v) => v === id) });
  },
  markDirty: (id) => {
    if (get().dirtyIds.includes(id)) return;
    set({ dirtyIds: [...get().dirtyIds, id] });
  },
  markClean: (id) => {
    if (!get().dirtyIds.includes(id)) return;
    set({ dirtyIds: get().dirtyIds.filter((v) => v !== id) });
  },
  prune: (existing) => {
    const openIds = get().openIds.filter((id) => existing.includes(id));
    if (openIds.length === get().openIds.length) return;
    persist(openIds);
    set({ openIds });
  },
}));
