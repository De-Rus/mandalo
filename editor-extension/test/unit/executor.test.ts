import { createServer, type Server } from "node:http";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { MandaloCli, type SpawnFn } from "../../src/core/cli";
import type { CollectionNode, RequestNode, WorkspaceNode } from "../../src/core/model";
import { MandaloExecutor, suiteRequests, type ExecutionMode } from "../../src/executor";

let server: Server;
let origin: string;
let root: string;

function request(
  name: string,
  file: string,
  index: number,
  kind = "http",
  method = "GET",
): RequestNode {
  return {
    id: `${file}-${index}`,
    name,
    kind,
    method,
    relPath: `${file}#${index}`,
    fsPath: join(root, "collections", "api", ...file.split("/")),
    index,
    line: 0,
  };
}

function collection(): CollectionNode {
  return {
    id: "api",
    slug: "api",
    name: "API",
    dirPath: join(root, "collections", "api"),
    manifestPath: join(root, "collections", "api", "collection.toml"),
    folders: [
      {
        name: "sub",
        relPath: "sub",
        folders: [],
        requests: [request("Nested", "sub/nested.http", 0)],
      },
    ],
    requests: [request("Ping", "ping.http", 0), request("Say", "wire.grpc", 0, "grpc", "GRPC")],
  };
}

function httpOnly(): CollectionNode {
  return { ...collection(), requests: [request("Ping", "ping.http", 0)] };
}

function workspace(): WorkspaceNode {
  return {
    id: "ws",
    name: "WS",
    rootPath: root,
    manifestPath: join(root, "mandalo.toml"),
    collections: [collection()],
    environments: [],
    skipped: [],
  };
}

function build(mode: ExecutionMode, hasCli: boolean, spawnFn: SpawnFn) {
  const log = vi.fn();
  const cli = new MandaloCli({ cliPath: () => "mandalo", spawnFn });
  const executor = new MandaloExecutor({ cli, hasCli: () => hasCli, mode: () => mode, log });
  return { executor, log };
}

const neverSpawn: SpawnFn = async () => {
  throw new Error("the CLI must not be spawned here");
};

const grpcOutcome = {
  path: "wire.grpc#0",
  name: "Say",
  method: "GRPC",
  url: "grpc://x",
  response: null,
  grpc: { body: "{}", durationMs: 1 },
  tests: [],
  captures: [],
  logs: [],
  passed: true,
  durationMs: 1,
  error: null,
  errorCode: null,
};

const cliRun: SpawnFn = async () => ({
  code: 0,
  stdout: JSON.stringify({
    collection: "api",
    env: null,
    total: 1,
    passed: 1,
    failed: 0,
    durationMs: 1,
    requests: [grpcOutcome],
  }),
  stderr: "",
});

const cliSend: SpawnFn = async () => ({
  code: 0,
  stdout: JSON.stringify({ collection: "api", env: null, ...grpcOutcome }),
  stderr: "",
});

function ok(name: string): string {
  return `### ${name}
GET ${origin}/x

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
%}

`;
}

beforeAll(async () => {
  server = createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end('{"ok":true}');
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("no server address");
  origin = `http://127.0.0.1:${address.port}`;

  root = mkdtempSync(join(tmpdir(), "mandalo-exec-"));
  mkdirSync(join(root, "collections", "api", "sub"), { recursive: true });
  mkdirSync(join(root, "files"), { recursive: true });
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "ws"\nname = "WS"\n');
  writeFileSync(
    join(root, "collections", "api", "collection.toml"),
    'schema_version = 1\nid = "api"\nname = "API"\n',
  );
  writeFileSync(join(root, "files", "payload.json"), '{"name":"nova"}\n');
  writeFileSync(join(root, "collections", "api", "ping.http"), ok("Ping") + ok("Second"));
  writeFileSync(join(root, "collections", "api", "sub", "nested.http"), ok("Nested"));
  writeFileSync(
    join(root, "collections", "api", "wire.grpc"),
    `### Say
localhost:1/mock.v1.Mock/Say
proto: protos/mock.proto

{"text": "hola"}
`,
  );
  writeFileSync(
    join(root, "collections", "api", "upload.http"),
    `### Upload
POST ${origin}/x

< ./files/payload.json
`,
  );
  writeFileSync(
    join(root, "collections", "api", "form.http"),
    `### Form
POST ${origin}/x
Content-Type: multipart/form-data

title = Q3 expenses
attachments = < ./files/payload.json
`,
  );
});

afterAll(async () => {
  if (root) rmSync(root, { recursive: true, force: true });
  if (server) await new Promise<void>((done) => server.close(() => done()));
});

