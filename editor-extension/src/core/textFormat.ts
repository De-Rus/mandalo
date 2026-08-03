import type { RequestKind } from "./model";

const HTTP_EXTENSIONS = ["http", "rest"];
const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];
const HTTP_VERSIONS = ["HTTP/1.1", "HTTP/1.0", "HTTP/2", "HTTP/2.0", "HTTP/3"];
const GRAPHQL_MARKER = "x-request-type";

export type TextFileKind = "http" | "grpc";

export interface TextRequestBlock {
  index: number;
  name: string;
  method: string;
  url: string;
  kind: RequestKind;
  lineNumber: number;
}

interface SourceLine {
  text: string;
  number: number;
}

interface Segment {
  name: string | null;
  lines: SourceLine[];
  lineNumber: number;
}

interface Preamble {
  named: string | null;
  consumed: number;
}

function extensionOf(fsPath: string): string {
  const base = fsPath.slice(Math.max(fsPath.lastIndexOf("/"), fsPath.lastIndexOf("\\")) + 1);
  const dot = base.lastIndexOf(".");
  return dot === -1 ? "" : base.slice(dot + 1).toLowerCase();
}

export function textFileKind(fsPath: string): TextFileKind | undefined {
  const extension = extensionOf(fsPath);
  if (HTTP_EXTENSIONS.includes(extension)) return "http";
  if (extension === "grpc") return "grpc";
  return undefined;
}

export function isTextRequestPath(fsPath: string): boolean {
  return textFileKind(fsPath) !== undefined;
}

/** `auth.http#0` addresses one request inside a file; `ping.toml` addresses a whole file. */
export function requestFilePath(relPath: string): string {
  const hash = relPath.lastIndexOf("#");
  if (hash === -1) return relPath;
  const head = relPath.slice(0, hash);
  return isTextRequestPath(head) ? head : relPath;
}

export function requestPathAt(relPath: string, index: number): string {
  return `${relPath}#${index}`;
}

function toLines(source: string): SourceLine[] {
  const out: SourceLine[] = [];
  let start = 0;
  while (start < source.length) {
    const at = source.indexOf("\n", start);
    const end = at === -1 ? source.length : at;
    const raw = source.slice(start, end);
    out.push({ text: raw.endsWith("\r") ? raw.slice(0, -1) : raw, number: out.length });
    start = end + 1;
  }
  return out;
}

function isSeparator(text: string): boolean {
  return text.startsWith("###");
}

function isComment(text: string): boolean {
  const trimmed = text.trimStart();
  return (trimmed.startsWith("#") && !isSeparator(trimmed)) || trimmed.startsWith("//");
}

function commentBody(text: string): string {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("//")) return trimmed.slice(2).trim();
  if (trimmed.startsWith("#")) return trimmed.slice(1).trim();
  return trimmed.trim();
}

function isVarDefinition(text: string): boolean {
  const trimmed = text.trimStart();
  return trimmed.startsWith("@") && trimmed.includes("=");
}

function toSegments(lines: SourceLine[]): Segment[] {
  const out: Segment[] = [{ name: null, lines: [], lineNumber: 0 }];
  for (const line of lines) {
    if (isSeparator(line.text)) {
      out.push({ name: line.text.slice(3).trim(), lines: [], lineNumber: line.number });
      continue;
    }
    out[out.length - 1]!.lines.push(line);
  }
  return out;
}

/** Blanks, comments and `@var` lines declare no request — that is the file header or a stray `###`. */
function isDeclarative(segment: Segment): boolean {
  return segment.lines.every(
    (line) => line.text.trim() === "" || isComment(line.text) || isVarDefinition(line.text),
  );
}

function skipScript(lines: SourceLine[], index: number): number {
  const first = lines[index]!.text;
  const open = first.indexOf("{%");
  if (open !== -1 && first.indexOf("%}", open + 2) !== -1) return index + 1;
  let cursor = index + 1;
  while (cursor < lines.length) {
    if (lines[cursor]!.text.includes("%}")) return cursor + 1;
    cursor += 1;
  }
  return lines.length;
}

