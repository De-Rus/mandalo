// INTERIM: a hand-written reader for the `.http` and `.grpc` formats, so the tree,
// the CodeLens positions, the diagnostics and the in-process engine work without
// spawning the CLI. It is to be replaced by a WASM build of `crates/core`, which is
// the reference implementation; until then `test/unit/parserParity.test.ts` parses a
// corpus with both and fails the build the moment they disagree.
import type { Auth, RequestModel } from "./model";

export const HTTP_EXTENSIONS = ["http", "rest"];

export type TextFileKind = "http" | "grpc";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];
const HTTP_VERSIONS = ["HTTP/1.1", "HTTP/1.0", "HTTP/2.0", "HTTP/2", "HTTP/3"];
const GRAPHQL_MARKER = "X-REQUEST-TYPE";
const PROTO_KEY = "proto";
const GRPC_RESERVED = ["url", "service", "method", "message", "proto-path", "protos", "import"];

export class TextFormatError extends Error {
  constructor(
    message: string,
    readonly line: number,
    readonly offset: number,
    readonly length: number,
  ) {
    super(message);
    this.name = "TextFormatError";
  }
}

export interface SourceLine {
  text: string;
  start: number;
  end: number;
  number: number;
}

export interface Segment {
  lines: SourceLine[];
  name: string | null;
  /** Zero-based line of the `###` separator, or 0 for a file that has none. */
  lineIndex: number;
}

export interface ParsedBlock {
  index: number;
  name: string;
  lineIndex: number;
  model: RequestModel;
}

export interface ParsedDocument {
  vars: [string, string][];
  blocks: ParsedBlock[];
}

export function sourceLines(source: string): SourceLine[] {
  const out: SourceLine[] = [];
  let start = 0;
  while (start < source.length) {
    const at = source.indexOf("\n", start);
    const end = at === -1 ? source.length : at;
    const textEnd = source.slice(start, end).endsWith("\r") ? end - 1 : end;
    out.push({ text: source.slice(start, textEnd), start, end: textEnd, number: out.length + 1 });
    start = end + 1;
  }
  return out;
}

export function isSeparator(text: string): boolean {
  return text.startsWith("###");
}

export function isComment(text: string): boolean {
  const trimmed = text.trimStart();
  return (trimmed.startsWith("#") && !isSeparator(trimmed)) || trimmed.startsWith("//");
}

export function commentBody(text: string): string {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("//")) return trimmed.slice(2).trim();
  if (trimmed.startsWith("#")) return trimmed.slice(1).trim();
  return trimmed.trim();
}

export function varDefinition(text: string): [string, string] | undefined {
  const rest = text.trimStart();
  if (!rest.startsWith("@")) return undefined;
  const at = rest.indexOf("=");
  if (at === -1) return undefined;
  return [rest.slice(1, at).trim(), rest.slice(at + 1).trim()];
}

function validVarName(name: string): boolean {
  return name !== "" && /^[A-Za-z0-9_.-]+$/.test(name);
}

function validHeaderName(name: string): boolean {
  return name !== "" && /^[A-Za-z0-9\-_.{}$]+$/.test(name);
}

export function segmentsOf(lines: SourceLine[]): Segment[] {
  const out: Segment[] = [{ lines: [], name: null, lineIndex: 0 }];
  for (const line of lines) {
    if (isSeparator(line.text)) {
      out.push({ lines: [], name: line.text.slice(3).trim(), lineIndex: line.number - 1 });
      continue;
    }
    out[out.length - 1]!.lines.push(line);
  }
  return out;
}

/** Comments, blanks and `@var` lines declare no request — the file header, or a stray `###`. */
export function isDeclarative(segment: Segment): boolean {
  return segment.lines.every(
    (line) =>
      line.text.trim() === "" || isComment(line.text) || varDefinition(line.text) !== undefined,
  );
}

function at(line: SourceLine, message: string): TextFormatError {
  return new TextFormatError(
    `line ${line.number}: ${message}`,
    line.number,
    line.start,
    Math.max(line.end - line.start, 1),
  );
}

function slug(value: string): string {
  let out = "";
  for (const char of value) {
    if (/[A-Za-z0-9]/.test(char)) out += char.toLowerCase();
    else if (!out.endsWith("-")) out += "-";
  }
  const trimmed = out.replace(/^-+|-+$/g, "");
  return trimmed === "" ? "request" : trimmed;
}

