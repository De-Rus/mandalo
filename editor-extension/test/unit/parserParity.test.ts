import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { MandaloCli, type LsCollection, type LsRequest } from "../../src/core/cli";
import { parseTextDocument, TextFormatError, withFileVars } from "../../src/core/httpFormat";
import { runOne } from "../../src/engine/run";
import { textFileKind } from "../../src/core/textFormat";
import { cliIsRequired, probeCli } from "./support/cliBinary";

// The TypeScript reader in src/core/httpFormat.ts is an interim implementation; the
// Rust parser in crates/core is the reference. This suite is what keeps the stopgap
// honest — every corpus file is parsed by both and the two views must agree.
const { binary, reason } = probeCli();
const TIMEOUT = 120_000;

const HERE = dirname(fileURLToPath(import.meta.url));
const EXTENSION_ROOT = resolve(HERE, "../..");
const REPO_ROOT = resolve(EXTENSION_ROOT, "..");
const MOCK_WORKSPACE = join(REPO_ROOT, "examples", "mock-workspace");
const FIXTURE_WORKSPACE = join(EXTENSION_ROOT, "fixtures", "workspace");

/** The cases a hand-written reader gets wrong: they all live in one file on purpose. */
const AWKWARD: Record<string, string> = {
  "multi.http": `@host = api.dev
@base = https://{{host}}/v1

### One
GET {{base}}/one
Accept: application/json

### Two
POST {{base}}/two
Content-Type: application/json

{"a": 1}

> {%
pm.test("ok", function () { pm.response.to.have.status(200); });
%}

### Three
< {%
pm.variables.set("x", "1");
%}
DELETE {{base}}/three
`,
  "hashes.http": `### A body whose own line starts with three hashes
POST https://api.dev/markdown
Content-Type: text/markdown

# a heading stays body text
### but this line separates, because the format has no body exception
GET https://api.dev/after-the-body
`,
  "bare.http": `GET https://api.dev/only
Accept: */*
`,
  "nomethod.http": `### No method at all
https://api.dev/defaults-to-get

### Version suffix
https://api.dev/versioned HTTP/1.1
`,
  "named.http": `###
# @name From the directive
GET https://api.dev/named

###
GET https://api.dev/unnamed
`,
  "continuation.http": `### Wrapped URL
GET https://api.dev/search?q=one
  &page=2
  &limit=10
Accept: application/json
`,
  "closers.http": `### Real
GET https://api.dev/real

###
`,
  "grpc-multi.grpc": `@protoDir = protos

### Say
{{grpcUrl}}/mock.v1.Mock/Say
proto: {{protoDir}}/mock.proto
x-trace: mandalo

{"text": "hola"}

### GetUser
{{grpcUrl}}/mock.v1.Mock/GetUser
proto: protos/mock.proto

{"id": "u-1"}
`,
};

/** The same file written with CRLF must parse identically on both sides. */
const CRLF_SOURCE = AWKWARD["multi.http"]!.replace(/\n/g, "\r\n");

interface ParsedView {
  path: string;
  id: string;
  name: string;
  kind: string;
  method: string;
}

function flatten(collection: LsCollection): LsRequest[] {
  const out: LsRequest[] = [...collection.requests];
  const walk = (folders: LsCollection["folders"]): void => {
    for (const folder of folders) {
      out.push(...folder.requests);
      walk(folder.folders);
    }
  };
  walk(collection.folders);
  return out;
}

function view(request: LsRequest): ParsedView {
  return {
    path: request.path,
    id: request.id,
    name: request.name,
    kind: request.kind,
    method: request.method,
  };
}

function requestFiles(dir: string, prefix = ""): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const relPath = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
    if (entry.isDirectory()) out.push(...requestFiles(join(dir, entry.name), relPath));
    else if (textFileKind(entry.name) !== undefined) out.push(relPath);
  }
  return out.sort();
}

