import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { MandaloCli, type RequestOutcome } from "../../src/core/cli";
import { parseTextDocument, withFileVars } from "../../src/core/httpFormat";
import { runMany } from "../../src/engine/run";
import type { EngineRequest } from "../../src/engine/run";
import { cliIsRequired, probeCli } from "./support/cliBinary";

const { binary, reason } = probeCli();
const TIMEOUT = 90_000;

let server: Server;
let origin: string;
let workspace: string;

interface Fixture {
  path: string;
  http: (base: string) => string;
}

// Each fixture exercises a different slice of the contract the two engines must agree
// on: assertion kinds, script tests, auth, GraphQL and the failure shapes. Declarative
// tests and captures are gone with the TOML format — a `.http` file asserts and chains
// from `pm.*`, so that is what parity now covers.
const FIXTURES: Fixture[] = [
  {
    path: "status.http",
    http: (base) => `### Status
GET ${base}/json

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
pm.test("status is 404", function () { pm.response.to.have.status(404); });
pm.test("status is below 300", function () { pm.expect(pm.response.code).to.be.below(300); });
pm.test("under thirty seconds", function () { pm.expect(pm.response.responseTime).to.be.below(30000); });
%}
`,
  },
  {
    path: "json.http",
    http: (base) => `### Json
GET ${base}/json

> {%
pm.test("id is seven", function () { pm.expect(pm.response.json().id).to.equal(7); });
pm.test("name is nova", function () { pm.expect(pm.response.json().name).to.equal("nova"); });
pm.test("two tags", function () { pm.expect(pm.response.json().tags.length).to.equal(2); });
pm.test("first tag is a", function () { pm.expect(pm.response.json().tags[0]).to.equal("a"); });
pm.test("nested ok", function () { pm.expect(pm.response.json().nested.ok).to.equal(true); });
pm.test("nope is absent", function () { pm.expect(pm.response.json().nope).to.equal(undefined); });
pm.test("id is over a hundred", function () { pm.expect(pm.response.json().id).to.be.above(100); });
%}
`,
  },
  {
    path: "headers.http",
    http: (base) => `### Headers
GET ${base}/json

> {%
pm.test("content type is json", function () {
  pm.expect(pm.response.headers.get("content-type")).to.include("json");
});
pm.test("flavour is vanilla", function () {
  pm.expect(pm.response.headers.get("x-flavour")).to.equal("vanilla");
});
pm.test("absent header is absent", function () {
  pm.expect(pm.response.headers.get("x-absent")).to.equal(undefined);
});
pm.test("absent header is somehow present", function () {
  pm.expect(pm.response.headers.get("x-absent")).to.equal("here");
});
%}
`,
  },
  {
    path: "chain.http",
    http: (base) => `### 1 Capture
GET ${base}/json

> {%
pm.environment.set("who", pm.response.json().name);
pm.test("captured a name", function () { pm.expect(pm.environment.get("who")).to.equal("nova"); });
%}

### 2 Use the capture
POST ${base}/echo
Content-Type: application/json

{"who": "{{who}}"}

> {%
pm.test("the capture reached the next request", function () {
  pm.expect(pm.response.json().who).to.equal("nova");
});
%}
`,
  },
  {
    path: "scripts.http",
    http: (base) => `### Scripts
< {%
pm.variables.set("injected", "yes");
pm.test("pre runs", function () { pm.expect(pm.variables.get("seed")).to.equal("s1"); });
%}
POST ${base}/echo
Content-Type: application/json

{"seed": "{{seed}}"}

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
pm.test("body echoes", function () { pm.expect(pm.response.json().seed).to.equal("s1"); });
pm.test("deliberate failure", function () { pm.expect(1).to.equal(2); });
%}
`,
  },
  {
    path: "auth.http",
    http: (base) => `### Auth
GET ${base}/echo-headers
Authorization: Bearer {{token}}

> {%
pm.test("the bearer header went out", function () {
  pm.expect(pm.response.json().authorization).to.equal("Bearer t0ken");
});
%}
`,
  },
  {
    path: "graphql.http",
    http: (base) => `### Graphql
POST ${base}/echo
X-REQUEST-TYPE: GraphQL

query Me { me { id } }

{"who": "{{seed}}"}

> {%
pm.test("the document went out", function () {
  pm.expect(pm.response.json().query).to.include("Me");
});
pm.test("the variables went out", function () {
  pm.expect(pm.response.json().variables.who).to.equal("s1");
});
%}
`,
  },
  {
    path: "notjson.http",
    http: (base) => `### Not json
GET ${base}/text

> {%
pm.test("body parses as json", function () { pm.expect(pm.response.json().anything).to.exist; });
pm.test("status is 200", function () { pm.response.to.have.status(200); });
%}
`,
  },
];

function handler(request: IncomingMessage, response: ServerResponse): void {
  const url = request.url ?? "/";
  const chunks: Buffer[] = [];
  request.on("data", (chunk: Buffer) => chunks.push(chunk));
  request.on("end", () => {
    if (url === "/json") {
      response.writeHead(200, { "content-type": "application/json", "x-flavour": "vanilla" });
      response.end(
        JSON.stringify({ id: 7, name: "nova", tags: ["a", "b"], nested: { ok: true }, empty: null }),
      );
      return;
    }
    if (url === "/text") {
      response.writeHead(200, { "content-type": "text/plain" });
      response.end("plain words");
      return;
    }
    if (url === "/echo") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(Buffer.concat(chunks).toString("utf8"));
      return;
    }
    if (url === "/echo-headers") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(request.headers));
      return;
    }
    response.writeHead(404, { "content-type": "application/json" });
    response.end("{}");
  });
}

