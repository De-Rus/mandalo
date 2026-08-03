import { describe, expect, it, vi } from "vitest";
import {
  CliExitError,
  CliMissingError,
  CliOutputError,
  cliMissingMessage,
  MandaloCli,
} from "../../src/core/cli";
import type { SpawnFn } from "../../src/core/cli";
import { assertionIndex } from "../../src/core/report";

function cliWith(spawnFn: SpawnFn, log?: (line: string) => void): MandaloCli {
  return new MandaloCli({ cliPath: () => "mandalo", spawnFn, ...(log ? { log } : {}) });
}

const ok = (stdout: string, code = 0): SpawnFn => async () => ({ code, stdout, stderr: "" });

// Captured verbatim from `mandalo --workspace /tmp/mfix/ws send api auth/login.toml
// --env staging --reporter json` (exit 0).
const SEND_JSON = `{
  "collection": "api",
  "env": "staging",
  "path": "auth/login.toml",
  "name": "Login",
  "method": "POST",
  "url": "http://127.0.0.1:8731/login",
  "response": {
    "status": 200,
    "statusText": "OK",
    "headers": [
      ["server", "BaseHTTP/0.6 Python/3.13.3"],
      ["date", "Mon, 03 Aug 2026 19:54:22 GMT"],
      ["content-type", "application/json"],
      ["content-length", "19"]
    ],
    "body": "{\\"token\\": \\"abc123\\"}",
    "binary": false,
    "durationMs": 3,
    "sizeBytes": 19
  },
  "grpc": null,
  "tests": [
    { "id": "test:0", "name": "status eq 200", "kind": "assertion", "passed": true, "detail": null },
    { "id": "test:1", "name": "json $.token exists", "kind": "assertion", "passed": true, "detail": null }
  ],
  "captures": [
    { "from": "body.$.token", "into": "token", "value": "abc123", "scope": "run" }
  ],
  "logs": [],
  "passed": true,
  "durationMs": 14,
  "error": null,
  "errorCode": null
}`;

// Same command with nothing listening on 8731 — exit 1, JSON still on stdout.
const SEND_ERROR_JSON = `{
  "collection": "api",
  "env": null,
  "path": "auth/login.toml",
  "name": "Login",
  "method": "POST",
  "url": "http://127.0.0.1:8731/login",
  "response": null,
  "grpc": null,
  "tests": [],
  "captures": [],
  "logs": [],
  "passed": false,
  "durationMs": 0,
  "error": "error sending request for url (http://127.0.0.1:8731/login)",
  "errorCode": "E_NETWORK"
}`;

// `mandalo --workspace /tmp/mfix/ws run api --env staging --reporter json` (exit 1).
const RUN_JSON = `{
  "collection": "api",
  "env": "staging",
  "total": 2,
  "passed": 1,
  "failed": 1,
  "durationMs": 1,
  "requests": [
    {
      "path": "health.http#0",
      "name": "Health",
      "method": "GET",
      "url": "http://127.0.0.1:8731/health",
      "response": {
        "status": 200,
        "statusText": "OK",
        "headers": [["content-type", "application/json"], ["content-length", "13"]],
        "body": "{\\"ok\\": false}",
        "binary": false,
        "durationMs": 0,
        "sizeBytes": 13
      },
      "grpc": null,
      "tests": [
        { "id": "test:0", "name": "status eq 500", "kind": "assertion", "passed": false, "detail": "status was 200" }
      ],
      "captures": [],
      "logs": [],
      "passed": false,
      "durationMs": 0,
      "error": null,
      "errorCode": null
    },
    {
      "path": "auth/login.toml",
      "name": "Login",
      "method": "POST",
      "url": "http://127.0.0.1:8731/login",
      "response": {
        "status": 200,
        "statusText": "OK",
        "headers": [["content-type", "application/json"], ["content-length", "19"]],
        "body": "{\\"token\\": \\"abc123\\"}",
        "binary": false,
        "durationMs": 0,
        "sizeBytes": 19
      },
      "grpc": null,
      "tests": [
        { "id": "test:0", "name": "status eq 200", "kind": "assertion", "passed": true, "detail": null },
        { "id": "test:1", "name": "json $.token exists", "kind": "assertion", "passed": true, "detail": null }
      ],
      "captures": [
        { "from": "body.$.token", "into": "token", "value": "abc123", "scope": "run" }
      ],
      "logs": [],
      "passed": true,
      "durationMs": 0,
      "error": null,
      "errorCode": null
    }
  ]
}`;