describe("engine selection", () => {
  it("sends HTTP in-process with no CLI on the machine at all", async () => {
    const { executor, log } = build("auto", false, neverSpawn);
    const result = await executor.send(workspace(), collection(), "ping.http#0", undefined, {});
    expect(result.response?.status).toBe(200);
    expect(result.tests.map((test) => test.passed)).toEqual([true]);
    expect(log.mock.calls.flat()).toContain("engine: in-process · ping.http#0");
  });

  it("addresses the block the index names, not the first one in the file", async () => {
    const { executor } = build("auto", false, neverSpawn);
    const result = await executor.send(workspace(), collection(), "ping.http#1", undefined, {});
    expect(result.name).toBe("Second");
  });

  it("runs a whole collection of HTTP requests in-process", async () => {
    const { executor } = build("auto", false, neverSpawn);
    const result = await executor.run(workspace(), collection(), { folder: "sub" }, {});
    expect(result.total).toBe(1);
    expect(result.requests[0]?.path).toBe("sub/nested.http#0");
    expect(result.passed).toBe(1);
  });

  it("routes gRPC to the CLI in auto mode", async () => {
    const { executor, log } = build("auto", true, cliSend);
    const result = await executor.send(workspace(), collection(), "wire.grpc#0", undefined, {});
    expect(result.grpc?.body).toBe("{}");
    expect(log.mock.calls.flat()).toContain("engine: cli · wire.grpc#0");
  });

  it("routes a mixed collection to the CLI in auto mode", async () => {
    const { executor } = build("auto", true, cliRun);
    const result = await executor.run(workspace(), collection(), {}, {});
    expect(result.requests[0]?.path).toBe("wire.grpc#0");
  });

  it("always shells out in cli mode", async () => {
    const { executor, log } = build("cli", true, cliSend);
    await executor.send(workspace(), httpOnly(), "ping.http#0", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: cli · ping.http#0");
  });

  it("names gRPC as the reason and the missing binary as the blocker", async () => {
    const { executor } = build("in-process", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "wire.grpc#0", undefined, {}),
    ).rejects.toThrow(/gRPC needs HTTP\/2 trailers.*no binary is available/s);
  });

  it("escalates gRPC to the CLI when one exists", async () => {
    const { executor, log } = build("in-process", true, cliSend);
    const result = await executor.send(workspace(), collection(), "wire.grpc#0", undefined, {});
    expect(result.grpc?.body).toBe("{}");
    expect(log.mock.calls.flat().join("\n")).toMatch(/falling back to the CLI/);
  });

  it("reports a transport failure as E_NETWORK without touching the CLI", async () => {
    const { executor } = build("auto", false, neverSpawn);
    writeFileSync(
      join(root, "collections", "api", "dead.http"),
      "### Dead\nGET http://127.0.0.1:1/nope\n",
    );
    const result = await executor.send(workspace(), collection(), "dead.http#0", undefined, {});
    expect(result.passed).toBe(false);
    expect(result.errorCode).toBe("E_NETWORK");
  });
});

describe("what only the CLI can do", () => {
  it("hands a `< file` body to the CLI, which is the side that can read the disk", async () => {
    const { executor, log } = build("auto", true, cliSend);
    await executor.send(workspace(), collection(), "upload.http#0", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: cli · upload.http#0");
  });

  // The extension never builds a multipart body itself, so the bytes a form-data
  // request puts on the wire are the CLI's — the same ones `e2e_text_format`
  // compares part-for-part against the mock.
  it("hands a form-data body to the CLI too", async () => {
    const { executor, log } = build("auto", true, cliSend);
    await executor.send(workspace(), collection(), "form.http#0", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: cli · form.http#0");
  });

  it("matches a `#name` address in process, the way the CLI matches it", async () => {
    const { executor, log } = build("auto", false, neverSpawn);
    const result = await executor.send(workspace(), collection(), "ping.http#Second", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: in-process · ping.http#Second");
    expect(result.name).toBe("Second");
  });

  it("refuses a `#name` no block carries rather than running block 0", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "ping.http#Nope", undefined, {}),
    ).rejects.toThrow(/no request named "Nope" in this file/);
  });

  it("refuses a bare path into a file that holds more than one request", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "ping.http", undefined, {}),
    ).rejects.toThrow(/ping.http holds 2 requests — address one of them as ping.http#0/);
  });

  it("says which file is not a request file at all", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "collection.toml", undefined, {}),
    ).rejects.toThrow(/is not a request file/);
  });

  it("says how many requests the file holds when the index runs past the end", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "ping.http#9", undefined, {}),
    ).rejects.toThrow(/this file holds 2 requests, so there is no request 9/);
  });
});

describe("suiteRequests", () => {
  it("walks nested folders and sorts by path", () => {
    expect(suiteRequests(collection()).map((entry) => entry.relPath)).toEqual([
      "ping.http#0",
      "sub/nested.http#0",
      "wire.grpc#0",
    ]);
  });

  it("orders blocks of one file by index, not by string", () => {
    const many = collection();
    many.folders = [];
    many.requests = [
      request("Ten", "api.http", 10),
      request("Zero", "api.http", 0),
      request("Two", "api.http", 2),
    ];
    expect(suiteRequests(many).map((entry) => entry.relPath)).toEqual([
      "api.http#0",
      "api.http#2",
      "api.http#10",
    ]);
  });

  it("narrows to a folder prefix", () => {
    expect(suiteRequests(collection(), "sub").map((entry) => entry.relPath)).toEqual([
      "sub/nested.http#0",
    ]);
  });
});