/** The TypeScript reader's view of a collection directory, shaped like `ls` output. */
function typescriptView(collectionDir: string): { requests: ParsedView[]; skipped: string[] } {
  const requests: ParsedView[] = [];
  const skipped: string[] = [];
  for (const relPath of requestFiles(collectionDir)) {
    const fileKind = textFileKind(relPath)!;
    const source = readFileSync(join(collectionDir, ...relPath.split("/")), "utf8");
    let blocks;
    try {
      blocks = parseTextDocument(relPath, source, fileKind).blocks;
    } catch (error) {
      if (!(error instanceof TextFormatError)) throw error;
      skipped.push(`${relPath}: ${error.message}`);
      continue;
    }
    for (const block of blocks) {
      requests.push({
        path: `${relPath}#${block.index}`,
        id: block.model.id,
        name: block.name,
        kind: block.model.kind,
        method: block.model.kind === "grpc" ? "POST" : block.model.method,
      });
    }
  }
  return { requests, skipped };
}

function sortViews(views: ParsedView[]): ParsedView[] {
  return [...views].sort((a, b) => a.path.localeCompare(b.path));
}

function awkwardWorkspace(): string {
  const root = mkdtempSync(join(tmpdir(), "mandalo-parser-"));
  const dir = join(root, "collections", "awkward");
  mkdirSync(dir, { recursive: true });
  mkdirSync(join(root, "environments"), { recursive: true });
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "aw"\nname = "Awkward"\n');
  writeFileSync(join(dir, "collection.toml"), 'schema_version = 1\nid = "awkward"\nname = "Awkward"\n');
  for (const [name, source] of Object.entries(AWKWARD)) writeFileSync(join(dir, name), source);
  writeFileSync(join(dir, "crlf.http"), CRLF_SOURCE);
  return root;
}

// --- wire parity ------------------------------------------------------------------

interface Echo {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
}

const WIRE: Record<string, string> = {
  "wire.http": `@suffix = v2

### Query string and a plain header
GET {{baseUrl}}/echo/{{suffix}}?page=2&limit=10
X-Trace: {{trace}}
Accept: application/json

### JSON body
POST {{baseUrl}}/echo
Content-Type: application/json

{"name": "nova", "tier": "pro"}

### Form body
POST {{baseUrl}}/echo
Content-Type: application/x-www-form-urlencoded

username=ada+lovelace&password=lovelace

### Bearer auth
GET {{baseUrl}}/echo
Authorization: Bearer {{token}}

### Basic auth
GET {{baseUrl}}/echo
Authorization: Basic ada:lovelace

### API key header
GET {{baseUrl}}/echo
x-api-key: mock-api-key

### GraphQL with variables
POST {{baseUrl}}/echo
X-REQUEST-TYPE: GraphQL

query User($id: ID!) {
  user(id: $id) { id name }
}

{"id": "u-1"}

### GraphQL without variables
POST {{baseUrl}}/echo
X-REQUEST-TYPE: GraphQL

{ users { id name } }
`,
};

function wireWorkspace(base: string): string {
  const root = mkdtempSync(join(tmpdir(), "mandalo-wire-"));
  const dir = join(root, "collections", "wire");
  mkdirSync(dir, { recursive: true });
  mkdirSync(join(root, "environments"), { recursive: true });
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "wire"\nname = "Wire"\n');
  writeFileSync(join(dir, "collection.toml"), 'schema_version = 1\nid = "wire"\nname = "Wire"\n');
  writeFileSync(
    join(root, "environments", "test.toml"),
    `name = "test"\n[vars]\nbaseUrl = "${base}"\ntoken = "t0ken"\ntrace = "mandalo"\n`,
  );
  for (const [name, source] of Object.entries(WIRE)) writeFileSync(join(dir, name), source);
  return root;
}

const VARS = { baseUrl: "", token: "t0ken", trace: "mandalo" };