export function dedent(text: string): string {
  const body = text.replace(/^[\r\n]+/, "").replace(/[\r\n]+$/, "");
  const lines = body.split("\n").map((line) => (line.endsWith("\r") ? line.slice(0, -1) : line));
  const widths = lines
    .filter((line) => line.trim() !== "")
    .map((line) => line.length - line.trimStart().length);
  const indent = widths.length === 0 ? 0 : Math.min(...widths);
  return lines
    .map((line) => (line.length > indent ? line.slice(indent) : line.trimStart()))
    .join("\n")
    .trimEnd();
}

/** Replaces only the templates the map knows, leaving the rest for the environment. */
export function substitute(text: string, vars: Map<string, string>): string {
  if (vars.size === 0 || !text.includes("{{")) return text;
  let out = "";
  let rest = text;
  for (;;) {
    const open = rest.indexOf("{{");
    if (open === -1) break;
    out += rest.slice(0, open);
    const after = rest.slice(open + 2);
    const close = after.indexOf("}}");
    if (close === -1) {
      out += "{{";
      rest = after;
      continue;
    }
    const name = after.slice(0, close).trim();
    const value = vars.get(name);
    out += value === undefined ? `{{${after.slice(0, close)}}}` : value;
    rest = after.slice(close + 2);
  }
  return out + rest;
}

export function resolveVars(vars: readonly [string, string][]): Map<string, string> {
  const out = new Map<string, string>();
  for (const [name, value] of vars) out.set(name, substitute(value, out));
  return out;
}

interface Preamble {
  vars: [string, string][];
  named: string | null;
  pre: string | undefined;
  consumed: number;
}

function readScript(lines: SourceLine[], index: number): { text: string; next: number } {
  const first = lines[index]!;
  const open = first.text.indexOf("{%");
  if (open === -1) throw at(first, "a script block must open with `{%`");
  const head = first.text.slice(open + 2);
  const closeOnFirst = head.indexOf("%}");
  if (closeOnFirst !== -1) return { text: head.slice(0, closeOnFirst), next: index + 1 };
  const collected = [head];
  for (let cursor = index + 1; cursor < lines.length; cursor += 1) {
    const line = lines[cursor]!;
    const close = line.text.indexOf("%}");
    if (close !== -1) {
      collected.push(line.text.slice(0, close));
      return { text: collected.join("\n"), next: cursor + 1 };
    }
    collected.push(line.text);
  }
  throw at(first, "this script block is never closed with `%}`");
}

function readPreamble(lines: SourceLine[]): Preamble {
  const out: Preamble = { vars: [], named: null, pre: undefined, consumed: 0 };
  let index = 0;
  while (index < lines.length) {
    const line = lines[index]!;
    const trimmed = line.text.trim();
    if (trimmed === "") {
      index += 1;
      continue;
    }
    if (isComment(line.text)) {
      const body = commentBody(line.text);
      if (body.startsWith("@")) {
        const directive = body.slice(1);
        const space = directive.search(/\s/);
        const key = space === -1 ? directive : directive.slice(0, space);
        const value = space === -1 ? "" : directive.slice(space).trim();
        if (key.toLowerCase() !== "name") {
          throw at(
            line,
            `Mándalo does not support the \`@${key}\` directive — a .http file it writes carries no directives beyond \`@name\``,
          );
        }
        if (value === "") throw at(line, "`@name` needs a name");
        out.named = value;
      }
      index += 1;
      continue;
    }
    const definition = varDefinition(line.text);
    if (definition) {
      if (!validVarName(definition[0])) {
        throw at(line, `"${definition[0]}" is not a valid variable name`);
      }
      out.vars.push(definition);
      index += 1;
      continue;
    }
    if (trimmed.startsWith("<") && trimmed.slice(1).trimStart().startsWith("{%")) {
      const script = readScript(lines, index);
      out.pre = dedent(script.text);
      index = script.next;
      continue;
    }
    break;
  }
  out.consumed = index;
  return out;
}

interface RequestLine {
  method: string;
  url: string;
  next: number;
}

