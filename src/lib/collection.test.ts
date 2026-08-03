import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fromSaved,
  loadRequests,
  migrateLegacyCollection,
  toSaved,
} from "./collection";
import { newDraft, uid } from "./draft";
import type { SavedRequest } from "./api";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("draft <-> SavedRequest mapping", () => {
  it("folds enabled rows to tuples and drops disabled rows", () => {
    const draft = newDraft("Users");
    draft.method = "POST";
    draft.url = "https://x.dev/users";
    draft.headers = [
      { id: uid(), key: "Accept", value: "application/json", enabled: true },
      { id: uid(), key: "X-Debug", value: "1", enabled: false },
      { id: uid(), key: "", value: "", enabled: true },
    ];
    draft.params = [
      { id: uid(), key: "page", value: "2", enabled: true },
      { id: uid(), key: "limit", value: "5", enabled: false },
    ];
    draft.body = "{}";

    const saved = toSaved(draft);
    expect(saved.headers).toEqual([["Accept", "application/json"]]);
    expect(saved.url).toBe("https://x.dev/users?page=2");
    expect(saved.body).toBe("{}");
    expect(saved.graphql).toBeNull();
    expect(saved.grpc).toBeNull();
  });

  it("maps blank body to null", () => {
    const draft = newDraft();
    draft.body = "   ";
    expect(toSaved(draft).body).toBeNull();
  });

  it("roundtrips url and params including {{var}} tokens", () => {
    const draft = newDraft("Params roundtrip");
    draft.url = "https://x.dev/search";
    draft.params = [
      { id: uid(), key: "q", value: "{{term}} extra", enabled: true },
      { id: uid(), key: "page", value: "2", enabled: true },
      { id: uid(), key: "", value: "", enabled: true },
    ];

    const saved = toSaved(draft);
    expect(saved.url).toBe("https://x.dev/search?q={{term}}%20extra&page=2");

    const back = fromSaved(saved);
    expect(back.url).toBe("https://x.dev/search");
    expect(back.params.map((r) => [r.key, r.value])).toEqual([
      ["q", "{{term}} extra"],
      ["page", "2"],
      ["", ""],
    ]);
  });

  it("throws a readable error on an unknown kind", () => {
    const saved = { ...toSaved(newDraft("Broken")), kind: "soap" as never };
    expect(() => fromSaved(saved)).toThrow(
      'Request "Broken" has an unknown kind "soap"',
    );
  });

  it("roundtrips an http request with auth", () => {
    const draft = newDraft("Auth roundtrip");
    draft.method = "PUT";
    draft.url = "https://x.dev/a";
    draft.auth = {
      type: "apikey",
      token: "",
      username: "",
      password: "",
      key: "X-Key",
      value: "s3cret",
      placement: "query",
    };
    draft.headers = [
      { id: uid(), key: "A", value: "1", enabled: true },
      { id: uid(), key: "", value: "", enabled: true },
    ];

    const back = fromSaved(toSaved(draft));
    expect(back.id).toBe(draft.id);
    expect(back.name).toBe("Auth roundtrip");
    expect(back.kind).toBe("http");
    expect(back.method).toBe("PUT");
    expect(back.url).toBe("https://x.dev/a");
    expect(back.auth.type).toBe("apikey");
    expect(back.auth.key).toBe("X-Key");
    expect(back.auth.value).toBe("s3cret");
    expect(back.auth.placement).toBe("query");
    expect(back.headers.map((r) => [r.key, r.value, r.enabled])).toEqual([
      ["A", "1", true],
      ["", "", true],
    ]);
  });

  it("roundtrips basic auth", () => {
    const draft = newDraft();
    draft.auth = { ...draft.auth, type: "basic", username: "u", password: "p" };
    const back = fromSaved(toSaved(draft));
    expect(back.auth.type).toBe("basic");
    expect(back.auth.username).toBe("u");
    expect(back.auth.password).toBe("p");
  });

  it("roundtrips a graphql request", () => {
    const draft = newDraft("GQL");
    draft.kind = "graphql";
    draft.url = "https://x.dev/graphql";
    draft.graphqlQuery = "{ me { id } }";
    draft.graphqlVariables = '{"a":1}';

    const saved = toSaved(draft);
    expect(saved.graphql).toEqual({
      query: "{ me { id } }",
      variables: '{"a":1}',
    });
    const back = fromSaved(saved);
    expect(back.kind).toBe("graphql");
    expect(back.graphqlQuery).toBe("{ me { id } }");
    expect(back.graphqlVariables).toBe('{"a":1}');
  });

  it("roundtrips a grpc request", () => {
    const draft = newDraft("Echo");
    draft.kind = "grpc";
    draft.url = "http://localhost:50051";
    draft.grpc = {
      protoPaths: "/a/echo.proto\n/b/other.proto\n",
      service: "echo.Echo",
      method: "Say",
      message: '{"text":"hi"}',
      metadata: [
        { id: uid(), key: "x-trace", value: "t1", enabled: true },
        { id: uid(), key: "x-off", value: "no", enabled: false },
      ],
    };

    const saved = toSaved(draft);
    expect(saved.grpc).toEqual({
      protoPaths: ["/a/echo.proto", "/b/other.proto"],
      service: "echo.Echo",
      method: "Say",
      message: '{"text":"hi"}',
      metadata: [["x-trace", "t1"]],
    });

    const back = fromSaved(saved);
    expect(back.grpc.protoPaths).toBe("/a/echo.proto\n/b/other.proto");
    expect(back.grpc.service).toBe("echo.Echo");
    expect(back.grpc.metadata.map((r) => [r.key, r.value])).toEqual([
      ["x-trace", "t1"],
      ["", ""],
    ]);
  });
});