/** Only the headers the file itself declares — the rest is transport furniture. */
function declaredHeaders(echo: Echo, names: readonly string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const name of names) {
    const value = echo.headers[name.toLowerCase()];
    if (value !== undefined) out[name.toLowerCase()] = value;
  }
  return out;
}

let server: Server;
let origin: string;
let wireRoot: string;
let awkwardRoot: string;
const temporary: string[] = [];

function handler(request: IncomingMessage, response: ServerResponse): void {
  const chunks: Buffer[] = [];
  request.on("data", (chunk: Buffer) => chunks.push(chunk));
  request.on("end", () => {
    const headers: Record<string, string> = {};
    for (const [key, value] of Object.entries(request.headers)) {
      headers[key.toLowerCase()] = Array.isArray(value) ? value.join(", ") : String(value ?? "");
    }
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({
        method: request.method ?? "",
        url: request.url ?? "",
        headers,
        body: Buffer.concat(chunks).toString("utf8"),
      }),
    );
  });
}

beforeAll(async () => {
  server = createServer(handler);
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("no server address");
  origin = `http://127.0.0.1:${address.port}`;
  VARS.baseUrl = origin;
  wireRoot = wireWorkspace(origin);
  awkwardRoot = awkwardWorkspace();
  temporary.push(wireRoot, awkwardRoot);
});

afterAll(async () => {
  for (const root of temporary) rmSync(root, { recursive: true, force: true });
  if (server) await new Promise<void>((done) => server.close(() => done()));
});

describe.skipIf(binary === null)("the TypeScript reader and the Rust parser see the same file", () => {
  const cli = () => new MandaloCli({ cliPath: () => binary as string, timeoutMs: () => 60_000 });

  const corpora: [string, () => string][] = [
    ["the shipped mock workspace", () => MOCK_WORKSPACE],
    ["the extension's own fixture workspace", () => FIXTURE_WORKSPACE],
    ["a workspace of the awkward cases", () => awkwardRoot],
    ["the wire-parity workspace", () => wireRoot],
  ];

  for (const [label, rootOf] of corpora) {
    it(`agrees on every request in ${label}`, async () => {
      const root = rootOf();
      const listed = await cli().ls(root);
      expect(listed.collections.length).toBeGreaterThan(0);
      const refused: string[] = [];
      for (const collection of listed.collections) {
        const fromRust = sortViews(flatten(collection).map(view));
        const mine = typescriptView(join(root, "collections", collection.slug));
        expect(sortViews(mine.requests), `collection ${collection.slug} of ${root}`).toEqual(
          fromRust,
        );
        refused.push(...mine.skipped);
      }
      // A file one reader refuses the other must refuse too, with the same sentence.
      expect(refused.sort()).toEqual([...listed.skipped].sort());
    }, TIMEOUT);
  }

  it("counts the blocks of a file that a body's own ### splits", async () => {
    const listed = await cli().ls(awkwardRoot);
    const paths = flatten(listed.collections[0]!)
      .map((entry) => entry.path)
      .filter((path) => path.startsWith("hashes.http"));
    expect(paths).toEqual(["hashes.http#0", "hashes.http#1"]);
    expect(
      parseTextDocument("hashes.http", AWKWARD["hashes.http"]!, "http").blocks.map(
        (block) => block.model.body,
      ),
    ).toEqual(["# a heading stays body text", undefined]);
  }, TIMEOUT);

  it("reads a CRLF file exactly like its LF twin, on both sides", async () => {
    const listed = await cli().ls(awkwardRoot);
    const all = flatten(listed.collections[0]!);
    const strip = (prefix: string) =>
      all
        .filter((entry) => entry.path.startsWith(`${prefix}#`))
        .map((entry) => ({ name: entry.name, kind: entry.kind, method: entry.method }));
    expect(strip("crlf.http")).toEqual(strip("multi.http"));
    expect(
      parseTextDocument("multi.http", CRLF_SOURCE, "http").blocks.map((block) => block.name),
    ).toEqual(
      parseTextDocument("multi.http", AWKWARD["multi.http"]!, "http").blocks.map(
        (block) => block.name,
      ),
    );
  }, TIMEOUT);
});