function readRequestLine(lines: SourceLine[], index: number): RequestLine {
  const line = lines[index]!;
  const body = line.text.trim();
  let method = "GET";
  let rest = body;
  const space = body.search(/\s/);
  if (space !== -1) {
    const first = body.slice(0, space);
    const upper = first.toUpperCase();
    if (METHODS.includes(upper)) {
      method = upper;
      rest = body.slice(space).trimStart();
    } else if (first.length <= 12 && /^[A-Za-z]+$/.test(first)) {
      throw at(line, `"${first}" is not an HTTP method — Mándalo supports ${METHODS.join(", ")}`);
    }
  }
  let url = rest;
  for (const version of HTTP_VERSIONS) {
    if (url.endsWith(version)) {
      url = url.slice(0, url.length - version.length).trimEnd();
      break;
    }
  }
  if (url === "") throw at(line, "this request line has no URL");

  let next = index + 1;
  while (next < lines.length) {
    const candidate = lines[next]!;
    if (candidate.text.trim() === "" || isComment(candidate.text)) break;
    if (!candidate.text.startsWith(" ") && !candidate.text.startsWith("\t")) break;
    url += candidate.text.trim();
    next += 1;
  }
  return { method, url, next };
}

interface HeaderLine {
  name: string;
  value: string;
  line: SourceLine;
}

function readHeaders(lines: SourceLine[], from: number): { headers: HeaderLine[]; next: number } {
  const headers: HeaderLine[] = [];
  let index = from;
  while (index < lines.length) {
    const line = lines[index]!;
    const trimmed = line.text.trimStart();
    if (trimmed === "") break;
    if (isComment(line.text)) {
      index += 1;
      continue;
    }
    if (trimmed.startsWith(">") || trimmed.startsWith("<")) break;
    const colon = line.text.indexOf(":");
    if (colon === -1) {
      throw at(
        line,
        `expected \`Name: value\` or a blank line before the body, found "${line.text.trim()}"`,
      );
    }
    const name = line.text.slice(0, colon).trim();
    if (!validHeaderName(name)) throw at(line, `"${name}" is not a valid header name`);
    headers.push({ name, value: line.text.slice(colon + 1).trim(), line });
    index += 1;
  }
  return { headers, next: index };
}

interface Tail {
  body: string | undefined;
  bodyLine: SourceLine | undefined;
  post: string | undefined;
}

function readTail(lines: SourceLine[], from: number): Tail {
  const out: Tail = { body: undefined, bodyLine: undefined, post: undefined };
  const collected: string[] = [];
  let index = from;
  while (index < lines.length) {
    const line = lines[index]!;
    const trimmed = line.text.trimStart();
    if (trimmed.startsWith(">")) {
      if (!trimmed.slice(1).trimStart().startsWith("{%")) {
        throw at(line, "Mándalo runs inline `> {% … %}` scripts only, not a script file reference");
      }
      const script = readScript(lines, index);
      out.post = dedent(script.text);
      index = script.next;
      continue;
    }
    if (out.post !== undefined) {
      if (trimmed === "") {
        index += 1;
        continue;
      }
      throw at(line, "nothing may follow a `> {% … %}` response script inside a request");
    }
    if (trimmed === "" && collected.length === 0) {
      index += 1;
      continue;
    }
    if (collected.length === 0) out.bodyLine = line;
    collected.push(line.text);
    index += 1;
  }
  const body = collected.join("\n").replace(/\s+$/, "");
  if (body !== "") out.body = body;
  else out.bodyLine = undefined;
  return out;
}

function workspaceRelative(line: SourceLine, raw: string, what: string): string {
  const cleaned = raw.trim();
  if (cleaned === "") throw at(line, `${what} needs a path`);
  if (cleaned.startsWith("/") || cleaned.startsWith("\\") || cleaned.includes(":")) {
    throw at(line, `${what} must be a workspace-relative path, not "${cleaned}"`);
  }
  const normalized = cleaned.replace(/^\.\//, "");
  if (normalized.split(/[/\\]/).some((part) => part === ".." || part === "")) {
    throw at(line, `${what} must stay inside the workspace: "${cleaned}"`);
  }
  return normalized;
}

/** `Authorization: Bearer x` / `Basic user:pass` carry what a typed auth block carries. */
function authFromHeader(value: string): Auth | undefined {
  const space = value.search(/\s/);
  if (space === -1) return undefined;
  const scheme = value.slice(0, space).toLowerCase();
  const rest = value.slice(space).trim();
  if (rest === "") return undefined;
  if (scheme === "bearer") return { type: "bearer", token: rest };
  if (scheme === "basic") {
    const colon = rest.indexOf(":");
    if (colon === -1) return undefined;
    return {
      type: "basic",
      username: rest.slice(0, colon).trim(),
      password: rest.slice(colon + 1).trim(),
    };
  }
  return undefined;
}

/** The document, then a blank line, then the variables object. */
function splitGraphql(text: string): { query: string; variables: string } {
  const lines = text.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    if (lines[index]!.trim() !== "") continue;
    const rest = lines.slice(index + 1).join("\n");
    if (rest.trimStart().startsWith("{")) {
      return { query: lines.slice(0, index).join("\n").trimEnd(), variables: rest.trim() };
    }
  }
  return { query: text.trimEnd(), variables: "" };
}