function readPreamble(lines: SourceLine[]): Preamble {
  let named: string | null = null;
  let index = 0;
  while (index < lines.length) {
    const text = lines[index]!.text;
    const trimmed = text.trim();
    if (trimmed === "") {
      index += 1;
      continue;
    }
    if (isComment(text)) {
      const body = commentBody(text);
      if (body.startsWith("@")) {
        const directive = body.slice(1);
        const at = directive.search(/\s/);
        const key = at === -1 ? directive : directive.slice(0, at);
        const value = at === -1 ? "" : directive.slice(at).trim();
        if (key.toLowerCase() === "name" && value !== "") named = value;
      }
      index += 1;
      continue;
    }
    if (isVarDefinition(text)) {
      index += 1;
      continue;
    }
    if (trimmed.startsWith("<")) {
      if (!trimmed.slice(1).trimStart().startsWith("{%")) break;
      index = skipScript(lines, index);
      continue;
    }
    break;
  }
  return { named, consumed: index };
}

function readHttpRequestLine(
  lines: SourceLine[],
  from: number,
): { method: string; url: string; next: number } {
  const text = lines[from]!.text.trim();
  let method = "GET";
  let rest = text;
  const space = text.search(/\s/);
  if (space !== -1 && METHODS.includes(text.slice(0, space).toUpperCase())) {
    method = text.slice(0, space).toUpperCase();
    rest = text.slice(space).trimStart();
  }
  let url = rest;
  for (const version of HTTP_VERSIONS) {
    if (url.endsWith(version)) {
      url = url.slice(0, url.length - version.length).trimEnd();
      break;
    }
  }
  let next = from + 1;
  while (next < lines.length) {
    const candidate = lines[next]!.text;
    if (candidate.trim() === "" || isComment(candidate)) break;
    if (!candidate.startsWith(" ") && !candidate.startsWith("\t")) break;
    url += candidate.trim();
    next += 1;
  }
  return { method, url, next };
}

function hasGraphqlMarker(lines: SourceLine[], from: number): boolean {
  for (let index = from; index < lines.length; index += 1) {
    const text = lines[index]!.text;
    if (text.trim() === "") return false;
    if (isComment(text)) continue;
    const trimmed = text.trimStart();
    if (trimmed.startsWith(">") || trimmed.startsWith("<")) return false;
    const colon = text.indexOf(":");
    if (colon === -1) return false;
    if (
      text.slice(0, colon).trim().toLowerCase() === GRAPHQL_MARKER &&
      text
        .slice(colon + 1)
        .trim()
        .toLowerCase() === "graphql"
    ) {
      return true;
    }
  }
  return false;
}

/**
 * One entry per addressable `###` block, in file order — the index is the `#n`
 * the CLI takes. Enough for CodeLens, the tree and the Test Explorer; the CLI
 * stays the only parser that decides what a request actually sends.
 */
export function scanTextRequests(source: string, fileKind: TextFileKind): TextRequestBlock[] {
  const out: TextRequestBlock[] = [];
  for (const segment of toSegments(toLines(source))) {
    if (isDeclarative(segment)) continue;
    const preamble = readPreamble(segment.lines);
    if (preamble.consumed >= segment.lines.length) continue;

    const line = segment.lines[preamble.consumed]!;
    const grpc = fileKind === "grpc";
    const request = grpc
      ? { method: "GRPC", url: line.text.trim(), next: preamble.consumed + 1 }
      : readHttpRequestLine(segment.lines, preamble.consumed);
    const graphql = !grpc && hasGraphqlMarker(segment.lines, request.next);
    const fallback = grpc ? request.url : `${request.method} ${request.url}`;

    out.push({
      index: out.length,
      name: segment.name || preamble.named || fallback,
      method: request.method,
      url: request.url,
      kind: grpc ? "grpc" : graphql ? "graphql" : "http",
      lineNumber: segment.lineNumber,
    });
  }
  return out;
}
