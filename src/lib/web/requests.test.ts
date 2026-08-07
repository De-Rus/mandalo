import { describe, expect, it } from "vitest";
import { addSampleCollection } from "./mounts";
import { seedFiles } from "./seed";
import type { RequestSummary } from "../api";
import { MemoryVfs } from "./vfs.testkit";
import {
  deleteRequest,
  listTree,
  loadRequest,
  moveRequest,
  saveRequest,
} from "./workspace";

/**
 * The browser reads and writes the very same files the CLI does. Every fixture here is
 * `.http`/`.grpc` on purpose: a browser that only understood the old TOML requests saw
 * an empty collection and said nothing about it.
 */
async function seeded(): Promise<MemoryVfs> {
  const vfs = new MemoryVfs();
  for (const [path, text] of seedFiles()) await vfs.write(path, text);
  return vfs;
}

async function withCollection(files: Record<string, string>): Promise<MemoryVfs> {
  const vfs = new MemoryVfs();
  await vfs.write("mandalo.toml", 'schema_version = 1\nid = "w"\nname = "W"\n');
  await vfs.write(
    "collections/api/collection.toml",
    'schema_version = 1\nid = "api"\nname = "API"\n',
  );
  for (const [name, text] of Object.entries(files))
    await vfs.write(`collections/api/${name}`, text);
  return vfs;
}

const TWO_BLOCKS = `### Ping
GET https://api.dev/ping
Accept: application/json

### Pong
POST https://api.dev/pong

{"a": 1}
`;

/** The sample collection files live in folders, so its requests are not at the root. */
function flattenRequests(node: {
  requests: RequestSummary[];
  folders: { requests: RequestSummary[]; folders: unknown[] }[];
}): RequestSummary[] {
  const out = [...node.requests];
  for (const folder of node.folders) {
    out.push(...flattenRequests(folder as Parameters<typeof flattenRequests>[0]));
  }
  return out;
}

describe("the seeded sample workspace", () => {
  it("ships the .http and .grpc files the CLI reads", () => {
    const names = seedFiles().map(([path]) => path);
    expect(names).toContain("collections/mock/http/echo.http");
    expect(names).toContain("collections/mock/grpc/service.grpc");
    expect(names).toContain("mandalo.toml");
  });

  it("loads as a collection with requests in it, not an empty tree", async () => {
    const tree = await listTree(await seeded());

    expect(tree.skipped).toEqual([
      "collections/mock/streams/chat.ws: .ws requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet",
      "collections/mock/streams/sensors.mqtt: .mqtt requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet",
    ]);
    expect(tree.collections.length).toBe(1);
    expect(tree.collections[0]!.folders.map((f) => f.name)).toContain("auth");
    const requests = flattenRequests(tree.collections[0]!);
    expect(requests.length).toBeGreaterThan(5);
    expect(requests.every((request) => /#\d+$/.test(request.path))).toBe(true);
    expect(requests.map((request) => request.kind)).toContain("grpc");
    expect(requests.map((request) => request.kind)).toContain("graphql");
  });

  it("loads a request off the sample workspace with its file vars applied", async () => {
    const vfs = await seeded();
    const tree = await listTree(vfs);
    const summary = flattenRequests(tree.collections[0]!).find((r) => r.kind === "http");
    const request = await loadRequest(vfs, "mock", summary!.path);

    expect(request.name).toBe(summary!.name);
    expect(request.method).toBe(summary!.method);
    expect(request.url).not.toBe("");
  });
});

describe("a TOML request is reported, never silently dropped", () => {
  it("says the file has to be converted, the way the CLI says it", async () => {
    const vfs = await withCollection({ "ping.toml": 'id = "p"\nname = "Ping"\n' });

    const tree = await listTree(vfs);

    expect(tree.collections[0]!.requests).toEqual([]);
    expect(tree.skipped).toEqual([
      "collections/api/ping.toml: requests are .http and .grpc files now — this TOML request has to be converted",
    ]);
  });

  it("reports a request file it cannot parse instead of hiding it", async () => {
    const vfs = await withCollection({ "bad.http": "### R\nFETCH https://a.dev/x\n" });

    const tree = await listTree(vfs);

    expect(tree.collections[0]!.requests).toEqual([]);
    expect(tree.skipped[0]).toMatch(/bad\.http: line 2: "FETCH" is not an HTTP method/);
  });
});

describe("addressing a block", () => {
  it("refuses a bare path into a file that holds more than one request", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });

    await expect(loadRequest(vfs, "api", "two.http")).rejects.toThrow(
      /two\.http holds 2 requests — address one of them as two\.http#0/,
    );
  });

  it("matches a block by name and refuses a name nothing carries", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });

    expect((await loadRequest(vfs, "api", "two.http#Pong")).method).toBe("POST");
    await expect(loadRequest(vfs, "api", "two.http#Nope")).rejects.toThrow(
      /no request named "Nope" in this file/,
    );
  });
});