function buildWorkspace(base: string): string {
  const root = mkdtempSync(join(tmpdir(), "mandalo-parity-"));
  mkdirSync(join(root, "collections", "api"), { recursive: true });
  mkdirSync(join(root, "environments"), { recursive: true });
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "parity"\nname = "Parity"\n');
  writeFileSync(
    join(root, "collections", "api", "collection.toml"),
    'schema_version = 1\nid = "api"\nname = "API"\n',
  );
  writeFileSync(
    join(root, "environments", "test.toml"),
    'name = "test"\n[vars]\nseed = "s1"\ntoken = "t0ken"\n',
  );
  for (const fixture of FIXTURES) {
    writeFileSync(join(root, "collections", "api", fixture.path), fixture.http(base));
  }
  return root;
}

/** The same files the CLI reads, parsed by the extension's own `.http` parser. */
function engineRequests(only?: string): EngineRequest[] {
  const out: EngineRequest[] = [];
  for (const fixture of FIXTURES) {
    if (only !== undefined && fixture.path !== only) continue;
    const document = parseTextDocument(fixture.path, fixture.http(origin), "http");
    for (const block of withFileVars(document)) {
      out.push({ model: block.model, relPath: `${fixture.path}#${block.index}` });
    }
  }
  return out;
}

const NOT_JSON = "response body is not valid JSON: ";

// serde_json and V8 word a parse failure differently and always will. The prefix, the
// assertion name and the pass/fail verdict are the contract; the parser's own sentence is
// not. Every other detail string must match byte for byte.
function normaliseDetail(detail: string | null): string | null {
  return detail !== null && detail.startsWith(NOT_JSON) ? `${NOT_JSON}<parser wording>` : detail;
}

/** Durations and wall-clock values differ by construction; everything else must match. */
function comparable(outcome: RequestOutcome): unknown {
  return {
    path: outcome.path,
    name: outcome.name,
    passed: outcome.passed,
    status: outcome.response?.status ?? null,
    tests: outcome.tests.map((test) => ({
      id: test.id,
      name: test.name,
      kind: test.kind,
      passed: test.passed,
      detail: normaliseDetail(test.detail),
    })),
    captures: outcome.captures,
    logs: outcome.logs,
  };
}

beforeAll(async () => {
  server = createServer(handler);
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("no server address");
  origin = `http://127.0.0.1:${address.port}`;
  workspace = buildWorkspace(origin);
});

afterAll(async () => {
  if (workspace) rmSync(workspace, { recursive: true, force: true });
  if (server) await new Promise<void>((done) => server.close(() => done()));
});

describe("in-process engine, with no CLI involved at all", () => {
  it("sends real requests and evaluates every script assertion", async () => {
    const result = await runMany(engineRequests(), "api", "test", { seed: "s1", token: "t0ken" });
    expect(result.total).toBe(engineRequests().length);
    const status = result.requests.find((outcome) => outcome.path === "status.http#0");
    expect(status?.response?.status).toBe(200);
    expect(status?.tests.map((test) => test.passed)).toEqual([true, false, true, true]);
    const chained = result.requests.find((outcome) => outcome.path === "chain.http#1");
    expect(chained?.tests.every((test) => test.passed)).toBe(true);
  }, TIMEOUT);
});

describe.skipIf(binary === null)("the two engines agree", () => {
  it("produces identical test results for the same collection", async () => {
    const cli = new MandaloCli({ cliPath: () => binary as string, timeoutMs: () => 60_000 });
    const fromCli = await cli.run(workspace, "api", { env: "test" });
    const fromEngine = await runMany(engineRequests(), "api", "test", {
      seed: "s1",
      token: "t0ken",
    });

    expect(fromEngine.total).toBe(fromCli.total);
    expect(fromEngine.failed).toBe(fromCli.failed);

    const cliByPath = new Map(fromCli.requests.map((outcome) => [outcome.path, outcome]));
    for (const outcome of fromEngine.requests) {
      const reference = cliByPath.get(outcome.path);
      expect(reference, `the CLI did not run ${outcome.path}`).toBeDefined();
      expect(comparable(outcome), `mismatch on ${outcome.path}`).toEqual(
        comparable(reference as RequestOutcome),
      );
    }
  }, TIMEOUT);

  it("the only tolerated divergence is the JSON parser's own wording", async () => {
    const cli = new MandaloCli({ cliPath: () => binary as string, timeoutMs: () => 60_000 });
    const fromCli = await cli.run(workspace, "api", { env: "test" });
    const fromEngine = await runMany(engineRequests("notjson.http"), "api", "test", {});

    const cliDetail = fromCli.requests
      .find((outcome) => outcome.path === "notjson.http#0")
      ?.tests[0]?.detail;
    const engineDetail = fromEngine.requests[0]?.tests[0]?.detail;
    expect(cliDetail).toMatch(/^response body is not valid JSON: /);
    expect(engineDetail).toMatch(/^response body is not valid JSON: /);
    expect(engineDetail).not.toBe(cliDetail);
  }, TIMEOUT);
});

it.skipIf(binary !== null || cliIsRequired())(`parity skipped: ${reason}`, () => {
  expect(binary).toBeNull();
});

it.skipIf(binary !== null || !cliIsRequired())("MANDALO_REQUIRE_CLI is set but no CLI exists", () => {
  expect.fail(reason);
});