describe.skipIf(binary === null)("the two readers put the same bytes on the wire", () => {
  it("agrees on method, URL, declared headers and body for every block", async () => {
    const cli = new MandaloCli({ cliPath: () => binary as string, timeoutMs: () => 60_000 });
    const source = WIRE["wire.http"]!;
    const blocks = withFileVars(parseTextDocument("wire.http", source, "http"));
    expect(blocks.length).toBe(8);
    const seen: Echo[] = [];

    for (const block of blocks) {
      const relPath = `wire.http#${block.index}`;
      const fromCli = await cli.send(wireRoot, "wire", relPath, "test");
      const fromEngine = await runOne({ model: block.model, relPath }, "wire", "test", {
        ...VARS,
      });

      const cliEcho = JSON.parse(fromCli.response?.body ?? "{}") as Echo;
      const engineEcho = JSON.parse(fromEngine.response?.body ?? "{}") as Echo;
      expect(cliEcho.method, `${relPath} never reached the echo server`).not.toBe(undefined);
      expect(cliEcho.url).toMatch(/^\//);
      seen.push(cliEcho);

      const names = [
        ...block.model.headers.map(([name]) => name),
        "authorization",
        "content-type",
      ];
      expect(
        {
          method: engineEcho.method,
          url: engineEcho.url,
          headers: declaredHeaders(engineEcho, names),
          body: engineEcho.body,
        },
        `mismatch on ${relPath} (${block.name})`,
      ).toEqual({
        method: cliEcho.method,
        url: cliEcho.url,
        headers: declaredHeaders(cliEcho, names),
        body: cliEcho.body,
      });
    }

    // Anchors, so a suite that stopped reaching the server cannot pass by comparing
    // two empty echoes.
    expect(seen[0]?.url).toBe("/echo/v2?page=2&limit=10");
    expect(seen[1]?.body).toBe('{"name": "nova", "tier": "pro"}');
    expect(seen[3]?.headers["authorization"]).toBe("Bearer t0ken");
    expect(seen[4]?.headers["authorization"]).toBe(`Basic ${Buffer.from("ada:lovelace").toString("base64")}`);
    expect(JSON.parse(seen[6]?.body ?? "{}").variables).toEqual({ id: "u-1" });
  }, TIMEOUT);
});

// What this suite deliberately does not compare, because the CLI exposes no view of it:
//   · gRPC proto paths, metadata and message — `ls` reports only name/kind/method/path,
//     and sending needs a live gRPC server. Structure is compared; the rest is not.
//   · `< ./file` bodies — the in-process engine escalates them to the CLI by design, so
//     there is no second reader to disagree with.
//   · Parse *errors* — the CLI reports the first failure for a whole workspace, so a
//     message-for-message comparison would need one workspace per case.
describe("what the CLI exposes no view of", () => {
  it("still parses the gRPC structure the CLI cannot show", () => {
    const document = parseTextDocument("grpc-multi.grpc", AWKWARD["grpc-multi.grpc"]!, "grpc");
    expect(withFileVars(document)[0]!.model.grpc).toMatchObject({
      protoPaths: ["protos/mock.proto"],
      service: "mock.v1.Mock",
      method: "Say",
      metadata: [["x-trace", "mandalo"]],
    });
  });
});

it.skipIf(binary !== null || cliIsRequired())(`parser parity skipped: ${reason}`, () => {
  expect(binary).toBeNull();
});

it.skipIf(binary !== null || !cliIsRequired())(
  "MANDALO_REQUIRE_CLI is set but no CLI exists",
  () => {
    expect.fail(reason);
  },
);
