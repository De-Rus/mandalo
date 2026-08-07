import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { MandaloCli, type LsCollection, type LsRequest } from "../../src/core/cli";
import { parseTextDocument, TextFormatError, withFileVars } from "../../../src/lib/format/httpFormat";
import { runOne } from "../../src/engine/run";
import {
  engineOnlyReason,
  engineOnlyRequestKind,
  textFileKind,
} from "../../../src/lib/format/textFormat";
import type { RequestModel } from "../../../src/lib/format/model";
import { renderFile, renderRequest } from "../../../src/lib/format/render";
import { cliIsRequired, probeCli } from "./support/cliBinary";

// The TypeScript reader in src/lib/format/httpFormat.ts is an interim implementation;
// the Rust parser in crates/core is the reference. This suite is what keeps the stopgap
// honest — every corpus file is parsed by both and the two views must agree.
const { binary, reason } = probeCli();
const TIMEOUT = 120_000;

/** Request formats crates/core reads that src/lib/format does not implement yet. */
const ENGINE_ONLY = /\.(ws|mqtt)(#|$|:)/;

function partition<T>(items: T[], pick: (item: T) => boolean): [T[], T[]] {
  const yes: T[] = [];
  const no: T[] = [];
  for (const item of items) (pick(item) ? yes : no).push(item);
  return [yes, no];
}

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
  // Both form-data spellings, in one file: the readable one Mándalo writes and the
  // literal one an import from REST Client or JetBrains brings.
  "formdata.http": `### Readable form
POST https://api.dev/upload
Content-Type: multipart/form-data

title = Q3 expenses
attachments = < ./files/alpha.txt < ./files/beta.txt
report = < ./files/report.pdf; type=application/x-invoice
bio = <b>not a file</b>

### The older field spelling, still read
POST https://api.dev/upload
Content-Type: multipart/form-data

attachments < ./files/alpha.txt
attachments < ./files/beta.txt

### Boundary form
POST https://api.dev/upload
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name="caption"

two attachments, one field
--WebAppBoundary
Content-Disposition: form-data; name="attachments"; filename="alpha.txt"

< ./files/alpha.txt
--WebAppBoundary--
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
  "grpc-unnamed.grpc": `localhost:50051/pkg.Svc/Method
`,
  "bom.http": `\ufeff### A byte-order mark is not a separator
GET https://api.dev/bom
`,
  "crlf-body.http": "### A CRLF body keeps its own bytes\r\nPOST https://api.dev/x\r\nContent-Type: text/plain\r\n\r\none\r\ntwo\r\n",
  "dup-marker.http": `### Two GraphQL markers
POST https://api.dev/graphql
X-REQUEST-TYPE: GraphQL
X-REQUEST-TYPE: GraphQL

{ ping }
`,
  "broken.grpc": `### Not a call line
not-a-call-line
`,
  "empty.http": "",
  "four-hashes.http": `####
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

/** What `mandalo run` reports for a request it could not send: the addressing facts. */
interface RanView {
  path: string;
  name: string;
  method: string;
  url: string;
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
    else if (
      textFileKind(entry.name) !== undefined ||
      engineOnlyRequestKind(entry.name) !== undefined
    )
      out.push(relPath);
  }
  return out.sort();
}

