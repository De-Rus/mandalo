import type { Auth, Capture, GraphqlBody, GrpcRequest, RequestModel, Scripts, TestAssertion } from "./model";
import { parseRequest } from "./parse";

export interface Segment {
  header: string | null;
  start: number;
  end: number;
}

interface Edit {
  start: number;
  end: number;
  text: string;
}

const NEWLINE = /\r?\n/;

function isBareChar(char: string): boolean {
  return /[A-Za-z0-9_-]/.test(char);
}

function skipString(raw: string, at: number): number {
  const quote = raw[at] as string;
  const multi = raw.startsWith(quote.repeat(3), at);
  const literal = quote === "'";
  let pos = at + (multi ? 3 : 1);
  const closer = multi ? quote.repeat(3) : quote;
  while (pos < raw.length) {
    if (!literal && raw[pos] === "\\") {
      pos += 2;
      continue;
    }
    if (raw.startsWith(closer, pos)) return pos + closer.length;
    if (!multi && raw[pos] === "\n") return pos;
    pos += 1;
  }
  return raw.length;
}

/**
 * Table headers only count at the start of a line and outside strings — a body
 * holding `[foo]` inside a multi-line string must not split the document.
 */
export function scanSegments(raw: string): Segment[] {
  const segments: Segment[] = [];
  let current: Segment = { header: null, start: 0, end: raw.length };
  let pos = 0;
  let lineStart = 0;
  let sawContent = false;

  while (pos < raw.length) {
    const char = raw[pos] as string;
    if (char === "\n") {
      pos += 1;
      lineStart = pos;
      sawContent = false;
      continue;
    }
    if (char === " " || char === "\t" || char === "\r") {
      pos += 1;
      continue;
    }
    if (char === "#") {
      while (pos < raw.length && raw[pos] !== "\n") pos += 1;
      continue;
    }
    if (char === '"' || char === "'") {
      sawContent = true;
      pos = skipString(raw, pos);
      continue;
    }
    if (char === "[" && !sawContent) {
      const lineEnd = raw.indexOf("\n", pos);
      const stop = lineEnd === -1 ? raw.length : lineEnd;
      const header = raw.slice(pos, stop).trim();
      if (/^\[\[?[^[\]]+\]\]?$/.test(header)) {
        current.end = lineStart;
        segments.push(current);
        current = { header, start: lineStart, end: raw.length };
        pos = stop;
        continue;
      }
    }
    sawContent = true;
    pos += 1;
  }
  segments.push(current);
  return segments;
}

function valueEnd(raw: string, from: number): number {
  let pos = from;
  let depth = 0;
  while (pos < raw.length) {
    const char = raw[pos] as string;
    if (char === '"' || char === "'") {
      pos = skipString(raw, pos);
      continue;
    }
    if (char === "#" && depth === 0) break;
    if (char === "[" || char === "{") depth += 1;
    if (char === "]" || char === "}") depth -= 1;
    if (char === "\n" && depth <= 0) break;
    pos += 1;
  }
  while (pos > from && /[ \t\r]/.test(raw[pos - 1] as string)) pos -= 1;
  return pos;
}

export interface KeySpan {
  lineStart: number;
  valueStart: number;
  end: number;
}

export function findKey(raw: string, key: string, from: number, to: number): KeySpan | undefined {
  let pos = from;
  while (pos < to) {
    const lineEnd = raw.indexOf("\n", pos);
    const stop = lineEnd === -1 || lineEnd > to ? to : lineEnd;
    const line = raw.slice(pos, stop);
    const trimmed = line.trimStart();
    if (trimmed.startsWith(key) && !isBareChar(trimmed[key.length] ?? "")) {
      const afterKey = pos + line.length - trimmed.length + key.length;
      const eq = raw.indexOf("=", afterKey);
      if (eq !== -1 && raw.slice(afterKey, eq).trim() === "") {
        let valueStart = eq + 1;
        while (raw[valueStart] === " " || raw[valueStart] === "\t") valueStart += 1;
        return { lineStart: pos, valueStart, end: valueEnd(raw, valueStart) };
      }
    }
    if (stop >= to) break;
    pos = stop + 1;
  }
  return undefined;
}

