import { create } from "zustand";
import {
  createCollection as createCollectionCmd,
  createFolder as createFolderCmd,
  deleteCollection as deleteCollectionCmd,
  deleteFolder as deleteFolderCmd,
  errorMessage,
  renameCollection as renameCollectionCmd,
  renameFolder as renameFolderCmd,
  type Kind,
  type Tree,
} from "../lib/api";
import {
  deleteRequest as deleteRequestCmd,
  listTree,
  loadRequest,
  saveRequest,
} from "../lib/backend";
import { ACTIVE_KEY, fromSaved, toSaved } from "../lib/collection";
import { newDraft, type RequestDraft } from "../lib/draft";
import { folderOf, locateRequests } from "../lib/tree";
import { useSession } from "./session";
import { useTabs } from "./tabs";
import { useWorkspaces } from "./workspace";

const SAVE_DELAY_MS = 500;
const saveTimers = new Map<string, ReturnType<typeof setTimeout>>();
const persistChains = new Map<string, Promise<void>>();

const EMPTY_TREE: Tree = { collections: [], skipped: [] };

export interface Location {
  collection: string;
  path: string;
}

interface CollectionState {
  workspace: string | null;
  tree: Tree;
  drafts: Record<string, RequestDraft>;
  activeId: string | null;
  loadingId: string | null;
  error: string | null;
  warning: string | null;
  initStarted: boolean;
  init: () => Promise<void>;
  refreshTree: () => Promise<void>;
  switchWorkspace: (path: string) => Promise<void>;
  addRequest: (kind?: Kind, collection?: string, folder?: string) => void;
  openRequest: (id: string) => void;
  renameRequest: (id: string, name: string) => void;
  deleteRequest: (id: string) => void;
  updateActive: (patch: Partial<RequestDraft>) => void;
  saveActiveNow: () => Promise<void>;
  createCollection: (name: string) => Promise<void>;
  renameCollection: (slug: string, name: string) => Promise<void>;
  deleteCollection: (slug: string) => Promise<void>;
  createFolder: (collection: string, path: string) => Promise<void>;
  renameFolder: (collection: string, path: string, name: string) => Promise<void>;
  deleteFolder: (collection: string, path: string) => Promise<void>;
}

function fail(e: unknown): void {
  useCollection.setState({ error: errorMessage(e) });
}

function ok(): void {
  if (useCollection.getState().error) useCollection.setState({ error: null });
}

function enqueue(id: string, task: () => Promise<void>): Promise<void> {
  const chain = (persistChains.get(id) ?? Promise.resolve()).then(task);
  persistChains.set(id, chain);
  void chain.finally(() => {
    if (persistChains.get(id) === chain) persistChains.delete(id);
  });
  return chain;
}

async function doPersist(id: string): Promise<void> {
  const { workspace, drafts } = useCollection.getState();
  const draft = drafts[id];
  if (!workspace || !draft) return;
  try {
    const { path } = await saveRequest(
      workspace,
      draft.collection,
      draft.path,
      draft.path === null ? "" : folderOf(draft.path),
      toSaved(draft),
    );
    const moved = path !== draft.path;
    if (moved)
      useCollection.setState((s) => ({
        drafts: s.drafts[id]
          ? { ...s.drafts, [id]: { ...s.drafts[id], path } }
          : s.drafts,
      }));
    useTabs.getState().markClean(id);
    ok();
    await useCollection.getState().refreshTree();
  } catch (e) {
    fail(e);
  }
}

function persistNow(id: string): Promise<void> {
  return enqueue(id, () => doPersist(id));
}

function scheduleSave(id: string): void {
  const timer = saveTimers.get(id);
  if (timer) clearTimeout(timer);
  saveTimers.set(
    id,
    setTimeout(() => {
      saveTimers.delete(id);
      void persistNow(id);
    }, SAVE_DELAY_MS),
  );
}

