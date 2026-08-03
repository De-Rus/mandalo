import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { MandaloCli, type RequestOutcome } from "../../src/core/cli";
import { parseRequest } from "../../src/core/parse";
import { runMany } from "../../src/engine/run";
import { cliIsRequired, probeCli } from "./support/cliBinary";

const { binary, reason } = probeCli();
const TIMEOUT = 90_000;

let server: Server;
let origin: string;
let workspace: string;

interface Fixture {
  path: string;
  toml: (base: string) => string;
}

// Each fixture exercises a different slice of the contract the two engines must agree on:
// assertion kinds, capture sources, script tests, auth, and the failure shapes.
const FIXTURES: Fixture[] = [
  {
    path: "status.toml",
    toml: (base) => `schema_version = 1
id = "status"
name = "Status"
kind = "http"
method = "GET"
url = "${base}/json"

[[tests]]
kind = "status"
op = "eq"
value = 200

[[tests]]
kind = "status"
op = "eq"
value = 404

[[tests]]
kind = "status"
op = "lt"
value = 300

[[tests]]
kind = "duration"
op = "lt"
value = 30000
`,
  },
  {
    path: "json.toml",
    toml: (base) => `schema_version = 1
id = "json"
name = "Json"
kind = "http"
method = "GET"
url = "${base}/json"

[[tests]]
kind = "json"
path = "$.id"
op = "eq"
value = 7

[[tests]]
kind = "json"
path = "$.name"
op = "eq"
value = "nova"

[[tests]]
kind = "json"
path = "$.name"
op = "ne"
value = "other"

[[tests]]
kind = "json"
path = "$.tags"
op = "len"
value = 2

[[tests]]
kind = "json"
path = "$.tags"
op = "contains"
value = "b"

[[tests]]
kind = "json"
path = "$.tags[0]"
op = "eq"
value = "a"

[[tests]]
kind = "json"
path = "$.nested.ok"
op = "exists"

[[tests]]
kind = "json"
path = "$.nope"
op = "absent"

[[tests]]
kind = "json"
path = "$.nope"
op = "exists"

[[tests]]
kind = "json"
path = "$.id"
op = "gt"
value = 100

[[tests]]
kind = "json"
path = "$.name"
op = "matches"
value = "^no"

[[tests]]
kind = "json"
path = "$.name"
op = "len"
value = 4
`,
  },
  {
    path: "headers.toml",
    toml: (base) => `schema_version = 1
id = "headers"
name = "Headers"
kind = "http"
method = "GET"
url = "${base}/json"

[[tests]]
kind = "header"
name = "Content-Type"
op = "exists"

[[tests]]
kind = "header"
name = "content-type"
op = "contains"
value = "json"

[[tests]]
kind = "header"
name = "x-absent"
op = "absent"

[[tests]]
kind = "header"
name = "x-absent"
op = "exists"

[[tests]]
kind = "header"
name = "x-flavour"
op = "eq"
value = "vanilla"

[[tests]]
kind = "header"
name = "x-flavour"
op = "matches"
value = "^van"
`,
  },
  {
    path: "captures.toml",
    toml: (base) => `schema_version = 1
id = "captures"
name = "Captures"
kind = "http"
method = "GET"
url = "${base}/json"

[[captures]]
from = "status"
into = "code"
scope = "run"

[[captures]]
from = "header.x-flavour"
into = "flavour"
scope = "run"

[[captures]]
from = "body.$.name"
into = "who"
scope = "run"

[[captures]]
from = "body.$.id"
into = "ident"
scope = "run"
`,
  },
  {
    path: "scripts.toml",
    toml: (base) => `schema_version = 1
id = "scripts"
name = "Scripts"
kind = "http"
method = "POST"
url = "${base}/echo"
body = "{\\"seed\\": \\"{{seed}}\\"}"

[scripts]
pre = "pm.variables.set('injected', 'yes'); pm.test('pre runs', function () { pm.expect(pm.variables.get('seed')).to.equal('s1'); });"
post = "pm.test('status is 200', function () { pm.response.to.have.status(200); }); pm.test('body echoes', function () { pm.expect(pm.response.json().seed).to.equal('s1'); }); pm.test('deliberate failure', function () { pm.expect(1).to.equal(2); });"

[[tests]]
kind = "status"
op = "eq"
value = 200
`,
  },
  {
    path: "auth.toml",
    toml: (base) => `schema_version = 1
id = "auth"
name = "Auth"
kind = "http"
method = "GET"
url = "${base}/echo-headers"

[auth]
type = "bearer"
token = "{{token}}"

[[tests]]
kind = "json"
path = "$.authorization"
op = "eq"
value = "Bearer t0ken"
`,
  },
  {
    path: "graphql.toml",
    toml: (base) => `schema_version = 1
id = "graphql"
name = "Graphql"
kind = "graphql"
method = "POST"
url = "${base}/echo"

[graphql]
query = "query Me { me { id } }"
variables = "{\\"who\\": \\"{{seed}}\\"}"

[[tests]]
kind = "json"
path = "$.query"
op = "contains"
value = "Me"

[[tests]]
kind = "json"
path = "$.variables.who"
op = "eq"
value = "s1"
`,
  },
  {
    path: "notjson.toml",
    toml: (base) => `schema_version = 1
id = "notjson"
name = "Not json"
kind = "http"
method = "GET"
url = "${base}/text"

[[tests]]
kind = "json"
path = "$.anything"
op = "exists"

[[tests]]
kind = "status"
op = "eq"
value = 200
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
    writeFileSync(join(root, "collections", "api", fixture.path), fixture.toml(base));
  }
  return root;
}

const NOT_JSON = "response body is not JSON: ";

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
  it("sends a real HTTP request and evaluates every assertion kind", async () => {
    const requests = await Promise.all(
      FIXTURES.map(async (fixture) => ({
        relPath: fixture.path,
        model: parseRequest(fixture.toml(origin)),
      })),
    );
    const result = await runMany(requests, "api", "test", { seed: "s1", token: "t0ken" });
    expect(result.total).toBe(FIXTURES.length);
    const status = result.requests.find((outcome) => outcome.path === "status.toml");
    expect(status?.response?.status).toBe(200);
    expect(status?.tests.map((test) => test.passed)).toEqual([true, false, true, true]);
    const captures = result.requests.find((outcome) => outcome.path === "captures.toml");
    expect(captures?.captures.map((capture) => [capture.into, capture.value])).toEqual([
      ["code", "200"],
      ["flavour", "vanilla"],
      ["who", "nova"],
      ["ident", "7"],
    ]);
  }, TIMEOUT);
});

describe.skipIf(binary === null)("the two engines agree", () => {
  it("produces identical test results for the same collection", async () => {
    const cli = new MandaloCli({ cliPath: () => binary as string, timeoutMs: () => 60_000 });
    const fromCli = await cli.run(workspace, "api", { env: "test" });

    const requests = await Promise.all(
      FIXTURES.map(async (fixture) => ({
        relPath: fixture.path,
        model: parseRequest(fixture.toml(origin)),
      })),
    );
    const fromEngine = await runMany(requests, "api", "test", { seed: "s1", token: "t0ken" });

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
    const fromEngine = await runMany(
      [{ relPath: "notjson.toml", model: parseRequest(notJson().toml(origin)) }],
      "api",
      "test",
      {},
    );

    const cliDetail = fromCli.requests
      .find((outcome) => outcome.path === "notjson.toml")
      ?.tests[0]?.detail;
    const engineDetail = fromEngine.requests[0]?.tests[0]?.detail;
    expect(cliDetail).toMatch(/^response body is not JSON: /);
    expect(engineDetail).toMatch(/^response body is not JSON: /);
    expect(engineDetail).not.toBe(cliDetail);
  }, TIMEOUT);
});

function notJson(): Fixture {
  const fixture = FIXTURES.find((entry) => entry.path === "notjson.toml");
  if (!fixture) throw new Error("the notjson fixture disappeared");
  return fixture;
}

it.skipIf(binary !== null || cliIsRequired())(`parity skipped: ${reason}`, () => {
  expect(binary).toBeNull();
});

it.skipIf(binary !== null || !cliIsRequired())("MANDALO_REQUIRE_CLI is set but no CLI exists", () => {
  expect.fail(reason);
});