describe("saving", () => {
  it("writes a new request as .http and reads it back unchanged", async () => {
    const vfs = await withCollection({});

    const path = await saveRequest(vfs, "api", null, "", {
      id: "x",
      name: "Create user",
      kind: "http",
      method: "POST",
      url: "https://api.dev/users",
      headers: [["Accept", "application/json"]],
      auth: { type: "bearer", token: "{{token}}" },
      body: '{"name": "nova"}',
      scripts: { pre: null, post: 'pm.test("ok", function () {})' },
    });

    expect(path).toBe("create-user.http#0");
    expect(await vfs.read("collections/api/create-user.http")).toBe(
      `### Create user
POST https://api.dev/users
Authorization: Bearer {{token}}
Accept: application/json

{"name": "nova"}

> {%
pm.test("ok", function () {})
%}
`,
    );
    const back = await loadRequest(vfs, "api", path);
    expect(back).toMatchObject({
      name: "Create user",
      method: "POST",
      url: "https://api.dev/users",
      headers: [["Accept", "application/json"]],
      auth: { type: "bearer", token: "{{token}}" },
      body: '{"name": "nova"}',
    });
  });

  it("rewrites one block and leaves every other byte of the file alone", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });
    const request = await loadRequest(vfs, "api", "two.http#0");

    const path = await saveRequest(vfs, "api", "two.http#0", null, {
      ...request,
      name: "Ping v2",
      url: "https://api.dev/ping/v2",
    });

    expect(path).toBe("two.http#0");
    const raw = (await vfs.read("collections/api/two.http")) as string;
    expect(raw).toContain("### Ping v2");
    expect(raw).toContain("https://api.dev/ping/v2");
    expect(raw.slice(raw.indexOf("### Pong"))).toBe(TWO_BLOCKS.slice(TWO_BLOCKS.indexOf("### Pong")));
  });

  it("refuses to save a gRPC request into a .http file", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });
    const request = await loadRequest(vfs, "api", "two.http#0");

    await expect(
      saveRequest(vfs, "api", "two.http#0", null, {
        ...request,
        kind: "grpc",
        grpc: { protoPaths: [], service: "S", method: "M", message: "{}", metadata: [] },
      }),
    ).rejects.toThrow(/holds http requests, so a grpc request cannot be saved into it/);
  });

  it("refuses to write declarative tests into a file that cannot hold them", async () => {
    const vfs = await withCollection({});

    await expect(
      saveRequest(vfs, "api", null, "", {
        id: "x",
        name: "Asserted",
        kind: "http",
        method: "GET",
        url: "https://api.dev/x",
        headers: [],
        auth: { type: "none" },
        tests: [{ kind: "status", op: "eq", value: 200 }],
      }),
    ).rejects.toThrow(/cannot carry declarative tests or captures/);
  });

  it("round-trips a gRPC request through .grpc", async () => {
    const vfs = await withCollection({});

    const path = await saveRequest(vfs, "api", null, "", {
      id: "g",
      name: "Say",
      kind: "grpc",
      method: "POST",
      url: "localhost:50051",
      headers: [],
      auth: { type: "none" },
      grpc: {
        protoPaths: ["protos/mock.proto"],
        service: "mock.v1.Mock",
        method: "Say",
        message: '{"text": "hola"}',
        metadata: [["x-trace", "1"]],
      },
    });

    expect(path).toBe("say.grpc#0");
    expect(await vfs.read("collections/api/say.grpc")).toBe(
      `### Say
localhost:50051/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: 1

{"text": "hola"}
`,
    );
    expect((await loadRequest(vfs, "api", path)).grpc).toEqual({
      protoPaths: ["protos/mock.proto"],
      service: "mock.v1.Mock",
      method: "Say",
      message: '{"text": "hola"}',
      metadata: [["x-trace", "1"]],
    });
  });
});