function cancelSave(id: string): void {
  const timer = saveTimers.get(id);
  if (timer) clearTimeout(timer);
  saveTimers.delete(id);
}

export async function flushPendingSaves(): Promise<void> {
  const ids = [...saveTimers.keys()];
  for (const id of ids) cancelSave(id);
  await Promise.all(ids.map(persistNow));
}

function rememberActive(id: string | null): void {
  if (id) localStorage.setItem(ACTIVE_KEY, id);
  else localStorage.removeItem(ACTIVE_KEY);
}

function skippedLine(skipped: string[]): string | null {
  if (skipped.length === 0) return null;
  return `Skipped ${skipped.length} unreadable file(s): ${skipped.join("; ")}`;
}

export function locationOf(id: string): Location | null {
  return locateRequests(useCollection.getState().tree.collections).get(id) ?? null;
}

async function ensureDraft(id: string): Promise<void> {
  const state = useCollection.getState();
  if (state.drafts[id] || !state.workspace) return;
  const location = locationOf(id);
  if (!location) return;
  useCollection.setState({ loadingId: id });
  try {
    const saved = await loadRequest(
      state.workspace,
      location.collection,
      location.path,
    );
    const draft = fromSaved(saved, location.collection, location.path);
    useCollection.setState((s) => ({
      drafts: { ...s.drafts, [id]: draft },
      loadingId: s.loadingId === id ? null : s.loadingId,
    }));
    ok();
  } catch (e) {
    useCollection.setState((s) => ({
      loadingId: s.loadingId === id ? null : s.loadingId,
    }));
    fail(e);
  }
}