function blockName(segment: Segment, named: string | null, fallback: string): string {
  if (segment.name !== null && segment.name !== "") return segment.name;
  return named ?? fallback;
}

function parseHttpBlock(stem: string, index: number, segment: Segment): ParsedBlock {
  const lines = segment.lines;
  const preamble = readPreamble(lines);
  if (preamble.consumed >= lines.length) {
    const line = lines[0] ?? { text: "", start: 0, end: 1, number: segment.lineIndex + 1 };
    throw at(line, "this block has no request line — a request needs `METHOD url`");
  }
  const requestLine = readRequestLine(lines, preamble.consumed);
  const { headers, next } = readHeaders(lines, requestLine.next);
  const tail = readTail(lines, next);

  const badMarker = headers.find(
    (header) =>
      header.name.toUpperCase() === GRAPHQL_MARKER && header.value.toLowerCase() !== "graphql",
  );
  if (badMarker) {
    throw at(
      badMarker.line,
      `${GRAPHQL_MARKER} only marks a GraphQL request; "${badMarker.value}" means nothing to Mándalo`,
    );
  }
  const isGraphql = headers.some((header) => header.name.toUpperCase() === GRAPHQL_MARKER);

  let auth: Auth = { type: "none" };
  const kept: [string, string][] = [];
  for (const header of headers) {
    if (header.name.toUpperCase() === GRAPHQL_MARKER) continue;
    if (auth.type === "none" && header.name.toLowerCase() === "authorization") {
      const found = authFromHeader(header.value);
      if (found) {
        auth = found;
        continue;
      }
    }
    kept.push([header.name, header.value]);
  }

  let bodyFile: string | undefined;
  if (tail.body !== undefined && tail.bodyLine !== undefined) {
    const opener = tail.body.trimStart();
    if (opener.startsWith("<@")) {
      throw at(
        tail.bodyLine,
        "Mándalo does not support `<@` file bodies — use `<` and keep the variables in the request",
      );
    }
    if (opener.startsWith("< ") || opener.startsWith("<\t")) {
      if (tail.body.split("\n").filter((line) => line.trim() !== "").length > 1) {
        throw at(tail.bodyLine, "a `< file` body must be the whole body — no text may follow it");
      }
      bodyFile = workspaceRelative(tail.bodyLine, opener.slice(1), "the body file");
    }
  }

  const kind = isGraphql ? "graphql" : "http";
  const model: RequestModel = {
    schemaVersion: 1,
    id: `${slug(stem)}-${index}`,
    name: blockName(segment, preamble.named, `${requestLine.method} ${requestLine.url}`),
    kind,
    method: requestLine.method,
    url: requestLine.url,
    headers: kept,
    auth,
    scripts: {},
    tests: [],
    captures: [],
  };
  if (preamble.pre !== undefined) model.scripts.pre = preamble.pre;
  if (tail.post !== undefined) model.scripts.post = tail.post;
  if (bodyFile !== undefined) model.bodyFile = bodyFile;
  else if (kind === "graphql") model.graphql = splitGraphql(tail.body ?? "");
  else if (tail.body !== undefined) model.body = tail.body;

  return { index, name: model.name, lineIndex: segment.lineIndex, model };
}