// `mandalo --workspace /tmp/mfix/ws ls --reporter json` (exit 0).
const LS_JSON = `{
  "collections": [
    {
      "id": "api",
      "slug": "api",
      "name": "API",
      "folders": [
        {
          "name": "auth",
          "path": "auth",
          "folders": [],
          "requests": [
            { "id": "req-login", "name": "Login", "kind": "http", "method": "POST", "path": "auth/login.toml" }
          ]
        }
      ],
      "requests": [
        { "id": "req-health", "name": "Health", "kind": "http", "method": "GET", "path": "health.http#0" }
      ]
    }
  ],
  "skipped": []
}`;

// `mandalo --workspace /tmp/mfix/ws env list --reporter json` (exit 0).
const ENV_LIST_JSON = `{
  "items": [{ "name": "staging", "vars": { "base_url": "http://127.0.0.1:8731" } }],
  "skipped": []
}`;

describe("MandaloCli.send", () => {
  it("parses a successful send", async () => {
    const spawnFn = vi.fn(ok(SEND_JSON));
    const result = await cliWith(spawnFn).send("/ws", "api", "auth/login.toml", "staging");
    expect(result.collection).toBe("api");
    expect(result.env).toBe("staging");
    expect(result.response?.status).toBe(200);
    expect(result.response?.statusText).toBe("OK");
    expect(result.response?.durationMs).toBe(3);
    expect(result.response?.sizeBytes).toBe(19);
    expect(result.response?.headers).toContainEqual(["content-type", "application/json"]);
    expect(result.grpc).toBeNull();
    expect(result.logs).toEqual([]);
    expect(result.passed).toBe(true);
    expect(result.durationMs).toBe(14);
    expect(result.errorCode).toBeNull();
    expect(spawnFn).toHaveBeenCalledWith(
      "mandalo",
      [
        "send",
        "api",
        "auth/login.toml",
        "--workspace",
        "/ws",
        "--reporter",
        "json",
        "--env",
        "staging",
      ],
      undefined,
    );
  });

  it("parses test identity/kind and capture provenance", async () => {
    const result = await cliWith(ok(SEND_JSON)).send("/ws", "api", "auth/login.toml", "staging");
    expect(result.tests).toEqual([
      { id: "test:0", name: "status eq 200", kind: "assertion", passed: true, detail: null },
      { id: "test:1", name: "json $.token exists", kind: "assertion", passed: true, detail: null },
    ]);
    expect(result.captures[0]).toEqual({
      from: "body.$.token",
      into: "token",
      value: "abc123",
      scope: "run",
    });
  });

  it("parses a script test alongside assertions", async () => {
    const payload = JSON.stringify({
      path: "a.toml",
      tests: [
        { id: "test:0", name: "status eq 200", kind: "assertion", passed: true, detail: null },
        { id: "script:post", name: "post script", kind: "script", passed: false, detail: "boom" },
      ],
    });
    const result = await cliWith(ok(payload)).send("/ws", "api", "a.toml");
    expect(result.tests.map((test) => test.kind)).toEqual(["assertion", "script"]);
    expect(result.tests[1]?.detail).toBe("boom");
  });

  it("defaults kind to assertion when the CLI omits it", async () => {
    const payload = JSON.stringify({
      path: "a.toml",
      tests: [{ id: "test:0", name: "t", passed: true }],
    });
    const result = await cliWith(ok(payload)).send("/ws", "api", "a.toml");
    expect(result.tests[0]?.kind).toBe("assertion");
  });

  it("fails loud when a test carries no id", async () => {
    const payload = JSON.stringify({ path: "a.toml", tests: [{ name: "t", passed: true }] });
    await expect(cliWith(ok(payload)).send("/ws", "api", "a.toml")).rejects.toThrow(
      /"tests\[\]\.id" must be a string/,
    );
  });

  it("omits --env when no environment is selected", async () => {
    const spawnFn = vi.fn(ok(SEND_JSON));
    await cliWith(spawnFn).send("/ws", "api", "auth/login.toml");
    expect(spawnFn.mock.calls[0]?.[1]).not.toContain("--env");
  });

  it("surfaces a transport failure carried in the payload despite exit 1", async () => {
    const result = await cliWith(ok(SEND_ERROR_JSON, 1)).send("/ws", "api", "auth/login.toml");
    expect(result.error).toBe("error sending request for url (http://127.0.0.1:8731/login)");
    expect(result.errorCode).toBe("E_NETWORK");
    expect(result.response).toBeNull();
    expect(result.passed).toBe(false);
    expect(result.env).toBeNull();
  });
});

