import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SavedRequest, Tree } from "../lib/api";
import { toSaved } from "../lib/collection";
import { newDraft } from "../lib/draft";
import { useCollection } from "./collection";
import { useTabs } from "./tabs";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const PATH = "users/list-users.toml";

const TREE: Tree = {
  collections: [
    {
      id: "c1",
      slug: "acme",
      name: "Acme API",
      folders: [],
      requests: [
        { id: "r1", name: "List users", kind: "http", method: "GET", path: PATH },
      ],
    },
  ],
  skipped: [],
};

const EMPTY_TREE: Tree = {
  collections: [{ ...TREE.collections[0], requests: [] }],
  skipped: [],
};

function saved(name: string): SavedRequest {
  return { ...toSaved(newDraft(name)), id: "r1", method: "GET", url: "https://api.dev" };
}

function mount(dirty: boolean, tree: Tree = TREE): void {
  useTabs.setState({ openIds: ["r1"], dirtyIds: dirty ? ["r1"] : [] });
  useCollection.setState({
    workspace: "/ws",
    tree,
    drafts: { r1: { ...newDraft("Mine"), id: "r1", collection: "acme", path: PATH } },
    activeId: "r1",
    conflicts: [],
    vanished: [],
  });
}

describe("a change made in another tab", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "load_request") return Promise.resolve(saved("Theirs"));
      if (cmd === "list_tree") return Promise.resolve(TREE);
      return Promise.resolve(undefined);
    });
  });

  it("appears here silently when this tab has nothing unsaved", async () => {
    mount(false);

    await useCollection.getState().applyRemoteRequest("acme", PATH);

    expect(useCollection.getState().drafts.r1.name).toBe("Theirs");
    expect(useCollection.getState().conflicts).toEqual([]);
  });

  it("never overwrites unsaved work — it asks instead", async () => {
    mount(true);

    await useCollection.getState().applyRemoteRequest("acme", PATH);

    expect(useCollection.getState().drafts.r1.name).toBe("Mine");
    expect(useCollection.getState().conflicts).toEqual(["r1"]);
  });

  it("loads their version when the user chooses to", async () => {
    mount(true);
    await useCollection.getState().applyRemoteRequest("acme", PATH);

    await useCollection.getState().takeTheirs("r1");

    expect(useCollection.getState().drafts.r1.name).toBe("Theirs");
    expect(useCollection.getState().conflicts).toEqual([]);
    expect(useTabs.getState().dirtyIds).toEqual([]);
  });

  it("keeps the user's version when they choose that, still dirty", async () => {
    mount(true);
    await useCollection.getState().applyRemoteRequest("acme", PATH);

    useCollection.getState().keepMine("r1");

    expect(useCollection.getState().drafts.r1.name).toBe("Mine");
    expect(useCollection.getState().conflicts).toEqual([]);
    expect(useTabs.getState().dirtyIds).toEqual(["r1"]);
  });

  it("ignores a file this tab is not showing", async () => {
    mount(false);

    await useCollection.getState().applyRemoteRequest("acme", "other/thing.toml");

    expect(useCollection.getState().drafts.r1.name).toBe("Mine");
    expect(invoke).not.toHaveBeenCalledWith("load_request", expect.anything());
  });

  it("flags an open request that another tab deleted", async () => {
    mount(false, TREE);
    invoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_tree" ? EMPTY_TREE : undefined),
    );

    await useCollection.getState().applyRemoteTree();

    expect(useCollection.getState().vanished).toEqual(["r1"]);
  });

  it("keeps the deleted request's tab open so the user can rescue their copy", async () => {
    mount(false, TREE);
    useTabs.setState({ openIds: ["r1", "gone"], dirtyIds: [] });
    invoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_tree" ? EMPTY_TREE : undefined),
    );

    await useCollection.getState().applyRemoteTree();

    expect(useTabs.getState().openIds).toEqual(["r1"]);
    expect(useCollection.getState().vanished).toEqual(["r1"]);
  });

  it("shows a request created in another tab without stealing focus", async () => {
    useTabs.setState({ openIds: [], dirtyIds: [] });
    useCollection.setState({
      workspace: "/ws",
      tree: EMPTY_TREE,
      drafts: {},
      activeId: null,
      conflicts: [],
      vanished: [],
    });

    await useCollection.getState().applyRemoteTree();

    expect(useCollection.getState().tree.collections[0].requests).toHaveLength(1);
    expect(useTabs.getState().openIds).toEqual([]);
    expect(useCollection.getState().activeId).toBeNull();
  });
});
