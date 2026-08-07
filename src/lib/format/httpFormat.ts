// INTERIM: the one hand-written reader for the `.http` and `.grpc` formats, shared by
// the browser build and the editor extension, so the tree, the CodeLens positions, the
// diagnostics and both in-process engines work without spawning the CLI. It is to be
// replaced by a WASM build of `crates/core`, which is the reference implementation;
// until then `editor-extension/test/unit/parserParity.test.ts` parses a corpus with
// both and fails the build the moment they disagree.
import type { Auth, FormDataRowModel, RequestModel } from "./model";

export const HTTP_EXTENSIONS = ["http", "rest"];

export type TextFileKind = "http" | "grpc";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];
const HTTP_VERSIONS = ["HTTP/1.1", "HTTP/1.0", "HTTP/2.0", "HTTP/2", "HTTP/3"];
const GRAPHQL_MARKER = "X-REQUEST-TYPE";
const SSE_MEDIA_TYPE = "text/event-stream";
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

export interface Span {
  start: number;
  end: number;
}

export interface Segment {
  lines: SourceLine[];
  name: string | null;
  /** Zero-based line of the `###` separator, or 0 for a file that has none. */
  lineIndex: number;
  /**
   * From the first byte of the `###` line (or of the file) to the last byte of the
   * segment, so a whole request can be cut out of the file.
   */
  span: Span;
}

