import { create } from "zustand";
import {
  createCollection as createCollectionCmd,
  createFolder as createFolderCmd,
  deleteCollection as deleteCollectionCmd,
  deleteFolder as deleteFolderCmd,
  errorMessage,
  moveRequest as moveRequestCmd,
  reorderRequests as reorderRequestsCmd,
  renameCollection as renameCollectionCmd,
  renameFolder as renameFolderCmd,
  seedWorkspace,
  type Kind,
  type Tree,
} from "../lib/api";
import {
  deleteRequest as deleteRequestCmd,
  listTree,
  loadRequest,
  saveRequest,
  basename,
} from "../lib/backend";
import { ACTIVE_KEY, fromSaved, toSaved } from "../lib/collection";
import { newDraft, uid, type RequestDraft } from "../lib/draft";
import { folderOf, locateRequests } from "../lib/tree";
import { useSession } from "./session";
import { useTabs } from "./tabs";
import { toast } from "./toast";
import { useGit } from "./git";
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
  saveError: string | null;
  conflicts: string[];
  vanished: string[];
  initStarted: boolean;
  init: () => Promise<void>;
  refreshTree: () => Promise<void>;
  applyRemoteRequest: (collection: string, path: string) => Promise<void>;
  applyRemoteTree: () => Promise<void>;
  takeTheirs: (id: string) => Promise<void>;
  keepMine: (id: string) => void;
  switchWorkspace: (path: string) => Promise<void>;
  /** If the workspace has no collections yet, create one so the sidebar is usable. */
  ensureStarterCollection: (name?: string) => Promise<void>;
  addRequest: (kind?: Kind, collection?: string, folder?: string) => void;
  openRequest: (id: string) => void;
  renameRequest: (id: string, name: string) => void;
  deleteRequest: (id: string) => void;
  duplicateRequest: (id: string) => Promise<void>;
  moveRequest: (id: string, target: string) => Promise<void>;
  reorderRequests: (collection: string, folder: string, ordered: string[]) => Promise<void>;
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
    useCollection.setState({ saveError: null });
    await useCollection.getState().refreshTree();
    void useGit.getState().refresh(useCollection.getState().workspace);
  } catch (e) {
    useCollection.setState({ saveError: errorMessage(e) });
    fail(e);
  }
}

export function retryPendingSave(): Promise<void> {
  const { activeId } = useCollection.getState();
  return activeId ? persistNow(activeId) : Promise.resolve();
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

/** WHY: moving or copying a file must carry the edits the debounce still holds. */
async function flushSave(id: string): Promise<void> {
  if (!saveTimers.has(id) && !useTabs.getState().dirtyIds.includes(id)) return;
  cancelSave(id);
  await persistNow(id);
}

export async function flushPendingSaves(): Promise<void> {
  const ids = [...saveTimers.keys()];
  for (const id of ids) cancelSave(id);
  await Promise.all(ids.map(persistNow));
}

function rememberActive(id: string | null): void {
  if (id) {
    sessionStorage.setItem(ACTIVE_KEY, id);
    localStorage.setItem(ACTIVE_KEY, id);
  } else {
    sessionStorage.removeItem(ACTIVE_KEY);
    localStorage.removeItem(ACTIVE_KEY);
  }
}

function skippedLine(skipped: string[]): string | null {
  if (skipped.length === 0) return null;
  return `Skipped ${skipped.length} unreadable file(s): ${skipped.join("; ")}`;
}

export function locationOf(id: string): Location | null {
  return locateRequests(useCollection.getState().tree.collections).get(id) ?? null;
}

async function reloadDraft(id: string, location: Location): Promise<void> {
  const { workspace } = useCollection.getState();
  if (!workspace) return;
  const saved = await loadRequest(workspace, location.collection, location.path);
  useCollection.setState((s) =>
    s.drafts[id]
      ? {
          drafts: {
            ...s.drafts,
            [id]: fromSaved(saved, location.collection, location.path),
          },
        }
      : s,
  );
  useTabs.getState().markClean(id);
}

function idsAt(collection: string, path: string): string[] {
  const out: string[] = [];
  for (const [id, location] of locateRequests(
    useCollection.getState().tree.collections,
  ))
    if (location.collection === collection && location.path === path) out.push(id);
  return out;
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

const SCRATCH_COLLECTION = "Scratch";

function startRequest(
  workspace: string,
  slug: string,
  folder: string,
  kind: Kind,
): void {
  const draft = newDraft("New Request", kind, slug);
  useCollection.setState((s) => ({
    drafts: { ...s.drafts, [draft.id]: draft },
    activeId: draft.id,
  }));
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
      useCollection.setState((s) => ({
        drafts: s.drafts[draft.id]
          ? { ...s.drafts, [draft.id]: { ...s.drafts[draft.id], path } }
          : s.drafts,
      }));
      ok();
      await useCollection.getState().refreshTree();
    } catch (e) {
      fail(e);
    }
  });
}