describe("workspace-backed collection", () => {
  beforeEach(() => {
    invoke.mockReset();
    localStorage.clear();
  });

  it("loads requests via list_requests and maps them to drafts", async () => {
    const saved: SavedRequest = {
      id: "abc123",
      name: "Listed",
      kind: "http",
      method: "GET",
      url: "https://x.dev",
      headers: [["A", "1"]],
      body: null,
      auth: { type: "none" },
      graphql: null,
      grpc: null,
    };
    invoke.mockResolvedValueOnce({ items: [saved], skipped: [] });

    const { requests: drafts, warnings } = await loadRequests("/ws");
    expect(invoke).toHaveBeenCalledWith("list_requests", { workspace: "/ws" });
    expect(warnings).toEqual([]);
    expect(drafts).toHaveLength(1);
    expect(drafts[0].id).toBe("abc123");
    expect(drafts[0].name).toBe("Listed");
    expect(drafts[0].headers.map((r) => r.key)).toEqual(["A", ""]);
  });

  it("skips requests with an unknown kind and reports a warning", async () => {
    const good = toSaved(newDraft("Good"));
    const bad = { ...toSaved(newDraft("Bad")), kind: "soap" };
    invoke.mockResolvedValueOnce({ items: [good, bad], skipped: [] });

    const { requests: drafts, warnings } = await loadRequests("/ws");
    expect(drafts).toHaveLength(1);
    expect(drafts[0].name).toBe("Good");
    expect(warnings).toEqual([
      'Request "Bad" has an unknown kind "soap"',
    ]);
  });

  it("merges unparseable files reported by the backend into the warnings", async () => {
    const good = toSaved(newDraft("Good"));
    const bad = { ...toSaved(newDraft("Bad")), kind: "soap" };
    invoke.mockResolvedValueOnce({
      items: [good, bad],
      skipped: ["/ws/requests/broken.toml: expected an equals"],
    });

    const { requests: drafts, warnings } = await loadRequests("/ws");
    expect(drafts.map((d) => d.name)).toEqual(["Good"]);
    expect(warnings).toEqual([
      "/ws/requests/broken.toml: expected an equals",
      'Request "Bad" has an unknown kind "soap"',
    ]);
  });

  it("migrates the legacy localStorage collection then removes the key", async () => {
    const a = newDraft("Legacy A");
    const b = newDraft("Legacy B");
    localStorage.setItem(
      "mandalo.collection.v1",
      JSON.stringify({ requests: [a, b], activeId: b.id }),
    );
    invoke.mockResolvedValue("saved");

    const migration = await migrateLegacyCollection("/ws");
    expect(migration?.activeId).toBe(b.id);
    expect(migration?.skipped).toEqual([]);
    expect(localStorage.getItem("mandalo.collection.v1")).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenNthCalledWith(1, "save_request", {
      workspace: "/ws",
      request: toSaved(a),
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "save_request", {
      workspace: "/ws",
      request: toSaved(b),
    });
  });

  it("is a no-op without a legacy key", async () => {
    expect(await migrateLegacyCollection("/ws")).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("discards an unparseable legacy blob", async () => {
    localStorage.setItem("mandalo.collection.v1", "{broken");
    expect(await migrateLegacyCollection("/ws")).toBeNull();
    expect(localStorage.getItem("mandalo.collection.v1")).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("salvages valid legacy entries, skips malformed ones, and removes the key", async () => {
    const good = newDraft("Legacy good");
    localStorage.setItem(
      "mandalo.collection.v1",
      JSON.stringify({
        requests: [good, { name: "Mangled" }, "garbage", null],
        activeId: good.id,
      }),
    );
    invoke.mockResolvedValue("saved");

    const migration = await migrateLegacyCollection("/ws");
    expect(migration?.activeId).toBe(good.id);
    expect(migration?.skipped).toEqual(["Mangled", "unnamed request", "unnamed request"]);
    expect(localStorage.getItem("mandalo.collection.v1")).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("save_request", {
      workspace: "/ws",
      request: toSaved(good),
    });
  });

  it("removes the legacy key even when a save fails mid-migration", async () => {
    localStorage.setItem(
      "mandalo.collection.v1",
      JSON.stringify({ requests: [newDraft("Doomed")], activeId: null }),
    );
    invoke.mockRejectedValue("disk full");

    await expect(migrateLegacyCollection("/ws")).rejects.toBe("disk full");
    expect(localStorage.getItem("mandalo.collection.v1")).toBeNull();
  });
});