export const useCollection = create<CollectionState>((set, get) => ({
  workspace: null,
  tree: EMPTY_TREE,
  drafts: {},
  activeId: null,
  loadingId: null,
  error: null,
  warning: null,
  initStarted: false,
  init: async () => {
    if (get().initStarted) return;
    set({ initStarted: true });
    try {
      const workspace = await useWorkspaces.getState().load();
      if (!workspace) throw new Error("No workspace available");
      const tree = await listTree(workspace);
      set({ workspace, tree, warning: skippedLine(tree.skipped) });
      const ids = [...locateRequests(tree.collections).keys()];
      useTabs.getState().prune(ids);
      const remembered = localStorage.getItem(ACTIVE_KEY);
      const activeId = remembered && ids.includes(remembered)
        ? remembered
        : (useTabs.getState().openIds[0] ?? ids[0] ?? null);
      if (activeId) {
        useTabs.getState().open(activeId);
        set({ activeId });
        rememberActive(activeId);
        await Promise.all(useTabs.getState().openIds.map(ensureDraft));
      }
    } catch (e) {
      set({ initStarted: false });
      fail(e);
    }
  },
  refreshTree: async () => {
    const { workspace } = get();
    if (!workspace) return;
    try {
      const tree = await listTree(workspace);
      set({ tree, warning: skippedLine(tree.skipped) });
    } catch (e) {
      fail(e);
    }
  },
  switchWorkspace: async (path) => {
    await flushPendingSaves();
    useTabs.getState().closeAll();
    set({ workspace: path, tree: EMPTY_TREE, drafts: {}, activeId: null });
    try {
      const tree = await listTree(path);
      set({ tree, warning: skippedLine(tree.skipped) });
      const first = [...locateRequests(tree.collections).keys()][0] ?? null;
      rememberActive(first);
      if (first) {
        useTabs.getState().open(first);
        set({ activeId: first });
        await ensureDraft(first);
      }
    } catch (e) {
      fail(e);
    }
  },
  addRequest: (kind = "http", collection, folder = "") => {
    const { workspace, tree } = get();
    const slug = collection ?? tree.collections[0]?.slug;
    if (!workspace || !slug) {
      fail(new Error("Create a collection before adding requests"));
      return;
    }
    const draft = newDraft("New Request", kind, slug);
    set((s) => ({ drafts: { ...s.drafts, [draft.id]: draft }, activeId: draft.id }));
    rememberActive(draft.id);
    useTabs.getState().open(draft.id);
    void enqueue(draft.id, async () => {
      try {
        const { path } = await saveRequest(
          workspace,
          slug,
          null,
          folder,
          toSaved(draft),
        );
        set((s) => ({
          drafts: s.drafts[draft.id]
            ? { ...s.drafts, [draft.id]: { ...s.drafts[draft.id], path } }
            : s.drafts,
        }));
        ok();
        await get().refreshTree();
      } catch (e) {
        fail(e);
      }
    });
  },
  openRequest: (id) => {
    void flushPendingSaves();
    rememberActive(id);
    useTabs.getState().open(id);
    set({ activeId: id });
    void ensureDraft(id);
  },
  renameRequest: (id, name) => {
    const trimmed = name.trim();
    if (trimmed === "") return;
    void (async () => {
      await ensureDraft(id);
      set((s) =>
        s.drafts[id]
          ? { drafts: { ...s.drafts, [id]: { ...s.drafts[id], name: trimmed } } }
          : s,
      );
      useTabs.getState().markDirty(id);
      await persistNow(id);
    })();
  },
  deleteRequest: (id) => {
    cancelSave(id);
    const location = locationOf(id);
    const next = useTabs.getState().close(id, get().activeId);
    set((s) => {
      const drafts = { ...s.drafts };
      delete drafts[id];
      const activeId = s.activeId === id ? next : s.activeId;
      rememberActive(activeId);
      return { drafts, activeId };
    });
    useSession.getState().evictResponse(id);
    const nextId = get().activeId;
    if (nextId) void ensureDraft(nextId);
    const { workspace } = get();
    if (!workspace || !location) return;
    void enqueue(id, async () => {
      try {
        await deleteRequestCmd(workspace, location.collection, location.path);
        ok();
        await get().refreshTree();
      } catch (e) {
        fail(e);
      }
    });
  },
  updateActive: (patch) => {
    const id = get().activeId;
    if (!id || !get().drafts[id]) return;
    set((s) => ({
      drafts: { ...s.drafts, [id]: { ...s.drafts[id], ...patch } },
    }));
    useTabs.getState().markDirty(id);
    scheduleSave(id);
  },
  saveActiveNow: async () => {
    const id = get().activeId;
    if (!id) return;
    cancelSave(id);
    await persistNow(id);
  },
  createCollection: async (name) => {
    const { workspace } = get();
    if (!workspace) return;
    await createCollectionCmd(workspace, name);
    await get().refreshTree();
  },
  renameCollection: async (slug, name) => {
    const { workspace } = get();
    if (!workspace) return;
    await renameCollectionCmd(workspace, slug, name);
    await get().refreshTree();
  },
  deleteCollection: async (slug) => {
    const { workspace } = get();
    if (!workspace) return;
    await deleteCollectionCmd(workspace, slug);
    await get().refreshTree();
  },
  createFolder: async (collection, path) => {
    const { workspace } = get();
    if (!workspace) return;
    await createFolderCmd(workspace, collection, path);
    await get().refreshTree();
  },
  renameFolder: async (collection, path, name) => {
    const { workspace } = get();
    if (!workspace) return;
    await renameFolderCmd(workspace, collection, path, name);
    await get().refreshTree();
  },
  deleteFolder: async (collection, path) => {
    const { workspace } = get();
    if (!workspace) return;
    await deleteFolderCmd(workspace, collection, path);
    await get().refreshTree();
  },
}));

function flushInBackground(): void {
  void flushPendingSaves();
}

window.addEventListener("beforeunload", flushInBackground);
window.addEventListener("blur", flushInBackground);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") flushInBackground();
});

export function useActiveRequest(): RequestDraft | null {
  return useCollection((s) => (s.activeId ? (s.drafts[s.activeId] ?? null) : null));
}
