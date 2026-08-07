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

/**
 * The `#n` of a request path; a path without a fragment addresses the first request.
 * A `#name` fragment resolves to no index — only the CLI matches block names.
 */
export function requestIndexOf(relPath: string): number | undefined {
  const file = requestFilePath(relPath);
  if (file === relPath) return 0;
  const fragment = relPath.slice(file.length + 1);
  return /^\d+$/.test(fragment) ? Number(fragment) : undefined;
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
  for (const segment of segmentsOf(sourceLines(source))) {
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
