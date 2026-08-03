import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { MandaloCli } from "../../src/core/cli";
import { cliIsRequired, probeCli } from "./support/cliBinary";

// Resolved exactly as the extension resolves it: the bundled bin/ first, then PATH, then
// the repo's own build outputs. `npm run bundle:cli` is what makes this suite run.
const { binary: resolved, reason: SKIP_REASON } = probeCli();
const available = resolved !== null;
const binary = resolved ?? "mandalo";

// The request URL points at a port nothing listens on, so `run` deterministically
// exercises the transport-error path without needing a server. Port 1 refuses
// immediately; a filtered port would hang until the connect timeout.
const DEAD_PORT_URL = "http://127.0.0.1:1/health";

// A debug-profile binary costs seconds per spawn on macOS, past vitest's 5s default.
// Scoped per test so the pure unit tests keep failing fast.
const E2E_TIMEOUT_MS = 60_000;

// Fires before the vitest timeout so a wrong binary reports "timed out" rather than
// stalling the suite.
const SPAWN_TIMEOUT_MS = 20_000;

function makeWorkspace(): string {
  const root = mkdtempSync(join(tmpdir(), "mandalo-e2e-"));
  mkdirSync(join(root, "collections", "api", "auth"), { recursive: true });
  mkdirSync(join(root, "environments"), { recursive: true });
  writeFileSync(
    join(root, "mandalo.toml"),
    'schema_version = 1\nid = "e2e-ws"\nname = "E2E"\n',
  );
  writeFileSync(
    join(root, "collections", "api", "collection.toml"),
    'schema_version = 1\nid = "api"\nname = "API"\n',
  );
  writeFileSync(
    join(root, "collections", "api", "health.http"),
    `### Health\nGET ${DEAD_PORT_URL}\n`,
  );
  writeFileSync(
    join(root, "collections", "api", "auth", "login.http"),
    "### Login\nPOST {{base_url}}/login\n",
  );
  writeFileSync(
    join(root, "environments", "staging.toml"),
    'name = "staging"\n[vars]\nbase_url = "http://127.0.0.1:1"\n',
  );
  return root;
}

const workspaces: string[] = [];

function workspace(): string {
  const root = makeWorkspace();
  workspaces.push(root);
  return root;
}

afterAll(() => {
  for (const root of workspaces) rmSync(root, { recursive: true, force: true });
});

const cli = new MandaloCli({ cliPath: () => binary, timeoutMs: () => SPAWN_TIMEOUT_MS });

describe.skipIf(!available)("MandaloCli against the real binary", () => {
  it("parses real `ls --reporter json` output", async () => {
    const result = await cli.ls(workspace());
    const collection = result.collections.find((entry) => entry.slug === "api");
    expect(collection).toBeDefined();
    expect(collection!.name).toBe("API");
    expect(collection!.requests.map((request) => request.path)).toEqual(["health.http#0"]);
    expect(collection!.folders[0]?.path).toBe("auth");
    expect(collection!.folders[0]?.requests[0]?.path).toBe("auth/login.http#0");
  }, E2E_TIMEOUT_MS);

  it("parses real `env list --reporter json` output", async () => {
    const result = await cli.envList(workspace());
    expect(result.items.map((item) => item.name)).toContain("staging");
    expect(result.items[0]?.vars["base_url"]).toBe("http://127.0.0.1:1");
  }, E2E_TIMEOUT_MS);

  it("parses a real `run --reporter json` transport failure (exit 1, JSON on stdout)", async () => {
    const result = await cli.run(workspace(), "api", { env: "staging" });
    expect(result.collection).toBe("api");
    expect(result.env).toBe("staging");
    expect(result.total).toBe(result.requests.length);
    expect(result.failed).toBeGreaterThan(0);
    const health = result.requests.find((request) => request.path === "health.http#0");
    expect(health).toBeDefined();
    expect(health!.passed).toBe(false);
    expect(health!.response).toBeNull();
    expect(health!.error).toBeTruthy();
    expect(health!.errorCode).toBe("E_NETWORK");
  }, E2E_TIMEOUT_MS);

  it("parses a real `send --reporter json` transport failure", async () => {
    const result = await cli.send(workspace(), "api", "health.http#0");
    expect(result.path).toBe("health.http#0");
    expect(result.name).toBe("Health");
    expect(result.passed).toBe(false);
    expect(result.errorCode).toBe("E_NETWORK");
    expect(result.tests).toEqual([]);
  }, E2E_TIMEOUT_MS);
});

it.skipIf(available || cliIsRequired())(`skipped: ${SKIP_REASON}`, () => {
  expect(available).toBe(false);
});

// CI sets MANDALO_REQUIRE_CLI=1, so a missing binary is a red build rather than a
// green suite that quietly covered nothing.
it.skipIf(available || !cliIsRequired())("MANDALO_REQUIRE_CLI is set but no CLI exists", () => {
  expect.fail(SKIP_REASON);
});