export const useCollection = create<CollectionState>((set, get) => ({
  workspace: null,
  tree: EMPTY_TREE,
  drafts: {},
  activeId: null,
  loadingId: null,
  error: null,
  warning: null,
  saveError: null,
  conflicts: [],
  vanished: [],
  initStarted: false,
  init: async () => {
    if (get().initStarted) return;
    set({ initStarted: true });
    try {
      const workspace = await useWorkspaces.getState().load();
      if (!workspace) throw new Error("No workspace available");
      set({ workspace });
      await get().ensureStarterCollection();
      const tree = await listTree(workspace);
      set({ tree, warning: skippedLine(tree.skipped) });
      const ids = [...locateRequests(tree.collections).keys()];
      useTabs.getState().prune(ids);
      const remembered =
        sessionStorage.getItem(ACTIVE_KEY) ?? localStorage.getItem(ACTIVE_KEY);
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
  applyRemoteRequest: async (collection, path) => {
    const dirty = new Set(useTabs.getState().dirtyIds);
    const { drafts } = get();
    const conflicted: string[] = [];
    for (const id of idsAt(collection, path)) {
      if (!drafts[id]) continue;
      if (dirty.has(id)) {
        conflicted.push(id);
        continue;
      }
      try {
        await reloadDraft(id, { collection, path });
      } catch (e) {
        fail(e);
      }
    }
    if (conflicted.length > 0)
      set((s) => ({ conflicts: [...new Set([...s.conflicts, ...conflicted])] }));
  },
  applyRemoteTree: async () => {
    await get().refreshTree();
    const ids = new Set(locateRequests(get().tree.collections).keys());
    const open = useTabs.getState().openIds;
    const vanished = open.filter((id) => !ids.has(id) && get().drafts[id]);
    const stale = open.filter((id) => !ids.has(id) && !get().drafts[id]);
    if (stale.length > 0) useTabs.getState().prune([...ids, ...vanished]);
    set((s) => ({
      vanished: [...new Set([...s.vanished, ...vanished])],
    }));
  },
  takeTheirs: async (id) => {
    const location = locationOf(id);
    set((s) => ({ conflicts: s.conflicts.filter((v) => v !== id) }));
    if (!location) return;
    try {
      await reloadDraft(id, location);
    } catch (e) {
      fail(e);
    }
  },
  keepMine: (id) => {
    set((s) => ({ conflicts: s.conflicts.filter((v) => v !== id) }));
  },
  switchWorkspace: async (path) => {
    await flushPendingSaves();
    useTabs.getState().closeAll();
    set({ workspace: path, tree: EMPTY_TREE, drafts: {}, activeId: null });
    try {
      await get().ensureStarterCollection(basename(path));
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
  ensureStarterCollection: async (name) => {
    const { workspace } = get();
    if (!workspace) return;
    try {
      const seeded = await seedWorkspace(workspace);
      if (seeded) {
        toast("success", seeded.summary);
        return;
      }
      const label = name?.trim();
      if (!label) return;
      const tree = await listTree(workspace);
      if (locateRequests(tree.collections).size > 0) return;
      if (tree.collections.length > 0) return;
      await get().createCollection(label);
    } catch (e) {
      fail(e);
    }
  },
  addRequest: (kind = "http", collection, folder = "") => {
    const { workspace, tree } = get();
    if (!workspace) {
      fail(new Error("Open a workspace before adding requests"));
      return;
    }
    const slug = collection ?? tree.collections[0]?.slug;
    if (slug) {
      startRequest(workspace, slug, folder, kind);
      return;
    }
    void (async () => {
      try {
        await get().createCollection(SCRATCH_COLLECTION);
      } catch (e) {
        fail(e);
        return;
      }
      const created = get().tree.collections[0]?.slug;
      if (!created) {
        fail(new Error(`Created "${SCRATCH_COLLECTION}" but it is not readable`));
        return;
      }
      ok();
      toast("success", `Created collection "${SCRATCH_COLLECTION}"`);
      startRequest(workspace, created, folder, kind);
    })();
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
  duplicateRequest: async (id) => {
    const { workspace } = get();
    if (!workspace) {
      fail(new Error("Open a workspace before duplicating requests"));
      return;
    }
    await flushSave(id);
    const location = locationOf(id);
    if (!location) {
      fail(new Error("This request has no saved file yet, so it cannot be duplicated"));
      return;
    }
    try {
      const saved = await loadRequest(
        workspace,
        location.collection,
        location.path,
      );
      const copy = { ...saved, id: uid(), name: `${saved.name} copy` };
      const { path } = await saveRequest(
        workspace,
        location.collection,
        null,
        folderOf(location.path),
        copy,
      );
      set((s) => ({
        drafts: {
          ...s.drafts,
          [copy.id]: fromSaved(copy, location.collection, path),
        },
        activeId: copy.id,
      }));
      rememberActive(copy.id);
      useTabs.getState().open(copy.id);
      ok();
      await get().refreshTree();
    } catch (e) {
      fail(e);
    }
  },
  reorderRequests: async (collection, folder, ordered) => {
    const { workspace } = get();
    if (!workspace) {
      fail(new Error("Open a workspace before reordering requests"));
      return;
    }
    try {
      await reorderRequestsCmd(workspace, collection, folder, ordered);
      ok();
      await get().refreshTree();
    } catch (e) {
      fail(e);
      throw e;
    }
  },
  moveRequest: async (id, target) => {
    const { workspace } = get();
    if (!workspace) {
      fail(new Error("Open a workspace before moving requests"));
      return;
    }
    await flushSave(id);
    const location = locationOf(id);
    if (!location) {
      fail(new Error("This request has no saved file yet, so it cannot be moved"));
      return;
    }
    await enqueue(id, async () => {
      try {
        const { path } = await moveRequestCmd(
          workspace,
          location.collection,
          location.path,
          target,
        );
        set((s) => ({
          drafts: s.drafts[id]
            ? { ...s.drafts, [id]: { ...s.drafts[id], path } }
            : s.drafts,
        }));
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