export interface ParsedBlock {
  index: number;
  name: string;
  lineIndex: number;
  span: Span;
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

export function segmentsOf(lines: SourceLine[], length = 0): Segment[] {
  const out: Segment[] = [{ lines: [], name: null, lineIndex: 0, span: { start: 0, end: 0 } }];
  for (const line of lines) {
    if (isSeparator(line.text)) {
      out[out.length - 1]!.span.end = line.start;
      out.push({
        lines: [],
        name: line.text.slice(3).trim(),
        lineIndex: line.number - 1,
        span: { start: line.start, end: line.end },
      });
      continue;
    }
    const last = out[out.length - 1]!;
    last.span.end = line.end;
    last.lines.push(line);
  }
  out[out.length - 1]!.span.end = length;
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
  autoReconnect: boolean | undefined;
  inheritedAuth: boolean;
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
  const out: Preamble = {
    vars: [],
    named: null,
    pre: undefined,
    autoReconnect: undefined,
    inheritedAuth: false,
    consumed: 0,
  };
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
        const directiveKey = key.toLowerCase();
        if (directiveKey === "auth") {
          if (value.toLowerCase() !== "inherited") {
            throw at(
              line,
              "`@auth` only takes `inherited` — write the auth itself as an Authorization header",
            );
          }
          out.inheritedAuth = true;
          index += 1;
          continue;
        }
        if (directiveKey === "reconnect") {
          const setting = value.toLowerCase();
          if (setting !== "on" && setting !== "off" && setting !== "") {
            throw at(line, `\`@reconnect\` is on or off, not "${value}"`);
          }
          out.autoReconnect = setting !== "off";
          index += 1;
          continue;
        }
        if (directiveKey !== "name") {
          throw at(
            line,
            `Mándalo does not support the \`@${key}\` directive — a .http file it writes carries no directives beyond \`@name\`, \`@auth\` and \`@reconnect\``,
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

// The body is a slice of the source, never a re-join of the parsed lines: a CRLF file
// puts `\r\n` between its body lines and those bytes go on the wire, exactly as the
// Rust reader's span does.
function readTail(source: string, lines: SourceLine[], from: number): Tail {
  const out: Tail = { body: undefined, bodyLine: undefined, post: undefined };
  let bodyStart: number | undefined;
  let bodyEnd = 0;
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
    if (trimmed === "" && bodyStart === undefined) {
      index += 1;
      continue;
    }
    if (bodyStart === undefined) {
      bodyStart = line.start;
      out.bodyLine = line;
    }
    bodyEnd = line.end;
    index += 1;
  }
  if (bodyStart === undefined) return out;
  const body = source.slice(bodyStart, bodyEnd).replace(/\s+$/, "");
  if (body !== "") out.body = body;
  else out.bodyLine = undefined;
  return out;
}

/** The Rust twin is `accepts_event_stream`: any Accept alternative naming the type. */
export function acceptsEventStream(value: string): boolean {
  return value
    .split(",")
    .some((part) => (part.split(";")[0] ?? "").trim().toLowerCase() === SSE_MEDIA_TYPE);
}

function headerParam(value: string, name: string): string | undefined {
  for (const part of value.split(";").slice(1)) {
    const eq = part.indexOf("=");
    if (eq === -1) continue;
    if (part.slice(0, eq).trim().toLowerCase() === name) {
      return part
        .slice(eq + 1)
        .trim()
        .replace(/^"|"$/g, "");
    }
  }
  return undefined;
}

function isMultipartFormdata(value: string): boolean {
  return (value.split(";")[0] ?? "").trim().toLowerCase() === "multipart/form-data";
}

/**
 * Which of the two form-data spellings a body is written in, decided by its own first
 * line: a boundary delimiter opens the literal wire format, anything else is the
 * field-per-line form Mándalo writes. The Rust twin is `is_literal_multipart`.
 */
function isLiteralMultipart(body: string): boolean {
  const first = body.split("\n").find((line) => line.trim() !== "");
  return first !== undefined && first.trim().startsWith("--");
}

/**
 * The path in a `= < ./path` value, if that is what the value is. `<` opens a file
 * reference only when whitespace or a `.` follows it, so `k = <b>bold</b>` stays the
 * text it looks like. The Rust twin is `form_file_ref`.
 */
function formFileRef(value: string): string | undefined {
  if (!value.startsWith("<")) return undefined;
  const rest = value.slice(1);
  if (rest === "" || rest.startsWith(" ") || rest.startsWith("\t") || rest.startsWith(".")) {
    return rest;
  }
  return undefined;
}

/** The `; type=…` a file field may carry, split off the path(s) it follows. */
function splitFileRef(
  line: SourceLine,
  rest: string,
): { pathsRaw: string; contentType: string | undefined } {
  const semicolon = rest.indexOf(";");
  if (semicolon === -1) return { pathsRaw: rest, contentType: undefined };
  const params = rest.slice(semicolon + 1);
  const eq = params.indexOf("=");
  if (eq === -1) throw at(line, "a form file takes one parameter, written `; type=text/plain`");
  const key = params.slice(0, eq).trim();
  if (key.toLowerCase() !== "type") {
    throw at(
      line,
      `a form file takes only \`; type=…\`, not "${key}" — every other part header belongs on the request`,
    );
  }
  const value = params.slice(eq + 1).trim();
  if (value === "") throw at(line, "`; type=` needs a content type");
  return { pathsRaw: rest.slice(0, semicolon), contentType: value };
}

function nextFileAngle(s: string): number | undefined {
  let from = 0;
  while (from < s.length) {
    const i = s.indexOf("<", from);
    if (i === -1) return undefined;
    if (i > 0 && !/\s/.test(s[i - 1]!)) {
      from = i + 1;
      continue;
    }
    const after = s.slice(i + 1);
    if (after === "" || after.startsWith(" ") || after.startsWith("\t") || after.startsWith(".")) {
      return i;
    }
    from = i + 1;
  }
  return undefined;
}

function stripLeadingFileAngle(s: string): string | undefined {
  if (!s.startsWith("<")) return undefined;
  const rest = s.slice(1);
  if (rest === "" || rest.startsWith(" ") || rest.startsWith("\t") || rest.startsWith(".")) {
    return rest;
  }
  return undefined;
}

/** One or more `< ./path` references on a single form field line. */
function parseFilePaths(line: SourceLine, raw: string): string[] {
  const paths: string[] = [];
  let rest = raw.trim();
  if (rest === "") throw at(line, "the form-data file needs a path");
  while (rest !== "") {
    rest = rest.trimStart();
    if (rest === "") break;
    const afterAngle = stripLeadingFileAngle(rest);
    if (afterAngle !== undefined) rest = afterAngle.trimStart();
    else if (paths.length > 0) {
      throw at(line, "another file on this field starts with `< ./path`");
    }
    const next = nextFileAngle(rest);
    const path = (next === undefined ? rest : rest.slice(0, next)).trim();
    rest = next === undefined ? "" : rest.slice(next);
    if (path === "") throw at(line, "the form-data file needs a path");
    paths.push(workspaceRelative(line, path, "the form-data file"));
  }
  return paths;
}

function sameFileField(a: FormDataRowModel, b: FormDataRowModel): boolean {
  return (
    (a.files?.length ?? 0) > 0 &&
    (b.files?.length ?? 0) > 0 &&
    a.key === b.key &&
    a.contentType === b.contentType
  );
}

/** Repeated `name = < path` lines fold into one field with several files. */
function pushFormRow(rows: FormDataRowModel[], row: FormDataRowModel): void {
  const last = rows[rows.length - 1];
  if (last !== undefined && sameFileField(last, row)) {
    last.files = [...(last.files ?? []), ...(row.files ?? [])];
    return;
  }
  rows.push(row);
}

function parseFormFields(body: string, bodyLine: SourceLine): FormDataRowModel[] {
  const rows: FormDataRowModel[] = [];
  const raw = body.split("\n");
  let cursor = bodyLine.start;
  for (let index = 0; index < raw.length; index += 1) {
    const source = raw[index]!;
    const start = cursor;
    cursor += source.length + 1;
    const text = source.trim();
    if (text === "") continue;
    const line: SourceLine = {
      text,
      start,
      end: start + Math.max(source.length, 1),
      number: bodyLine.number + index,
    };
    const eq = text.indexOf("=");
    const angle = text.indexOf("<");
    const found = [eq, angle].filter((position) => position !== -1);
    if (found.length === 0) {
      throw at(
        line,
        `a form field reads \`name = value\`, or \`name = < ./path\` to send a file, not "${text}"`,
      );
    }
    const position = Math.min(...found);
    const separator = text[position]!;
    const key = text.slice(0, position).trim();
    if (key === "") throw at(line, `this form field has no name before its \`${separator}\``);
    const value = text.slice(position + 1).trim();
    // `name < ./path` is the shape Mándalo wrote for one release. It still reads.
    const reference = separator === "<" ? value : formFileRef(value);
    if (reference === undefined) {
      pushFormRow(rows, { key, value });
      continue;
    }
    const { pathsRaw, contentType } = splitFileRef(line, reference);
    const row: FormDataRowModel = {
      key,
      files: parseFilePaths(line, pathsRaw),
    };
    if (contentType !== undefined) row.contentType = contentType;
    pushFormRow(rows, row);
  }
  return rows;
}

function multipartFileRef(content: string): string | undefined {
  const trimmed = content.trimStart();
  if (!trimmed.startsWith("<")) return undefined;
  const rest = trimmed.slice(1);
  if (rest === "" || rest.startsWith(" ") || rest.startsWith("\t")) return rest;
  return undefined;
}

function parseMultipart(
  body: string,
  boundary: string,
  bodyLine: SourceLine,
): FormDataRowModel[] {
  const delimiter = `--${boundary}`;
  const closing = `--${boundary}--`;
  const raw = body.split("\n");
  const lines = raw.map((line) => line.replace(/\r$/, ""));
  // Offsets are walked over the unstripped lines so a CRLF file still squiggles
  // the line the reader is looking at.
  const starts: number[] = [];
  let cursor = bodyLine.start;
  for (const line of raw) {
    starts.push(cursor);
    cursor += line.length + 1;
  }
  const lineAt = (index: number): SourceLine => {
    const start = starts[index] ?? bodyLine.start;
    return {
      text: lines[index] ?? "",
      start,
      end: start + Math.max(raw[index]?.length ?? 0, 1),
      number: bodyLine.number + index,
    };
  };
  const unclosed = (line: SourceLine): TextFormatError =>
    at(line, `this multipart body is never closed with \`${closing}\``);
  const rows: FormDataRowModel[] = [];
  let index = 0;
  while (index < lines.length && lines[index]!.trim() !== delimiter) {
    if (lines[index]!.trim() !== "") {
      throw at(
        lineAt(index),
        `a multipart body starts at its first \`${delimiter}\` line — nothing may come before it`,
      );
    }
    index += 1;
  }
  if (index === lines.length) {
    throw at(bodyLine, `this multipart body has no \`${delimiter}\` part`);
  }
  while (index < lines.length) {
    const partAt = index;
    index += 1;
    let name: string | undefined;
    let filename: string | undefined;
    let contentType: string | undefined;
    for (;;) {
      const line = lines[index];
      if (line === undefined) throw unclosed(lineAt(partAt));
      const trimmed = line.trim();
      if (trimmed === "") {
        index += 1;
        break;
      }
      if (trimmed === delimiter || trimmed === closing) {
        throw at(
          lineAt(index),
          "a part's headers end with a blank line before its content — add one before this boundary",
        );
      }
      const colon = trimmed.indexOf(":");
      if (colon === -1) throw at(lineAt(index), "a part header reads `Name: value`");
      const key = trimmed.slice(0, colon).trim().toLowerCase();
      const value = trimmed.slice(colon + 1).trim();
      if (key === "content-disposition") {
        if (!value.toLowerCase().startsWith("form-data")) {
          throw at(
            lineAt(index),
            `a form part's disposition is \`form-data\`, not "${value}"`,
          );
        }
        name = headerParam(value, "name");
        filename = headerParam(value, "filename");
      } else if (key === "content-type") {
        contentType = value;
      } else {
        throw at(
          lineAt(index),
          `Mándalo reads Content-Disposition and Content-Type part headers, not "${key}"`,
        );
      }
      index += 1;
    }
    if (name === undefined || name === "") {
      throw at(lineAt(partAt), 'every part needs `Content-Disposition: form-data; name="…"`');
    }
    const contentFirst = index;
    const content: string[] = [];
    for (;;) {
      const line = lines[index];
      if (line === undefined) throw unclosed(lineAt(partAt));
      const trimmed = line.trim();
      if (trimmed === delimiter || trimmed === closing) break;
      content.push(line);
      index += 1;
    }
    while (content.length > 0 && content[content.length - 1]!.trim() === "") content.pop();
    const textContent = content.join("\n");
    const fileRef = multipartFileRef(textContent);
    if (fileRef !== undefined) {
      if (textContent.trim().split("\n").length > 1) {
        throw at(lineAt(contentFirst), "a `< file` part holds only the file line");
      }
      const row: FormDataRowModel = {
        key: name,
        files: [workspaceRelative(lineAt(contentFirst), fileRef, "the form-data file")],
      };
      if (contentType !== undefined) row.contentType = contentType;
      pushFormRow(rows, row);
    } else if (filename !== undefined) {
      throw at(
        lineAt(contentFirst),
        "a file part references its file with `< path` — inline file content is not supported",
      );
    } else if (contentType !== undefined) {
      throw at(
        lineAt(partAt),
        "Mándalo sends a text part as plain text — a per-part content type belongs on a `< file` part",
      );
    } else {
      pushFormRow(rows, { key: name, value: textContent });
    }
    if (lines[index]!.trim() === closing) {
      for (let extra = index + 1; extra < lines.length; extra += 1) {
        if (lines[extra]!.trim() !== "") {
          throw at(lineAt(extra), `nothing may follow the closing \`${closing}\` line`);
        }
      }
      return rows;
    }
  }
  throw unclosed(bodyLine);
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

function parseHttpBlock(
  source: string,
  stem: string,
  index: number,
  segment: Segment,
): ParsedBlock {
  const lines = segment.lines;
  const preamble = readPreamble(lines);
  if (preamble.consumed >= lines.length) {
    const line = lines[0] ?? { text: "", start: 0, end: 1, number: segment.lineIndex + 1 };
    throw at(line, "this block has no request line — a request needs `METHOD url`");
  }
  const requestLine = readRequestLine(lines, preamble.consumed);
  const { headers, next } = readHeaders(lines, requestLine.next);
  const tail = readTail(source, lines, next);

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

  const isEventStream = kept.some(
    ([name, value]) => name.toLowerCase() === "accept" && acceptsEventStream(value),
  );
  const kind = isGraphql ? "graphql" : isEventStream ? "sse" : "http";
  let formdata: FormDataRowModel[] | undefined;
  if (kind === "http" && bodyFile === undefined && tail.body !== undefined && tail.bodyLine !== undefined) {
    const contentTypeAt = kept.findIndex(([k]) => k.toLowerCase() === "content-type");
    if (contentTypeAt !== -1 && isMultipartFormdata(kept[contentTypeAt]![1])) {
      const declared = headerParam(kept[contentTypeAt]![1], "boundary");
      if (isLiteralMultipart(tail.body)) {
        if (declared === undefined || declared === "") {
          throw at(
            tail.bodyLine,
            "multipart/form-data needs a `boundary=` parameter in its Content-Type",
          );
        }
        formdata = parseMultipart(tail.body, declared, tail.bodyLine);
      } else {
        if (declared !== undefined && declared !== "") {
          throw at(
            tail.bodyLine,
            "this form body is written as `name = value` lines, which carry no boundary — remove the `boundary=` parameter, because the one on the wire is chosen when the request is sent",
          );
        }
        formdata = parseFormFields(tail.body, tail.bodyLine);
      }
      kept.splice(contentTypeAt, 1);
    }
  }
  const model: RequestModel = {
    schemaVersion: 1,
    id: `${slug(stem)}-${index}`,
    name: blockName(segment, preamble.named, `${requestLine.method} ${requestLine.url}`),
    kind,
    method: requestLine.method,
    url: requestLine.url,
    headers: kept,
    auth: preamble.inheritedAuth && auth.type !== "none" ? { type: "inherited", auth } : auth,
    scripts: {},
    tests: [],
    captures: [],
  };
  if (preamble.pre !== undefined) model.scripts.pre = preamble.pre;
  if (tail.post !== undefined) model.scripts.post = tail.post;
  if (preamble.autoReconnect !== undefined)
    model.stream = { autoReconnect: preamble.autoReconnect };
  if (bodyFile !== undefined) model.bodyFile = bodyFile;
  else if (kind === "graphql") model.graphql = splitGraphql(tail.body ?? "");
  else if (formdata !== undefined) model.formdata = formdata;
  else if (tail.body !== undefined) model.body = tail.body;

  return { index, name: model.name, lineIndex: segment.lineIndex, span: segment.span, model };
}

function parseGrpcBlock(
  source: string,
  stem: string,
  index: number,
  segment: Segment,
): ParsedBlock {
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
  const tail = readTail(source, lines, next);

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
    name: blockName(segment, preamble.named, `${service}/${method}`),
    kind: "grpc",
    method: "POST",
    url: target,
    headers: [],
    auth: { type: "none" },
    grpc: { protoPaths, service, method, message: tail.body ?? "{}", metadata },
    scripts: {},
    tests: [],
    captures: [],
  };
  if (preamble.pre !== undefined) model.scripts.pre = preamble.pre;
  if (tail.post !== undefined) model.scripts.post = tail.post;
  return { index, name: model.name, lineIndex: segment.lineIndex, span: segment.span, model };
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
  const segments = segmentsOf(sourceLines(source), source.length);
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
        ? parseGrpcBlock(source, stem, blocks.length, segment)
        : parseHttpBlock(source, stem, blocks.length, segment),
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
    if (model.formdata !== undefined) {
      model.formdata = model.formdata.map((row) => {
        const out: typeof row = { key: substitute(row.key, vars) };
        if (row.value !== undefined) out.value = substitute(row.value, vars);
        if (row.files !== undefined) out.files = row.files.map((f) => substitute(f, vars));
        if (row.contentType !== undefined) out.contentType = row.contentType;
        return out;
      });
    }
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
