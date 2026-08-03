import { createServer, type Server } from "node:http";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { MandaloCli, type SpawnFn } from "../../src/core/cli";
import type { CollectionNode, WorkspaceNode } from "../../src/core/model";
import { MandaloExecutor, suiteRequests, type ExecutionMode } from "../../src/executor";

let server: Server;
let origin: string;
let root: string;

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
        requests: [
          {
            id: "nested",
            name: "Nested",
            kind: "http",
            method: "GET",
            relPath: "sub/nested.toml",
            fsPath: join(root, "collections", "api", "sub", "nested.toml"),
            index: 0,
            line: 0,
          },
        ],
      },
    ],
    requests: [
      {
        id: "ping",
        name: "Ping",
        kind: "http",
        method: "GET",
        relPath: "ping.toml",
        fsPath: join(root, "collections", "api", "ping.toml"),
        index: 0,
        line: 0,
      },
      {
        id: "grpc",
        name: "Grpc",
        kind: "grpc",
        method: "POST",
        relPath: "wire.toml",
        fsPath: join(root, "collections", "api", "wire.toml"),
        index: 0,
        line: 0,
      },
    ],
  };
}

function textCollection(): CollectionNode {
  const base = collection();
  const fsPath = join(root, "collections", "api", "api.http");
  return {
    ...base,
    folders: [],
    requests: [
      { id: "api.http-0", name: "One", kind: "http", method: "GET", relPath: "api.http#0", fsPath, index: 0, line: 0 },
      { id: "api.http-1", name: "Two", kind: "http", method: "POST", relPath: "api.http#1", fsPath, index: 1, line: 3 },
      {
        id: "mock.grpc-0",
        name: "Say",
        kind: "grpc",
        method: "GRPC",
        relPath: "mock.grpc#0",
        fsPath: join(root, "collections", "api", "mock.grpc"),
        index: 0,
        line: 0,
      },
    ],
  };
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
  const executor = new MandaloExecutor({
    cli,
    hasCli: () => hasCli,
    mode: () => mode,
    log,
  });
  return { executor, log };
}

const neverSpawn: SpawnFn = async () => {
  throw new Error("the CLI must not be spawned here");
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
    requests: [
      {
        path: "wire.toml",
        name: "Grpc",
        method: "POST",
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
      },
    ],
  }),
  stderr: "",
});