function parseGrpcBlock(stem: string, index: number, segment: Segment): ParsedBlock {
  const lines = segment.lines;
  const preamble = readPreamble(lines);
  if (preamble.consumed >= lines.length) {
    const line = lines[0] ?? { text: "", start: 0, end: 1, number: segment.lineIndex + 1 };
    throw at(line, "this block has no call line — a request needs `target/package.Service/Method`");
  }
  const callLine = lines[preamble.consumed]!;
  const body = callLine.text.trim();
  const lastSlash = body.lastIndexOf("/");
  if (lastSlash === -1) {
    throw at(callLine, `expected \`target/package.Service/Method\`, found "${body}"`);
  }
  const head = body.slice(0, lastSlash);
  const method = body.slice(lastSlash + 1);
  const secondSlash = head.lastIndexOf("/");
  if (secondSlash === -1) {
    throw at(callLine, `"${body}" names no target — write \`target/package.Service/Method\``);
  }
  const target = head.slice(0, secondSlash);
  const service = head.slice(secondSlash + 1);
  for (const [what, value] of [
    ["target", target],
    ["service", service],
    ["method", method],
  ] as const) {
    if (value.trim() === "") throw at(callLine, `this call line has no ${what}`);
    if (/\s/.test(value)) throw at(callLine, `the ${what} must not contain whitespace: "${value}"`);
  }

  const { headers, next } = readHeaders(lines, preamble.consumed + 1);
  const tail = readTail(lines, next);

  const protoPaths: string[] = [];
  const metadata: [string, string][] = [];
  for (const header of headers) {
    const key = header.name.toLowerCase();
    if (key === PROTO_KEY) {
      protoPaths.push(workspaceRelative(header.line, header.value, "a `proto:` path"));
      continue;
    }
    if (key.startsWith("grpc-")) {
      throw at(
        header.line,
        `gRPC reserves the \`grpc-\` metadata prefix for the protocol itself, so "${header.name}" cannot be sent`,
      );
    }
    if (GRPC_RESERVED.includes(key)) {
      throw at(
        header.line,
        `"${header.name}" reads like a Mándalo directive but would be sent as gRPC metadata — the only reserved key is \`proto:\`, and the call target lives on the request line`,
      );
    }
    metadata.push([header.name, header.value]);
  }

  const model: RequestModel = {
    schemaVersion: 1,
    id: `${slug(stem)}-${index}`,
    name: blockName(segment, preamble.named, body),
    kind: "grpc",
    method: "GRPC",
    url: body,
    headers: [],
    auth: { type: "none" },
    grpc: { protoPaths, service, method, message: tail.body ?? "{}", metadata },
    scripts: {},
    tests: [],
    captures: [],
  };
  if (preamble.pre !== undefined) model.scripts.pre = preamble.pre;
  if (tail.post !== undefined) model.scripts.post = tail.post;
  return { index, name: model.name, lineIndex: segment.lineIndex, model };
}

/**
 * `stem` seeds the request ids — pass the file's path inside the collection so two
 * files cannot mint the same id, exactly as the Rust parser does.
 */
export function parseTextDocument(
  stem: string,
  source: string,
  fileKind: TextFileKind,
): ParsedDocument {
  const segments = segmentsOf(sourceLines(source));
  const vars: [string, string][] = [];
  const blocks: ParsedBlock[] = [];
  for (const segment of segments) {
    const preamble = readPreamble(segment.lines);
    vars.push(...preamble.vars);
    // A separator with no name and nothing under it is how people close a file; a
    // *named* one that declares no request is a mistake worth saying.
    if (isDeclarative(segment) && (segment.name ?? "") === "" && preamble.named === null) continue;
    blocks.push(
      fileKind === "grpc"
        ? parseGrpcBlock(stem, blocks.length, segment)
        : parseHttpBlock(stem, blocks.length, segment),
    );
  }
  return { vars, blocks };
}

/** The request the runner sends: the file's own `@vars` applied, so a local name wins. */
export function withFileVars(document: ParsedDocument): ParsedBlock[] {
  const vars = resolveVars(document.vars);
  if (vars.size === 0) return document.blocks;
  return document.blocks.map((block) => {
    const model = { ...block.model };
    model.url = substitute(model.url, vars);
    model.headers = model.headers.map(([k, v]) => [substitute(k, vars), substitute(v, vars)]);
    if (model.body !== undefined) model.body = substitute(model.body, vars);
    if (model.bodyFile !== undefined) model.bodyFile = substitute(model.bodyFile, vars);
    if (model.graphql) {
      model.graphql = {
        query: substitute(model.graphql.query, vars),
        variables: substitute(model.graphql.variables, vars),
      };
    }
    if (model.auth.type === "bearer") {
      model.auth = { type: "bearer", token: substitute(model.auth.token, vars) };
    } else if (model.auth.type === "basic") {
      model.auth = {
        type: "basic",
        username: substitute(model.auth.username, vars),
        password: substitute(model.auth.password, vars),
      };
    }
    if (model.grpc) {
      model.grpc = {
        protoPaths: model.grpc.protoPaths.map((path) => substitute(path, vars)),
        service: model.grpc.service,
        method: model.grpc.method,
        message: substitute(model.grpc.message, vars),
        metadata: model.grpc.metadata.map(([k, v]) => [substitute(k, vars), substitute(v, vars)]),
      };
    }
    return { ...block, model };
  });
}