describe("deleting and moving", () => {
  it("cuts one block out and keeps the rest of the file", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });

    await deleteRequest(vfs, "api", "two.http#0");

    const raw = await vfs.read("collections/api/two.http");
    expect(raw).not.toContain("### Ping");
    expect(raw).toContain("### Pong");
  });

  it("removes the file when the last block goes", async () => {
    const vfs = await withCollection({ "one.http": "### Only\nGET https://a.dev/x\n" });

    await deleteRequest(vfs, "api", "one.http#0");

    expect(await vfs.read("collections/api/one.http")).toBeNull();
  });

  it("moves a block into its own file in the target folder", async () => {
    const vfs = await withCollection({ "two.http": TWO_BLOCKS });
    await vfs.mkdirp("collections/api/deep");

    const path = await moveRequest(vfs, "api", "two.http#1", "deep");

    expect(path).toBe("deep/pong.http#0");
    expect(await vfs.read("collections/api/deep/pong.http")).toContain("### Pong");
    expect(await vfs.read("collections/api/two.http")).not.toContain("### Pong");
  });
});

describe("the sample collection can always be brought back", () => {
  it("adds it to an empty workspace and the tree can send from it", async () => {
    const vfs = new MemoryVfs();

    const slug = await addSampleCollection(vfs);

    expect(slug).toBe("mock");
    const tree = await listTree(vfs);
    expect(tree.skipped).toEqual([
      "collections/mock/streams/chat.ws: .ws requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet",
      "collections/mock/streams/sensors.mqtt: .mqtt requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet",
    ]);
    const collection = tree.collections.find((c) => c.slug === "mock");
    const requests = flattenRequests(collection!);
    expect(requests.length).toBeGreaterThan(5);
    const request = await loadRequest(vfs, "mock", requests[0]!.path);
    expect(request.url).not.toBe("");
    expect(await vfs.read("protos/mock.proto")).not.toBeNull();
  });

  it("adds a second copy rather than overwriting the user's own work", async () => {
    const vfs = new MemoryVfs();
    await addSampleCollection(vfs);
    await vfs.write("collections/mock/mine.http", "### Mine\nGET https://mine.dev/x\n");

    const slug = await addSampleCollection(vfs);

    expect(slug).toBe("mock-2");
    expect(await vfs.read("collections/mock/mine.http")).toContain("### Mine");
    expect((await listTree(vfs)).collections.map((c) => c.slug).sort()).toEqual([
      "mock",
      "mock-2",
    ]);
  });

  it("keeps an environment the workspace already has", async () => {
    const vfs = new MemoryVfs();
    await vfs.write("environments/local.toml", 'name = "local"\n\n[vars]\nbaseUrl = "http://mine"\n');

    await addSampleCollection(vfs);

    expect(await vfs.read("environments/local.toml")).toContain("http://mine");
  });
});

describe("a request format only the engine reads", () => {
  it("is reported as skipped, never silently omitted", async () => {
    const vfs = await withCollection({ "chat.ws": "### Echo\nwss://a.dev/echo\n" });

    const tree = await listTree(vfs);

    expect(tree.collections[0]!.requests).toEqual([]);
    expect(tree.skipped).toEqual([
      "collections/api/chat.ws: .ws requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet",
    ]);
  });
});