describe("MandaloCli.run", () => {
  it("parses a run with failures and keeps the payload despite a non-zero exit", async () => {
    const result = await cliWith(ok(RUN_JSON, 1)).run("/ws", "api", { env: "staging" });
    expect(result.collection).toBe("api");
    expect(result.env).toBe("staging");
    expect(result.total).toBe(2);
    expect(result.failed).toBe(1);
    expect(result.durationMs).toBe(1);
    expect(result.requests[0]?.tests[0]?.detail).toBe("status was 200");
  });

  it("reads the top-level passed COUNT from the payload rather than deriving it", async () => {
    const result = await cliWith(ok(RUN_JSON, 1)).run("/ws", "api", { env: "staging" });
    expect(result.passed).toBe(1);
    expect(result.requests[0]?.passed).toBe(false);
    expect(result.requests[1]?.passed).toBe(true);
  });

  it("derives counters when the CLI omits them", async () => {
    const bare = JSON.stringify({
      requests: [
        {
          path: "a.toml",
          response: null,
          tests: [{ id: "test:0", name: "t", passed: false }],
          captures: [],
          error: null,
        },
      ],
    });
    const result = await cliWith(ok(bare)).run("/ws", "api");
    expect(result.total).toBe(1);
    expect(result.failed).toBe(1);
    expect(result.passed).toBe(0);
  });

  it("passes --folder and --fail-fast through", async () => {
    const spawnFn = vi.fn(ok(RUN_JSON));
    await cliWith(spawnFn).run("/ws", "api", { folder: "auth", failFast: true });
    expect(spawnFn.mock.calls[0]?.[1]).toEqual([
      "run",
      "api",
      "--workspace",
      "/ws",
      "--reporter",
      "json",
      "--folder",
      "auth",
      "--fail-fast",
    ]);
  });
});

describe("per-assertion mapping", () => {
  it("maps every assertion of a real run payload to the right verdict by id", async () => {
    const result = await cliWith(ok(RUN_JSON, 1)).run("/ws", "api", { env: "staging" });
    const health = result.requests.find((request) => request.path === "health.http#0")!;
    const login = result.requests.find((request) => request.path === "auth/login.toml")!;

    const healthIndex = assertionIndex("/ws::api::health.http#0", health.tests);
    expect([...healthIndex.keys()]).toEqual(["/ws::api::health.http#0::test:0"]);
    expect(healthIndex.get("/ws::api::health.http#0::test:0")?.passed).toBe(false);

    const loginIndex = assertionIndex("/ws::api::auth/login.toml", login.tests);
    expect(loginIndex.get("/ws::api::auth/login.toml::test:0")?.passed).toBe(true);
    expect(loginIndex.get("/ws::api::auth/login.toml::test:1")?.passed).toBe(true);
    expect(loginIndex.get("/ws::api::auth/login.toml::test:2")).toBeUndefined();
  });

  it("keeps duplicate assertion names distinct", async () => {
    const payload = JSON.stringify({
      requests: [
        {
          path: "a.toml",
          tests: [
            { id: "test:0", name: "status eq 200", kind: "assertion", passed: true, detail: null },
            { id: "test:1", name: "status eq 200", kind: "assertion", passed: false, detail: "nope" },
          ],
        },
      ],
    });
    const result = await cliWith(ok(payload)).run("/ws", "api");
    const index = assertionIndex("item", result.requests[0]!.tests);
    expect(index.size).toBe(2);
    expect(index.get("item::test:0")?.passed).toBe(true);
    expect(index.get("item::test:1")?.passed).toBe(false);
  });
});

