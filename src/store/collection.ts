import { create } from "zustand";
import { defaultWorkspaceDir, errorMessage } from "../lib/api";
import {
  ACTIVE_KEY,
  loadRequests,
  migrateLegacyCollection,
  persistRequest,
  removeRequest,
} from "../lib/collection";
import { newDraft, type RequestDraft } from "../lib/draft";
import { useSession } from "./session";

const SAVE_DELAY_MS = 500;
const saveTimers = new Map<string, ReturnType<typeof setTimeout>>();
const persistChains = new Map<string, Promise<void>>();

interface CollectionState {
  workspace: string | null;
  requests: RequestDraft[];
  activeId: string | null;
  error: string | null;
  initStarted: boolean;
  init: () => Promise<void>;
  reload: () => Promise<void>;
  addRequest: () => void;
  openRequest: (id: string) => void;
  renameRequest: (id: string, name: string) => void;
  deleteRequest: (id: string) => void;
  updateActive: (patch: Partial<RequestDraft>) => void;
}

function fail(e: unknown): void {
  useCollection.setState({ error: errorMessage(e) });
}

function clearError(): void {
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
  const { workspace, requests } = useCollection.getState();
  if (!workspace) return;
  const draft = requests.find((r) => r.id === id);
  if (!draft) return;
  try {
    await persistRequest(workspace, draft);
    clearError();
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

function warningLine(warnings: string[]): string | null {
  if (warnings.length === 0) return null;
  return `Skipped ${warnings.length} unreadable request(s): ${warnings.join("; ")}`;
}

export const useCollection = create<CollectionState>((set, get) => ({
  workspace: null,
  requests: [],
  activeId: null,
  error: null,
  initStarted: false,
  init: async () => {
    if (get().initStarted) return;
    set({ initStarted: true });
    try {
      const workspace = await defaultWorkspaceDir();
      const migration = await migrateLegacyCollection(workspace);
      const loaded = await loadRequests(workspace);
      let requests = loaded.requests;
      if (requests.length === 0) {
        const first = newDraft();
        await persistRequest(workspace, first);
        requests = [first];
      }
      const remembered =
        migration?.activeId ?? localStorage.getItem(ACTIVE_KEY);
      const activeId = requests.some((r) => r.id === remembered)
        ? remembered
        : (requests[0]?.id ?? null);
      rememberActive(activeId);
      const warnings = [
        ...(migration?.skipped ?? []).map(
          (name) => `could not migrate "${name}"`,
        ),
        ...loaded.warnings,
      ];
      set({ workspace, requests, activeId, error: warningLine(warnings) });
    } catch (e) {
      set({ initStarted: false });
      fail(e);
    }
  },
  reload: async () => {
    const { workspace } = get();
    if (!workspace) return;
    await flushPendingSaves();
    const { requests, warnings } = await loadRequests(workspace);
    set((s) => {
      const activeId = requests.some((r) => r.id === s.activeId)
        ? s.activeId
        : (requests[0]?.id ?? null);
      rememberActive(activeId);
      return { requests, activeId, error: warningLine(warnings) };
    });
  },
  addRequest: () => {
    const draft = newDraft();
    set((s) => ({ requests: [...s.requests, draft], activeId: draft.id }));
    rememberActive(draft.id);
    const { workspace } = get();
    if (workspace) void persistNow(draft.id);
  },
  openRequest: (id) => {
    void flushPendingSaves();
    rememberActive(id);
    set({ activeId: id });
  },
  renameRequest: (id, name) => {
    set((s) => ({
      requests: s.requests.map((r) =>
        r.id === id ? { ...r, name: name.trim() || r.name } : r,
      ),
    }));
    scheduleSave(id);
  },
  deleteRequest: (id) => {
    cancelSave(id);
    set((s) => {
      const requests = s.requests.filter((r) => r.id !== id);
      const activeId =
        s.activeId === id ? (requests[0]?.id ?? null) : s.activeId;
      rememberActive(activeId);
      return { requests, activeId };
    });
    useSession.getState().evictResponse(id);
    const { workspace } = get();
    if (workspace)
      void enqueue(id, async () => {
        try {
          await removeRequest(workspace, id);
          clearError();
        } catch (e) {
          fail(e);
        }
      });
  },
  updateActive: (patch) => {
    const id = get().activeId;
    if (!id) return;
    set((s) => ({
      requests: s.requests.map((r) => (r.id === id ? { ...r, ...patch } : r)),
    }));
    scheduleSave(id);
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
  return useCollection(
    (s) => s.requests.find((r) => r.id === s.activeId) ?? null,
  );
}
