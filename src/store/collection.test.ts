import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toSaved } from "../lib/collection";
import { newDraft } from "../lib/draft";
import { flushPendingSaves, useCollection } from "./collection";
import { useSession } from "./session";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe("collection store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invoke.mockReset();
    invoke.mockResolvedValue("saved");
    localStorage.clear();
    const first = newDraft("First");
    useCollection.setState({
      workspace: "/ws",
      requests: [first],
      activeId: first.id,
      error: null,
      initStarted: false,
    });
    useSession.setState({ responses: {} });
  });

  afterEach(async () => {
    await flushPendingSaves();
    vi.useRealTimers();
  });

  it("adds a request, makes it active, and persists it immediately", async () => {
    useCollection.getState().addRequest();
    const s = useCollection.getState();
    expect(s.requests).toHaveLength(2);
    expect(s.activeId).toBe(s.requests[1].id);
    expect(s.requests[1].kind).toBe("http");
    await flushMicrotasks();
    expect(invoke).toHaveBeenCalledWith("save_request", {
      workspace: "/ws",
      request: toSaved(s.requests[1]),
    });
  });

  it("renames a request and ignores blank names", () => {
    const id = useCollection.getState().requests[0].id;
    useCollection.getState().renameRequest(id, "  Users API  ");
    expect(useCollection.getState().requests[0].name).toBe("Users API");
    useCollection.getState().renameRequest(id, "   ");
    expect(useCollection.getState().requests[0].name).toBe("Users API");
  });

  it("deletes a request, moves the active pointer, and deletes the file", async () => {
    useCollection.getState().addRequest();
    await flushMicrotasks();
    const [a, b] = useCollection.getState().requests;
    invoke.mockClear();
    useCollection.getState().deleteRequest(b.id);
    let s = useCollection.getState();
    expect(s.requests.map((r) => r.id)).toEqual([a.id]);
    expect(s.activeId).toBe(a.id);
    await flushMicrotasks();
    expect(invoke).toHaveBeenCalledWith("delete_request", {
      workspace: "/ws",
      id: b.id,
    });
    useCollection.getState().deleteRequest(a.id);
    s = useCollection.getState();
    expect(s.requests).toHaveLength(0);
    expect(s.activeId).toBeNull();
  });

  it("updates only the active request", () => {
    useCollection.getState().addRequest();
    const [a, b] = useCollection.getState().requests;
    useCollection.getState().updateActive({ url: "https://x.dev" });
    const s = useCollection.getState();
    expect(s.requests.find((r) => r.id === b.id)?.url).toBe("https://x.dev");
    expect(s.requests.find((r) => r.id === a.id)?.url).toBe("");
  });

  it("debounces edit saves into one save_request call", async () => {
    useCollection.getState().updateActive({ url: "https://x" });
    useCollection.getState().updateActive({ url: "https://x.d" });
    useCollection.getState().updateActive({ url: "https://x.dev" });
    expect(invoke).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(500);

    expect(invoke).toHaveBeenCalledTimes(1);
    const active = useCollection.getState().requests[0];
    expect(invoke).toHaveBeenCalledWith("save_request", {
      workspace: "/ws",
      request: toSaved({ ...active, url: "https://x.dev" }),
    });
  });

  it("flushes the pending save when switching requests", async () => {
    useCollection.getState().addRequest();
    const [a, b] = useCollection.getState().requests;
    invoke.mockClear();
    useCollection.getState().updateActive({ url: "https://pending" });
    useCollection.getState().openRequest(a.id);
    await flushMicrotasks();

    expect(useCollection.getState().activeId).toBe(a.id);
    expect(invoke).toHaveBeenCalledWith("save_request", {
      workspace: "/ws",
      request: expect.objectContaining({ id: b.id, url: "https://pending" }),
    });
  });

  it("cancels a pending save when the request is deleted", async () => {
    const id = useCollection.getState().requests[0].id;
    useCollection.getState().updateActive({ url: "https://gone" });
    useCollection.getState().deleteRequest(id);
    await vi.advanceTimersByTimeAsync(600);
    await flushMicrotasks();

    const calls = invoke.mock.calls.map((c) => c[0]);
    expect(calls).toContain("delete_request");
    expect(calls).not.toContain("save_request");
  });

  it("remembers the active request id", () => {
    useCollection.getState().addRequest();
    const s = useCollection.getState();
    expect(localStorage.getItem("mandalo.collection.active")).toBe(s.activeId);
  });

  it("init loads the workspace collection and seeds one draft when empty", async () => {
    useCollection.setState({ workspace: null, requests: [], activeId: null });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws2");
      if (cmd === "list_requests")
        return Promise.resolve({ items: [], skipped: [] });
      return Promise.resolve("saved");
    });

    await useCollection.getState().init();

    const s = useCollection.getState();
    expect(s.workspace).toBe("/ws2");
    expect(s.requests).toHaveLength(1);
    expect(s.activeId).toBe(s.requests[0].id);
    expect(invoke).toHaveBeenCalledWith("save_request", {
      workspace: "/ws2",
      request: toSaved(s.requests[0]),
    });
  });

  it("surfaces persistence failures as store errors", async () => {
    invoke.mockRejectedValue("disk full");
    useCollection.getState().updateActive({ url: "https://x.dev" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(useCollection.getState().error).toBe("disk full");
  });

  it("clears a sticky error on the next successful save", async () => {
    invoke.mockRejectedValueOnce("disk full");
    useCollection.getState().updateActive({ url: "https://x.dev" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(useCollection.getState().error).toBe("disk full");

    useCollection.getState().updateActive({ url: "https://x.dev/ok" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(useCollection.getState().error).toBeNull();
  });

  it("serializes a delete behind an in-flight save for the same id", async () => {
    const id = useCollection.getState().requests[0].id;
    let resolveSave!: (value: string) => void;
    invoke.mockImplementation((cmd: string) =>
      cmd === "save_request"
        ? new Promise<string>((r) => {
            resolveSave = r;
          })
        : Promise.resolve(undefined),
    );

    useCollection.getState().updateActive({ url: "https://slow" });
    await vi.advanceTimersByTimeAsync(500);
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["save_request"]);

    useCollection.getState().deleteRequest(id);
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["save_request"]);

    resolveSave("saved");
    await flushMicrotasks();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual([
      "save_request",
      "delete_request",
    ]);
  });

  it("evicts the session response when a request is deleted", () => {
    const id = useCollection.getState().requests[0].id;
    useSession.setState({
      responses: { [id]: { phase: "error", message: "boom" } },
    });
    useCollection.getState().deleteRequest(id);
    expect(useSession.getState().responses[id]).toBeUndefined();
  });

  it("runs init only once even when invoked twice", async () => {
    useCollection.setState({
      workspace: null,
      requests: [],
      activeId: null,
      initStarted: false,
    });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws2");
      if (cmd === "list_requests")
        return Promise.resolve({ items: [], skipped: [] });
      return Promise.resolve("saved");
    });

    await Promise.all([
      useCollection.getState().init(),
      useCollection.getState().init(),
    ]);

    expect(useCollection.getState().requests).toHaveLength(1);
    expect(
      invoke.mock.calls.filter((c) => c[0] === "list_requests"),
    ).toHaveLength(1);
  });

  it("init skips unreadable saved requests and surfaces a warning", async () => {
    useCollection.setState({
      workspace: null,
      requests: [],
      activeId: null,
      initStarted: false,
    });
    const good = toSaved(newDraft("Good"));
    const bad = { ...toSaved(newDraft("Broken")), kind: "soap" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws2");
      if (cmd === "list_requests")
        return Promise.resolve({ items: [good, bad], skipped: [] });
      return Promise.resolve("saved");
    });

    await useCollection.getState().init();

    const s = useCollection.getState();
    expect(s.requests.map((r) => r.name)).toEqual(["Good"]);
    expect(s.error).toContain("Broken");
  });

  it("init surfaces backend-skipped request files as a warning", async () => {
    useCollection.setState({
      workspace: null,
      requests: [],
      activeId: null,
      initStarted: false,
    });
    const good = toSaved(newDraft("Good"));
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws2");
      if (cmd === "list_requests")
        return Promise.resolve({
          items: [good],
          skipped: ["/ws2/requests/broken.toml: expected an equals"],
        });
      return Promise.resolve("saved");
    });

    await useCollection.getState().init();

    const s = useCollection.getState();
    expect(s.requests.map((r) => r.name)).toEqual(["Good"]);
    expect(s.error).toContain("broken.toml");
    expect(s.error).toContain("expected an equals");
  });
});
