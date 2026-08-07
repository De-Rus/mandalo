import type { RequestKind } from "./model";
import {
  HTTP_EXTENSIONS,
  isComment,
  isDeclarative,
  parseTextDocument,
  segmentsOf,
  sourceLines,
  varDefinition,
  type Segment,
  type TextFileKind,
} from "./httpFormat";

export type { TextFileKind };

export interface TextRequestBlock {
  index: number;
  name: string;
  method: string;
  url: string;
  kind: RequestKind;
  lineNumber: number;
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

/**
 * Request formats the Rust core reads that this reader does not. A file in one of
 * them holds real requests, so it has to be reported as skipped rather than
 * silently passed over — an invisible request looks like a lost request.
 */
const ENGINE_ONLY_EXTENSIONS = ["ws", "mqtt"];

export function engineOnlyRequestKind(fsPath: string): string | undefined {
  const extension = extensionOf(fsPath);
  return ENGINE_ONLY_EXTENSIONS.includes(extension) ? extension : undefined;
}

export function engineOnlyReason(extension: string): string {
  return `.${extension} requests run through the Mándalo desktop app or the CLI — this reader cannot open them yet`;
}

/** `auth.http#0` addresses one request inside a file; a bare path addresses the file. */
export function requestFilePath(relPath: string): string {
  const hash = relPath.lastIndexOf("#");
  if (hash === -1) return relPath;
  const head = relPath.slice(0, hash);
  return isTextRequestPath(head) ? head : relPath;
}

export function requestPathAt(relPath: string, index: number): string {
  return `${relPath}#${index}`;
}

/** The `#…` of a request path, or undefined when the path addresses the whole file. */
export function requestFragmentOf(relPath: string): string | undefined {
  const file = requestFilePath(relPath);
  return file === relPath ? undefined : relPath.slice(file.length + 1);
}

export class AddressError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AddressError";
  }
}

/**
 * Which block a `#n` or `#Name` fragment addresses. The Rust twin is
 * `text_format::indexes_of`: an index past the end, an unknown name and an ambiguous
 * name are all errors, never a silent block 0.
 */
export function indexOfFragment(
  names: readonly string[],
  fragment: string,
  total: number,
): number {
  if (/^\d+$/.test(fragment)) {
    const index = Number(fragment);
    if (index < total) return index;
    throw new AddressError(`this file holds ${total} requests, so there is no request ${index}`);
  }
  const matches = names.flatMap((name, index) => (name === fragment ? [index] : []));
  if (matches.length === 1) return matches[0]!;
  if (matches.length === 0)
    throw new AddressError(`no request named ${JSON.stringify(fragment)} in this file`);
  throw new AddressError(
    `${matches.length} requests are named ${JSON.stringify(fragment)} — address this one by index instead`,
  );
}

/**
 * The block a request path addresses inside a parsed file. A bare path is only an
 * address when the file holds exactly one request; Rust refuses to guess, so neither
 * does this.
 */
export function indexIn(
  names: readonly string[],
  relPath: string,
  fragment: string | undefined,
): number {
  if (fragment !== undefined) return indexOfFragment(names, fragment, names.length);
  if (names.length === 1) return 0;
  throw new AddressError(
    `${relPath} holds ${names.length} requests — address one of them as ${relPath}#0`,
  );
}

/** A segment reparsed alone, so one bad block cannot hide the ones after it. */
function segmentSource(segment: Segment): string {
  return segment.lines.map((line) => line.text).join("\n");
}

/** The first line a half-written block could plausibly hang a request on. */
function firstRequestLine(segment: Segment): string {
  for (const line of segment.lines) {
    const trimmed = line.text.trim();
    if (trimmed === "" || isComment(line.text) || varDefinition(line.text) !== undefined) continue;
    if (trimmed.startsWith("<") || trimmed.startsWith("%}")) continue;
    return trimmed;
  }
  return "";
}

function bestEffort(segment: Segment, index: number, fileKind: TextFileKind): TextRequestBlock {
  const line = firstRequestLine(segment);
  const space = line.search(/\s/);
  const grpc = fileKind === "grpc";
  return {
    index,
    name: segment.name || line || `request ${index}`,
    method: grpc || space === -1 ? "GRPC" : line.slice(0, space).toUpperCase(),
    url: grpc || space === -1 ? line : line.slice(space).trim(),
    kind: grpc ? "grpc" : "http",
    lineNumber: segment.lineIndex,
  };
}

/**
 * One entry per addressable `###` block, in file order — the index is the `#n` the
 * CLI takes. A block the strict parser rejects still gets an entry: CodeLens and the
 * tree have to survive a file that is mid-edit, and diagnostics report the error.
 */
export function scanTextRequests(source: string, fileKind: TextFileKind): TextRequestBlock[] {
  const out: TextRequestBlock[] = [];
  for (const segment of segmentsOf(sourceLines(source), source.length)) {
    let block;
    try {
      block = parseTextDocument("scan", segmentSource(segment), fileKind).blocks[0];
    } catch {
      if (isDeclarative(segment) && (segment.name ?? "") === "") continue;
      out.push(bestEffort(segment, out.length, fileKind));
      continue;
    }
    if (block === undefined) continue;
    out.push({
      index: out.length,
      name: segment.name || block.name,
      method: block.model.method,
      url: block.model.url,
      kind: block.model.kind as RequestKind,
      lineNumber: segment.lineIndex,
    });
  }
  return out;
}
