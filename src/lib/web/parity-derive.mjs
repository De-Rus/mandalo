// Runs parity.fixtures.json through the Rust CLI against a local echo server and
// checks every pinned `wire` block against what actually reached the socket. This
// is where those pins come from: nothing in them is read off the Rust source.
//
//   node src/lib/web/parity-derive.mjs [path/to/mandalo]
//
// A mismatch prints both sides and exits 1. Pass --write to adopt what Rust sent.

import { execFile } from "node:child_process";
import { createServer } from "node:http";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../..");
const fixturesPath = join(here, "parity.fixtures.json");
const fixtures = JSON.parse(readFileSync(fixturesPath, "utf8"));
const write = process.argv.includes("--write");

// An older build is a different engine, so when both exist the freshest one wins.
const cli =
  process.argv.slice(2).find((argument) => !argument.startsWith("--")) ??
  [join(repoRoot, "target", "release", "mandalo"), join(repoRoot, "target", "debug", "mandalo")]
    .filter(existsSync)
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
if (!cli || !existsSync(cli)) {
  process.stderr.write("no mandalo CLI: build one with `make cli` or pass its path\n");
  process.exit(2);
}

const PLUMBING = new Set(["accept", "accept-encoding", "connection", "content-length", "host"]);
const captured = [];

const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const headers = [];
    for (let i = 0; i < request.rawHeaders.length; i += 2)
      headers.push([request.rawHeaders[i].toLowerCase(), request.rawHeaders[i + 1]]);
    const body = Buffer.concat(chunks).toString("utf8");
    captured.push({
      method: request.method,
      path: request.url,
      headers: headers.filter(([key]) => !PLUMBING.has(key)).sort((a, b) => a[0].localeCompare(b[0])),
      body: body === "" ? null : body,
    });
    response.writeHead(200, { "content-type": "application/json" });
    response.end('{"ok":true}');
  });
});

await new Promise((done) => server.listen(0, "127.0.0.1", done));
const origin = `http://127.0.0.1:${server.address().port}`;
const base = (text) => text.split("{{BASE}}").join(origin);

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
  `name = "test"\n[vars]\n${Object.entries(fixtures.vars)
    .map(([key, value]) => `${key} = ${JSON.stringify(value)}\n`)
    .join("")}`,
);
for (const fixture of fixtures.fixtures)
  writeFileSync(join(root, "collections", "api", `${fixture.name}.toml`), base(fixture.toml));

let report = "{}";
try {
  const { stdout } = await promisify(execFile)(
    cli,
    ["run", "api", "--workspace", root, "--env", "test", "--reporter", "json"],
    { maxBuffer: 64 * 1024 * 1024 },
  );
  report = stdout;
} catch (error) {
  report = error.stdout || "{}";
  if (captured.length === 0) process.stderr.write(`${error.stderr ?? error.message}\n`);
}

await new Promise((done) => server.close(done));
rmSync(root, { recursive: true, force: true });

if (JSON.parse(report).total === 0) {
  process.stderr.write(
    `${cli} ran none of these requests — the CLI's on-disk request format has moved on and ` +
      "these fixtures have to be rewritten in it before Rust can be compared again\n",
  );
  process.exit(2);
}

const byCase = new Map(
  captured.map((request) => [new URL(request.path, origin).searchParams.get("case") ?? "", request]),
);

const unpin = (text) => (typeof text === "string" && text.startsWith("~") ? text.slice(1) : null);

function agrees(actual, pinned) {
  if (pinned === null || actual === null) return actual === pinned;
  const pattern = unpin(pinned);
  return pattern === null ? actual === pinned : new RegExp(pattern).test(actual);
}

function diff(fixture, actual) {
  const wire = JSON.parse(base(JSON.stringify(fixture.wire)));
  const problems = [];
  if (actual.method !== wire.method) problems.push(`method ${actual.method} != ${wire.method}`);
  if (!agrees(actual.path, wire.path)) problems.push(`path ${actual.path} != ${wire.path}`);
  if (!agrees(actual.body, wire.body)) problems.push(`body ${actual.body} != ${wire.body}`);
  const names = actual.headers.map(([key]) => key).join(",");
  const pinnedNames = wire.headers.map(([key]) => key).join(",");
  if (names !== pinnedNames) problems.push(`headers ${names} != ${pinnedNames}`);
  else
    actual.headers.forEach(([key, value], index) => {
      if (!agrees(value, wire.headers[index][1]))
        problems.push(`header ${key}: ${value} != ${wire.headers[index][1]}`);
    });
  return problems;
}

let failed = 0;
for (const fixture of fixtures.fixtures) {
  const actual = byCase.get(fixture.name);
  if (!actual) {
    process.stdout.write(`✗ ${fixture.name}: the CLI never sent it\n`);
    failed += 1;
    continue;
  }
  const problems = diff(fixture, actual);
  if (problems.length === 0) {
    process.stdout.write(`✓ ${fixture.name}\n`);
    continue;
  }
  failed += 1;
  process.stdout.write(`✗ ${fixture.name}\n  ${problems.join("\n  ")}\n`);
  if (write)
    fixture.wire = JSON.parse(JSON.stringify(actual).split(origin).join("{{BASE}}"));
}

if (write) {
  writeFileSync(fixturesPath, `${JSON.stringify(fixtures, null, 2)}\n`);
  process.stdout.write("pins rewritten from this run — re-read them before committing\n");
}

process.exit(failed === 0 || write ? 0 : 1);