function stable(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (typeof value === "object") {
    const keys = Object.keys(value as Record<string, unknown>).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${stable((value as Record<string, unknown>)[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function same(a: unknown, b: unknown): boolean {
  return stable(a) === stable(b);
}

function quote(value: string): string {
  return JSON.stringify(value);
}

function literal(value: unknown): string {
  return typeof value === "string" ? quote(value) : JSON.stringify(value);
}

function headersLiteral(headers: [string, string][]): string {
  return `[${headers.map(([k, v]) => `[${quote(k)}, ${quote(v)}]`).join(", ")}]`;
}

function authBlock(auth: Auth): string {
  const lines = ["[auth]", `type = ${quote(auth.type)}`];
  for (const [key, value] of Object.entries(auth)) {
    if (key === "type") continue;
    lines.push(`${key} = ${quote(String(value))}`);
  }
  return lines.join("\n");
}

function graphqlBlock(graphql: GraphqlBody): string {
  return ["[graphql]", `query = ${quote(graphql.query)}`, `variables = ${quote(graphql.variables)}`].join("\n");
}

function grpcBlock(grpc: GrpcRequest): string {
  return [
    "[grpc]",
    `protoPaths = [${grpc.protoPaths.map(quote).join(", ")}]`,
    `service = ${quote(grpc.service)}`,
    `method = ${quote(grpc.method)}`,
    `message = ${quote(grpc.message)}`,
    `metadata = ${headersLiteral(grpc.metadata)}`,
  ].join("\n");
}

function scriptsBlock(scripts: Scripts): string {
  const lines = ["[scripts]"];
  if (scripts.pre !== undefined) lines.push(`pre = ${quote(scripts.pre)}`);
  if (scripts.post !== undefined) lines.push(`post = ${quote(scripts.post)}`);
  return lines.join("\n");
}

function testsBlock(tests: TestAssertion[]): string {
  return tests
    .map((test) => {
      const entries = Object.entries(test).filter(([, value]) => value !== undefined);
      return ["[[tests]]", ...entries.map(([key, value]) => `${key} = ${literal(value)}`)].join("\n");
    })
    .join("\n\n");
}

function capturesBlock(captures: Capture[]): string {
  return captures
    .map((capture) =>
      [
        "[[captures]]",
        `from = ${quote(capture.from)}`,
        `into = ${quote(capture.into)}`,
        `scope = ${quote(capture.scope)}`,
      ].join("\n"),
    )
    .join("\n\n");
}

const SCALARS = [
  "schema_version",
  "id",
  "name",
  "kind",
  "method",
  "url",
  "description",
  "body",
] as const;

function scalarOf(model: RequestModel, key: (typeof SCALARS)[number]): unknown {
  switch (key) {
    case "schema_version":
      return model.schemaVersion;
    case "id":
      return model.id;
    case "name":
      return model.name;
    case "kind":
      return model.kind;
    case "method":
      return model.method;
    case "url":
      return model.url;
    case "description":
      return model.description;
    case "body":
      return model.body;
  }
}

function lineEndOf(raw: string, at: number): number {
  const index = raw.indexOf("\n", at);
  return index === -1 ? raw.length : index + 1;
}

function trailingBlank(raw: string, from: number, to: number): number {
  let end = to;
  while (end > from) {
    const lineStart = raw.lastIndexOf("\n", end - 2) + 1;
    if (lineStart < from) break;
    if (raw.slice(lineStart, end).trim() !== "") break;
    end = lineStart;
  }
  return end;
}

function applyEdits(raw: string, edits: Edit[]): string {
  const ordered = [...edits].sort((a, b) => b.start - a.start);
  let out = raw;
  for (const edit of ordered) out = out.slice(0, edit.start) + edit.text + out.slice(edit.end);
  return out;
}

function eol(raw: string): string {
  return NEWLINE.exec(raw)?.[0] === "\r\n" ? "\r\n" : "\n";
}

function segmentsWith(segments: Segment[], header: string): Segment[] {
  return segments.filter((segment) => segment.header === header);
}

function blockEdit(
  raw: string,
  segments: Segment[],
  header: string,
  rendered: string | null,
  edits: Edit[],
  appendix: string[],
): void {
  const found = segmentsWith(segments, header);
  if (found.length === 0) {
    if (rendered !== null) appendix.push(rendered);
    return;
  }
  const first = found[0] as Segment;
  const last = found[found.length - 1] as Segment;
  const end = trailingBlank(raw, first.start, last.end);
  if (rendered === null) {
    edits.push({ start: first.start, end: last.end, text: "" });
    return;
  }
  const keepTail = raw.slice(end, last.end);
  edits.push({ start: first.start, end: last.end, text: rendered + eol(raw) + keepTail });
}

/**
 * Rewrites only the regions whose parsed value actually changed, so an edit made
 * in the GUI shows up in `git diff` as the lines the user touched and nothing else.
 */
export function applyRequestEdit(raw: string, next: RequestModel): string {
  const prev = parseRequest(raw);
  const segments = scanSegments(raw);
  const prelude = segments[0] as Segment;
  const br = eol(raw);
  const edits: Edit[] = [];
  const inserts: string[] = [];

  for (const key of SCALARS) {
    const before = scalarOf(prev, key);
    const after = scalarOf(next, key);
    if (same(before, after)) continue;
    const span = findKey(raw, key, prelude.start, prelude.end);
    if (after === undefined) {
      if (span) edits.push({ start: span.lineStart, end: lineEndOf(raw, span.end), text: "" });
      continue;
    }
    if (span) edits.push({ start: span.valueStart, end: span.end, text: literal(after) });
    else inserts.push(`${key} = ${literal(after)}`);
  }

  if (!same(prev.headers, next.headers)) {
    const span = findKey(raw, "headers", prelude.start, prelude.end);
    const value = headersLiteral(next.headers);
    if (span) edits.push({ start: span.valueStart, end: span.end, text: value });
    else inserts.push(`headers = ${value}`);
  }

  if (inserts.length > 0) {
    const at = trailingBlank(raw, prelude.start, prelude.end);
    edits.push({ start: at, end: at, text: inserts.join(br) + br });
  }

  const appendix: string[] = [];
  if (!same(prev.auth, next.auth)) {
    blockEdit(raw, segments, "[auth]", authBlock(next.auth), edits, appendix);
  }
  if (!same(prev.graphql, next.graphql)) {
    blockEdit(raw, segments, "[graphql]", next.graphql ? graphqlBlock(next.graphql) : null, edits, appendix);
  }
  if (!same(prev.grpc, next.grpc)) {
    blockEdit(raw, segments, "[grpc]", next.grpc ? grpcBlock(next.grpc) : null, edits, appendix);
  }
  if (!same(prev.scripts, next.scripts)) {
    const empty = next.scripts.pre === undefined && next.scripts.post === undefined;
    blockEdit(raw, segments, "[scripts]", empty ? null : scriptsBlock(next.scripts), edits, appendix);
  }
  if (!same(prev.tests, next.tests)) {
    blockEdit(raw, segments, "[[tests]]", next.tests.length === 0 ? null : testsBlock(next.tests), edits, appendix);
  }
  if (!same(prev.captures, next.captures)) {
    blockEdit(
      raw,
      segments,
      "[[captures]]",
      next.captures.length === 0 ? null : capturesBlock(next.captures),
      edits,
      appendix,
    );
  }

  let out = applyEdits(raw, edits);
  if (appendix.length > 0) {
    const padded = out.endsWith(br) ? out : out + br;
    out = `${padded}${br}${appendix.join(br + br)}${br}`;
  }
  return out;
}
