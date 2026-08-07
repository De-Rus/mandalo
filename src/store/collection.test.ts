import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SavedRequest, Tree } from "../lib/api";
import { toSaved } from "../lib/collection";
import { newDraft } from "../lib/draft";
import { flushPendingSaves, useCollection } from "./collection";
import { useSession } from "./session";
import { useTabs } from "./tabs";
import { useToasts } from "./toast";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const SAVED: SavedRequest = {
  ...toSaved(newDraft("List users")),
  id: "r1",
  method: "GET",
  url: "https://api.dev/users",
};

const TREE: Tree = {
  collections: [
    {
      id: "c1",
      slug: "acme",
      name: "Acme API",
      folders: [
        {
          name: "Users",
          path: "users",
          folders: [],
          requests: [
            {
              id: "r1",
              name: "List users",
              kind: "http",
              method: "GET",
              path: "users/list-users.toml",
            },
          ],
        },
      ],
      requests: [],
    },
  ],
  skipped: [],
};

function mockBackend(overrides: Record<string, unknown> = {}): void {
  invoke.mockImplementation((cmd: string) => {
    if (cmd in overrides) return Promise.resolve(overrides[cmd]);
    if (cmd === "list_workspaces")
      return Promise.resolve({
        items: [{ id: "w1", path: "/ws", name: "ws" }],
        active: "w1",
      });
    if (cmd === "list_tree") return Promise.resolve(TREE);
    if (cmd === "load_request") return Promise.resolve(SAVED);
    if (cmd === "save_request")
      return Promise.resolve({ path: "users/list-users.toml" });
    return Promise.resolve(undefined);
  });
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

describe("collection store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invoke.mockReset();
    localStorage.clear();
    mockBackend();
    useTabs.setState({ openIds: [], dirtyIds: [] });
    useSession.setState({ responses: {} });
    useToasts.setState({ items: [] });
    useCollection.setState({
      workspace: null,
      tree: { collections: [], skipped: [] },
      drafts: {},
      activeId: null,
      loadingId: null,
      error: null,
      warning: null,
      initStarted: false,
    });
  });

  afterEach(async () => {
    await flushPendingSaves();
    vi.useRealTimers();
  });

  it("init loads the workspace tree and opens the first request", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();

    const s = useCollection.getState();
    expect(s.workspace).toBe("/ws");
    expect(s.tree.collections[0].name).toBe("Acme API");
    expect(s.activeId).toBe("r1");
    expect(s.drafts.r1.name).toBe("List users");
    expect(useTabs.getState().openIds).toEqual(["r1"]);
    expect(invoke).toHaveBeenCalledWith("load_request", {
      workspace: "/ws",
      collection: "acme",
      path: "users/list-users.toml",
    });
  });

  it("runs init only once even when invoked twice", async () => {
    await Promise.all([
      useCollection.getState().init(),
      useCollection.getState().init(),
    ]);
    expect(invoke.mock.calls.filter((c) => c[0] === "list_tree")).toHaveLength(1);
  });

  it("surfaces skipped files from the tree as a warning", async () => {
    mockBackend({
      list_tree: { collections: TREE.collections, skipped: ["/ws/a.toml: bad"] },
    });
    await useCollection.getState().init();
    await flushMicrotasks();
    expect(useCollection.getState().warning).toContain("a.toml");
    expect(useCollection.getState().error).toBeNull();
  });

  it("debounces edits into one save_request call and clears the dirty flag", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    useCollection.getState().updateActive({ url: "https://x" });
    useCollection.getState().updateActive({ url: "https://x.d" });
    useCollection.getState().updateActive({ url: "https://x.dev" });
    expect(useTabs.getState().dirtyIds).toEqual(["r1"]);
    expect(
      invoke.mock.calls.filter((c) => c[0] === "save_request"),
    ).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();

    const saves = invoke.mock.calls.filter((c) => c[0] === "save_request");
    expect(saves).toHaveLength(1);
    expect(saves[0][1]).toMatchObject({
      workspace: "/ws",
      collection: "acme",
      path: "users/list-users.toml",
      folder: "users",
    });
    expect(useTabs.getState().dirtyIds).toEqual([]);
  });

  it("creates a request in the given collection and folder", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    useCollection.getState().addRequest("graphql", "acme", "users");
    const id = useCollection.getState().activeId as string;
    expect(useCollection.getState().drafts[id].kind).toBe("graphql");
    await flushMicrotasks();

    const create = invoke.mock.calls.find((c) => c[0] === "save_request");
    expect(create?.[1]).toMatchObject({
      collection: "acme",
      path: null,
      folder: "users",
    });
  });

  it("creates a Scratch collection on the first request of an empty workspace", async () => {
    let tree: Tree = { collections: [], skipped: [] };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_workspaces")
        return Promise.resolve({
          items: [{ id: "w1", path: "/ws", name: "ws" }],
          active: "w1",
        });
      if (cmd === "list_tree") return Promise.resolve(tree);
      if (cmd === "create_collection") {
        tree = TREE;
        return Promise.resolve({ id: "c1", slug: "acme", name: "Scratch" });
      }
      if (cmd === "save_request")
        return Promise.resolve({ path: "new-request.toml" });
      return Promise.resolve(undefined);
    });

    await useCollection.getState().init();
    await flushMicrotasks();
    expect(useCollection.getState().activeId).toBeNull();

    useCollection.getState().addRequest();
    await flushMicrotasks();

    expect(invoke).toHaveBeenCalledWith("create_collection", {
      workspace: "/ws",
      name: "Scratch",
    });
    expect(useToasts.getState().items.map((t) => t.text)).toEqual([
      'Created collection "Scratch"',
    ]);
    const id = useCollection.getState().activeId as string;
    expect(id).toBeTruthy();
    expect(useCollection.getState().drafts[id].collection).toBe("acme");
    expect(useTabs.getState().openIds).toEqual([id]);
    expect(useCollection.getState().error).toBeNull();
    const save = invoke.mock.calls.find((c) => c[0] === "save_request");
    expect(save?.[1]).toMatchObject({ collection: "acme", path: null, folder: "" });
  });

  it("fails loud when a request is added with no workspace at all", () => {
    useCollection.getState().addRequest();
    expect(useCollection.getState().error).toBe(
      "Open a workspace before adding requests",
    );
    expect(invoke).not.toHaveBeenCalledWith("create_collection", expect.anything());
  });

  it("duplicates a request into the same folder and opens the copy", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    await useCollection.getState().duplicateRequest("r1");
    await flushMicrotasks();

    const save = invoke.mock.calls.find((c) => c[0] === "save_request");
    expect(save?.[1]).toMatchObject({
      workspace: "/ws",
      collection: "acme",
      path: null,
      folder: "users",
    });
    const request = (save?.[1] as { request: SavedRequest }).request;
    expect(request.name).toBe("List users copy");
    expect(request.id).not.toBe("r1");

    const s = useCollection.getState();
    expect(s.activeId).toBe(request.id);
    expect(s.drafts[request.id].name).toBe("List users copy");
    expect(s.drafts[request.id].url).toBe("https://api.dev/users");
    expect(useTabs.getState().openIds).toContain(request.id);
  });

  it("flushes a pending edit before duplicating so the copy carries it", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    useCollection.getState().updateActive({ url: "https://api.dev/people" });
    const duplicating = useCollection.getState().duplicateRequest("r1");
    await flushMicrotasks();
    await duplicating;

    const saves = invoke.mock.calls.filter((c) => c[0] === "save_request");
    expect(saves[0][1]).toMatchObject({ path: "users/list-users.toml" });
    expect(
      (saves[0][1] as { request: SavedRequest }).request.url,
    ).toBe("https://api.dev/people");
    expect(useTabs.getState().dirtyIds).toEqual([]);
  });

  it("surfaces a duplicate failure through the store error", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();

    invoke.mockImplementationOnce(() => Promise.reject("unreadable"));
    await useCollection.getState().duplicateRequest("r1");
    expect(useCollection.getState().error).toBe("unreadable");
  });

  it("moves a request to another folder and keeps it open", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();
    mockBackend({ move_request: { path: "admin/list-users.toml" } });

    await useCollection.getState().moveRequest("r1", "admin");
    await flushMicrotasks();

    expect(invoke).toHaveBeenCalledWith("move_request", {
      workspace: "/ws",
      collection: "acme",
      from: "users/list-users.toml",
      toFolder: "admin",
    });
    expect(useCollection.getState().drafts.r1.path).toBe("admin/list-users.toml");
    expect(useTabs.getState().openIds).toEqual(["r1"]);
    expect(invoke.mock.calls.map((c) => c[0])).toContain("list_tree");
  });

  it("surfaces a move failure through the store error", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "move_request") return Promise.reject("target exists");
      return Promise.resolve(TREE);
    });

    await useCollection.getState().moveRequest("r1", "admin");
    await flushMicrotasks();
    expect(useCollection.getState().error).toBe("target exists");
  });

  it("refuses to move a request that has never been saved", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();

    await useCollection.getState().moveRequest("nope", "admin");
    expect(useCollection.getState().error).toBe(
      "This request has no saved file yet, so it cannot be moved",
    );
  });

  it("deletes a request through its collection path and drops the tab", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    useCollection.getState().deleteRequest("r1");
    await flushMicrotasks();

    expect(useCollection.getState().drafts.r1).toBeUndefined();
    expect(useTabs.getState().openIds).toEqual([]);
    expect(invoke).toHaveBeenCalledWith("delete_request", {
      workspace: "/ws",
      collection: "acme",
      path: "users/list-users.toml",
    });
  });

  it("cancels a pending save when the request is deleted", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    useCollection.getState().updateActive({ url: "https://gone" });
    useCollection.getState().deleteRequest("r1");
    await vi.advanceTimersByTimeAsync(600);
    await flushMicrotasks();

    const calls = invoke.mock.calls.map((c) => c[0]);
    expect(calls).toContain("delete_request");
    expect(calls).not.toContain("save_request");
  });

  it("evicts the session response when a request is deleted", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    useSession.setState({
      responses: { r1: { phase: "error", message: "boom", logs: [] } },
    });

    useCollection.getState().deleteRequest("r1");
    expect(useSession.getState().responses.r1).toBeUndefined();
  });

  it("surfaces persistence failures as store errors and clears them on success", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();

    invoke.mockImplementationOnce(() => Promise.reject("disk full"));
    useCollection.getState().updateActive({ url: "https://x.dev" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(useCollection.getState().error).toBe("disk full");

    mockBackend();
    useCollection.getState().updateActive({ url: "https://x.dev/ok" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(useCollection.getState().error).toBeNull();
  });

  it("serializes a delete behind an in-flight save for the same id", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();

    let resolveSave!: (value: unknown) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "save_request")
        return new Promise((r) => {
          resolveSave = r;
        });
      if (cmd === "list_tree") return Promise.resolve(TREE);
      return Promise.resolve(undefined);
    });

    useCollection.getState().updateActive({ url: "https://slow" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).toContain("save_request");
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("delete_request");

    useCollection.getState().deleteRequest("r1");
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("delete_request");

    resolveSave({ path: "users/list-users.toml" });
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).toContain("delete_request");
  });

  it("remembers the active request id across restarts", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    expect(localStorage.getItem("mandalo.collection.active")).toBe("r1");
  });

  it("creates a collection and refreshes the tree", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    await useCollection.getState().createCollection("Billing");

    expect(invoke).toHaveBeenCalledWith("create_collection", {
      workspace: "/ws",
      name: "Billing",
    });
    expect(invoke.mock.calls.map((c) => c[0])).toContain("list_tree");
  });

  it("creates a folder through the collection command", async () => {
    await useCollection.getState().init();
    await flushMicrotasks();
    invoke.mockClear();

    await useCollection.getState().createFolder("acme", "users/admin");

    expect(invoke).toHaveBeenCalledWith("create_folder", {
      workspace: "/ws",
      collection: "acme",
      path: "users/admin",
    });
  });
});