const cliSend: SpawnFn = async () => ({
  code: 0,
  stdout: JSON.stringify({
    collection: "api",
    env: null,
    path: "wire.toml",
    name: "Grpc",
    method: "POST",
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
  }),
  stderr: "",
});

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
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "ws"\nname = "WS"\n');
  writeFileSync(
    join(root, "collections", "api", "collection.toml"),
    'schema_version = 1\nid = "api"\nname = "API"\n',
  );
  const http = (name: string) => `schema_version = 1
id = "${name}"
name = "${name}"
kind = "http"
method = "GET"
url = "${origin}/x"

[[tests]]
kind = "status"
op = "eq"
value = 200
`;
  writeFileSync(join(root, "collections", "api", "ping.toml"), http("Ping"));
  writeFileSync(join(root, "collections", "api", "sub", "nested.toml"), http("Nested"));
  writeFileSync(
    join(root, "collections", "api", "wire.toml"),
    `schema_version = 1
id = "grpc"
name = "Grpc"
kind = "grpc"
method = "POST"
url = "grpc://localhost:1"

[grpc]
protoPaths = ["a.proto"]
service = "S"
method = "M"
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
    const result = await executor.send(workspace(), collection(), "ping.toml", undefined, {});
    expect(result.response?.status).toBe(200);
    expect(result.tests.map((test) => test.passed)).toEqual([true]);
    expect(log.mock.calls.flat()).toContain(`engine: in-process · ping.toml`);
  });

  it("runs a whole collection of HTTP requests in-process", async () => {
    const { executor } = build("auto", false, neverSpawn);
    const result = await executor.run(workspace(), collection(), { folder: "sub" }, {});
    expect(result.total).toBe(1);
    expect(result.requests[0]?.path).toBe("sub/nested.toml");
    expect(result.passed).toBe(1);
  });

  it("routes gRPC to the CLI in auto mode", async () => {
    const { executor, log } = build("auto", true, cliSend);
    const result = await executor.send(workspace(), collection(), "wire.toml", undefined, {});
    expect(result.grpc?.body).toBe("{}");
    expect(log.mock.calls.flat()).toContain("engine: cli · wire.toml");
  });

  it("routes a mixed collection to the CLI in auto mode", async () => {
    const { executor } = build("auto", true, cliRun);
    const result = await executor.run(workspace(), collection(), {}, {});
    expect(result.requests[0]?.path).toBe("wire.toml");
  });

  it("always shells out in cli mode", async () => {
    const { executor, log } = build("cli", true, cliSend);
    await executor.send(workspace(), collection(), "ping.toml", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: cli · ping.toml");
  });

  it("names gRPC as the reason and the missing binary as the blocker", async () => {
    const { executor } = build("in-process", false, neverSpawn);
    await expect(
      executor.send(workspace(), collection(), "wire.toml", undefined, {}),
    ).rejects.toThrow(/gRPC needs HTTP\/2 trailers.*no binary is available/s);
  });

  it("escalates gRPC to the CLI when one exists", async () => {
    const { executor, log } = build("in-process", true, cliSend);
    const result = await executor.send(workspace(), collection(), "wire.toml", undefined, {});
    expect(result.grpc?.body).toBe("{}");
    expect(log.mock.calls.flat().join("\n")).toMatch(/falling back to the CLI/);
  });

  it("reports a transport failure as E_NETWORK without touching the CLI", async () => {
    const { executor } = build("auto", false, neverSpawn);
    writeFileSync(
      join(root, "collections", "api", "dead.toml"),
      'schema_version = 1\nid = "dead"\nname = "Dead"\nkind = "http"\nmethod = "GET"\nurl = "http://127.0.0.1:1/nope"\n',
    );
    const result = await executor.send(workspace(), collection(), "dead.toml", undefined, {});
    expect(result.passed).toBe(false);
    expect(result.errorCode).toBe("E_NETWORK");
  });
});

describe("the .http and .grpc text formats", () => {
  it("sends a numbered block through the CLI, path and index intact", async () => {
    const seen: string[][] = [];
    const spy: SpawnFn = async (_binary, args) => {
      seen.push(args);
      return cliSend(_binary, args);
    };
    const { executor, log } = build("auto", true, spy);
    await executor.send(workspace(), textCollection(), "api.http#1", "prod", {});
    expect(seen[0]).toEqual([
      "send",
      "api",
      "api.http#1",
      "--workspace",
      root,
      "--reporter",
      "json",
      "--env",
      "prod",
    ]);
    expect(log.mock.calls.flat()).toContain("engine: cli · api.http#1");
  });

  it("sends a .grpc block through the CLI too", async () => {
    const { executor, log } = build("auto", true, cliSend);
    await executor.send(workspace(), textCollection(), "mock.grpc#0", undefined, {});
    expect(log.mock.calls.flat()).toContain("engine: cli · mock.grpc#0");
  });

  it("stays on the CLI even when the user forced in-process", async () => {
    const { executor, log } = build("in-process", true, cliSend);
    await executor.send(workspace(), textCollection(), "api.http#0", undefined, {});
    expect(log.mock.calls.flat().join("\n")).toMatch(/text format is parsed by the Mándalo CLI/);
  });

  it("names the text format and the missing binary when no CLI exists", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(executor.send(workspace(), textCollection(), "api.http#0", undefined, {})).rejects.toThrow(
      /text format is parsed by the Mándalo CLI.*no binary is available.*mandalo\.cliPath/s,
    );
  });

  it("runs a collection of text requests through the CLI", async () => {
    const { executor, log } = build("auto", true, cliRun);
    const result = await executor.run(workspace(), textCollection(), {}, {});
    expect(result.total).toBe(1);
    expect(log.mock.calls.flat()).toContain("engine: cli · api");
  });

  it("refuses to run a text collection with no CLI rather than half-running it", async () => {
    const { executor } = build("auto", false, neverSpawn);
    await expect(executor.run(workspace(), textCollection(), {}, {})).rejects.toThrow(
      /no binary is available/,
    );
  });
});

describe("suiteRequests", () => {
  it("walks nested folders and sorts by path", () => {
    expect(suiteRequests(collection()).map((request) => request.relPath)).toEqual([
      "ping.toml",
      "sub/nested.toml",
      "wire.toml",
    ]);
  });

  it("orders blocks of one file by index, not by string", () => {
    const many = textCollection();
    many.requests = [
      { ...many.requests[1]!, relPath: "api.http#10", index: 10 },
      many.requests[0]!,
      { ...many.requests[1]!, relPath: "api.http#2", index: 2 },
    ];
    expect(suiteRequests(many).map((request) => request.relPath)).toEqual([
      "api.http#0",
      "api.http#2",
      "api.http#10",
    ]);
  });

  it("narrows to a folder prefix", () => {
    expect(suiteRequests(collection(), "sub").map((request) => request.relPath)).toEqual([
      "sub/nested.toml",
    ]);
  });
});