/** The TypeScript reader's view of a collection directory, shaped like `ls` output. */
function typescriptView(collectionDir: string): { requests: ParsedView[]; skipped: string[] } {
  const requests: ParsedView[] = [];
  const skipped: string[] = [];
  for (const relPath of requestFiles(collectionDir)) {
    const engineOnly = engineOnlyRequestKind(relPath);
    if (engineOnly !== undefined) {
      skipped.push(`${relPath}: ${engineOnlyReason(engineOnly)}`);
      continue;
    }
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
        method: block.model.method,
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

/**
 * `ls` reports no URL, so the one CLI surface that shows the parser's view of it is a
 * run: every request here points at a closed port, fails on connect, and still reports
 * the name, method and URL the Rust reader took off the file.
 */
const ADDRESSED: Record<string, string> = {
  "http.http": `### Named GET
GET http://127.0.0.1:1/one
Accept: application/json

### POST with a body
POST http://127.0.0.1:1/two

{"a": 1}

###
GET http://127.0.0.1:1/unnamed
`,
  "calls.grpc": `### Named call
127.0.0.1:1/pkg.Svc/Named
proto: protos/mock.proto

{"id": "1"}

###
127.0.0.1:1/pkg.Svc/Unnamed
proto: protos/mock.proto
`,
};

function addressedWorkspace(): string {
  const root = mkdtempSync(join(tmpdir(), "mandalo-addr-"));
  const dir = join(root, "collections", "addr");
  mkdirSync(dir, { recursive: true });
  mkdirSync(join(root, "environments"), { recursive: true });
  mkdirSync(join(root, "protos"), { recursive: true });
  writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "ad"\nname = "Addr"\n');
  writeFileSync(join(dir, "collection.toml"), 'schema_version = 1\nid = "addr"\nname = "Addr"\n');
  writeFileSync(
    join(root, "protos", "mock.proto"),
    readFileSync(join(MOCK_WORKSPACE, "protos", "mock.proto"), "utf8"),
  );
  for (const [name, source] of Object.entries(ADDRESSED)) writeFileSync(join(dir, name), source);
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

### JSON body with no declared Content-Type
POST {{baseUrl}}/echo

{"sniffed": true}

### XML body with no declared Content-Type
POST {{baseUrl}}/echo

<user id="1"/>

### Prose body with no declared Content-Type
POST {{baseUrl}}/echo

just words
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
        const all = flatten(collection).map(view);
        // The Rust core reads request formats this reader does not have yet. Those
        // are compared by a different rule below: not "same view", but "never lost".
        const [engineOnly, shared] = partition(all, (r) => ENGINE_ONLY.test(r.path));
        const mine = typescriptView(join(root, "collections", collection.slug));
        expect(sortViews(mine.requests), `collection ${collection.slug} of ${root}`).toEqual(
          sortViews(shared),
        );
        for (const request of engineOnly) {
          const file = request.path.split("#")[0]!;
          expect(
            mine.skipped.some((line) => line.includes(file)),
            `${file} holds requests the Rust core reads, so this reader must report it as skipped rather than drop it`,
          ).toBe(true);
        }
        refused.push(...mine.skipped.filter((line) => !ENGINE_ONLY.test(line)));
      }
      // A file one reader refuses the other must refuse too, with the same sentence.
      expect(refused.sort()).toEqual([...listed.skipped].sort());
    }, TIMEOUT);
  }

  it("takes the same name, method and URL off every block, gRPC included", async () => {
    const root = addressedWorkspace();
    temporary.push(root);
    const report = await cli().run(root, "addr");
    const fromRust: RanView[] = report.requests
      .map(({ path, name, method, url }) => ({ path, name, method, url }))
      .sort((a, b) => a.path.localeCompare(b.path));

    const mine: RanView[] = [];
    for (const relPath of requestFiles(join(root, "collections", "addr"))) {
      const source = readFileSync(join(root, "collections", "addr", relPath), "utf8");
      for (const block of withFileVars(
        parseTextDocument(relPath, source, textFileKind(relPath)!),
      )) {
        mine.push({
          path: `${relPath}#${block.index}`,
          name: block.name,
          method: block.model.method,
          url: block.model.url,
        });
      }
    }

    expect(mine.sort((a, b) => a.path.localeCompare(b.path))).toEqual(fromRust);
    expect(fromRust.map((entry) => entry.method)).toEqual([
      "POST",
      "POST",
      "GET",
      "POST",
      "GET",
    ]);
  }, TIMEOUT);

  // What the browser build writes when someone saves a request. The CLI has to read it
  // back as the same request, or a workspace authored in a web page is a workspace the
  // rest of the product cannot open.
  it("reads back what the shared writer writes, block for block", async () => {
    const written: RequestModel[] = [
      {
        schemaVersion: 1,
        id: "a",
        name: "Written by the browser",
        kind: "http",
        method: "POST",
        url: "http://127.0.0.1:1/users",
        headers: [["Accept", "application/json"]],
        auth: { type: "bearer", token: "t0ken" },
        body: '{"name": "nova"}',
        scripts: { post: 'pm.test("ok", function () {});' },
        tests: [],
        captures: [],
      },
      {
        schemaVersion: 1,
        id: "b",
        name: "GraphQL by the browser",
        kind: "graphql",
        method: "POST",
        url: "http://127.0.0.1:1/graphql",
        headers: [],
        auth: { type: "none" },
        graphql: { query: "query Q($id: ID!) { user(id: $id) { id } }", variables: '{"id": "u-1"}' },
        scripts: {},
        tests: [],
        captures: [],
      },
      {
        schemaVersion: 1,
        id: "d",
        name: "Form by the browser",
        kind: "http",
        method: "POST",
        url: "http://127.0.0.1:1/upload",
        headers: [],
        auth: { type: "none" },
        formdata: [
          { key: "title", value: "Q3 expenses" },
          { key: "attachments", files: ["files/alpha.txt", "files/beta.txt"] },
          { key: "report", files: ["files/report.pdf"], contentType: "application/x-invoice" },
        ],
        scripts: {},
        tests: [],
        captures: [],
      },
      {
        schemaVersion: 1,
        id: "c",
        name: "Call by the browser",
        kind: "grpc",
        method: "POST",
        url: "127.0.0.1:1",
        headers: [],
        auth: { type: "none" },
        grpc: {
          protoPaths: ["protos/mock.proto"],
          service: "pkg.Svc",
          method: "Written",
          message: '{"text": "hola"}',
          metadata: [["x-trace", "1"]],
        },
        scripts: {},
        tests: [],
        captures: [],
      },
    ];

    const root = mkdtempSync(join(tmpdir(), "mandalo-written-"));
    temporary.push(root);
    const dir = join(root, "collections", "written");
    mkdirSync(dir, { recursive: true });
    mkdirSync(join(root, "environments"), { recursive: true });
    mkdirSync(join(root, "protos"), { recursive: true });
    writeFileSync(join(root, "mandalo.toml"), 'schema_version = 1\nid = "wr"\nname = "Wr"\n');
    writeFileSync(join(dir, "collection.toml"), 'schema_version = 1\nid = "written"\nname = "W"\n');
    writeFileSync(
      join(root, "protos", "mock.proto"),
      readFileSync(join(MOCK_WORKSPACE, "protos", "mock.proto"), "utf8"),
    );
    writeFileSync(join(dir, "http.http"), renderFile(written.slice(0, 3)));
    writeFileSync(join(dir, "call.grpc"), renderRequest(written[3]!));

    const listed = await cli().ls(root);
    expect(listed.skipped).toEqual([]);
    expect(flatten(listed.collections[0]!).map(view)).toEqual([
      {
        path: "call.grpc#0",
        id: "call-grpc-0",
        name: "Call by the browser",
        kind: "grpc",
        method: "POST",
      },
      {
        path: "http.http#0",
        id: "http-http-0",
        name: "Written by the browser",
        kind: "http",
        method: "POST",
      },
      {
        path: "http.http#1",
        id: "http-http-1",
        name: "GraphQL by the browser",
        kind: "graphql",
        method: "POST",
      },
      {
        path: "http.http#2",
        id: "http-http-2",
        name: "Form by the browser",
        kind: "http",
        method: "POST",
      },
    ]);

    // And the writer's own output parses back to the request it was given.
    const reread = withFileVars(
      parseTextDocument("http.http", renderFile(written.slice(0, 3)), "http"),
    );
    expect(reread[0]!.model.body).toBe('{"name": "nova"}');
    expect(reread[0]!.model.auth).toEqual({ type: "bearer", token: "t0ken" });
    expect(reread[1]!.model.graphql).toEqual(written[1]!.graphql);
    expect(reread[2]!.model.formdata).toEqual(written[2]!.formdata);
    expect(reread[2]!.model.headers).toEqual([]);
    const call = withFileVars(parseTextDocument("call.grpc", renderRequest(written[3]!), "grpc"));
    expect(call[0]!.model.grpc).toEqual(written[3]!.grpc);
    expect(call[0]!.model.url).toBe("127.0.0.1:1");
  }, TIMEOUT);

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
    expect(blocks.length).toBe(11);
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
    // A body with no declared Content-Type is where the two sniffers have to agree.
    expect(seen[8]?.headers["content-type"]).toBe("application/json");
    expect(seen[9]?.headers["content-type"]).toBe("application/xml");
    expect(seen[10]?.headers["content-type"]).toBe("text/plain");
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