describe("MandaloCli.ls and env list", () => {
  it("parses a tree", async () => {
    const spawnFn = vi.fn(ok(LS_JSON));
    const result = await cliWith(spawnFn).ls("/ws");
    const collection = result.collections[0]!;
    expect(collection.id).toBe("api");
    expect(collection.slug).toBe("api");
    expect(collection.name).toBe("API");
    expect(collection.folders[0]?.path).toBe("auth");
    expect(collection.folders[0]?.requests[0]).toEqual({
      id: "req-login",
      name: "Login",
      kind: "http",
      method: "POST",
      path: "auth/login.toml",
    });
    expect(collection.requests[0]?.path).toBe("health.http#0");
    expect(result.skipped).toEqual([]);
    expect(spawnFn.mock.calls[0]?.[1]).toEqual(["ls", "--workspace", "/ws", "--reporter", "json"]);
  });

  it("parses environments", async () => {
    const spawnFn = vi.fn(ok(ENV_LIST_JSON));
    const result = await cliWith(spawnFn).envList("/ws");
    expect(result.items[0]).toEqual({
      name: "staging",
      vars: { base_url: "http://127.0.0.1:8731" },
    });
    expect(spawnFn.mock.calls[0]?.[1]).toEqual([
      "env",
      "list",
      "--workspace",
      "/ws",
      "--reporter",
      "json",
    ]);
  });
});

describe("MandaloCli failure modes", () => {
  it("reports a missing binary with an actionable message", async () => {
    const spawnFn: SpawnFn = async () => {
      throw new CliMissingError("mandalo");
    };
    await expect(cliWith(spawnFn).ls("/ws")).rejects.toThrow(CliMissingError);
    await expect(cliWith(spawnFn).ls("/ws")).rejects.toThrow(cliMissingMessage("mandalo"));
  });

  it("throws CliExitError on a non-zero exit with no stdout", async () => {
    const spawnFn: SpawnFn = async () => ({
      code: 2,
      stdout: "",
      stderr: "unknown collection: nope",
    });
    await expect(cliWith(spawnFn).run("/ws", "nope")).rejects.toThrow(CliExitError);
    await expect(cliWith(spawnFn).run("/ws", "nope")).rejects.toThrow("unknown collection: nope");
  });

  it("throws CliOutputError carrying the raw output on malformed JSON", async () => {
    const raw = "thread 'main' panicked at src/main.rs:1";
    await expect(cliWith(ok(raw)).ls("/ws")).rejects.toThrow(CliOutputError);
    const error = (await cliWith(ok(raw))
      .ls("/ws")
      .catch((e: unknown) => e)) as CliOutputError;
    expect(error.raw).toBe(raw);
  });

  it("throws CliOutputError on empty stdout rather than showing nothing", async () => {
    await expect(cliWith(ok("   ")).ls("/ws")).rejects.toThrow(/empty stdout/);
  });

  it("throws CliOutputError on a shape mismatch", async () => {
    await expect(cliWith(ok('{"collections": "nope"}')).ls("/ws")).rejects.toThrow(
      /"collections" must be an array/,
    );
    await expect(cliWith(ok('{"requests": [{}]}')).run("/ws", "c")).rejects.toThrow(
      /"path" must be a string/,
    );
  });

  it("logs the invocation", async () => {
    const lines: string[] = [];
    await cliWith(ok(SEND_JSON), (line) => lines.push(line)).send("/ws", "c", "r.toml");
    expect(lines[0]).toContain("$ mandalo send c r.toml");
  });
});
