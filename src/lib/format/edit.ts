import { parseTextDocument, type ParsedBlock, type TextFileKind } from "./httpFormat";
import type { RequestModel } from "./model";
import { newlineOf, renderRequest } from "./render";
import { AddressError } from "./textFormat";

export interface ParsedFile {
  vars: [string, string][];
  blocks: ParsedBlock[];
  source: string;
  newline: string;
}

export function parseFile(
  stem: string,
  source: string,
  fileKind: TextFileKind,
): ParsedFile {
  return {
    ...parseTextDocument(stem, source, fileKind),
    source,
    newline: newlineOf(source),
  };
}

export function blockNames(file: ParsedFile): string[] {
  return file.blocks.map((block) => block.name);
}

function blockAt(file: ParsedFile, index: number): ParsedBlock {
  const block = file.blocks[index];
  if (block === undefined)
    throw new AddressError(`there is no request ${index} in this file`);
  return block;
}

/**
 * Rewrites one block, leaving every other byte of the file alone. The Rust twin edits
 * span by span and so keeps the comments and alignment *inside* the block too; this
 * one re-renders the block it was handed.
 */
export function replaceBlock(
  file: ParsedFile,
  index: number,
  request: RequestModel,
): string {
  const block = blockAt(file, index);
  const rendered = renderRequest(request, file.newline);
  const trailing = file.source.slice(block.span.end);
  return `${file.source.slice(0, block.span.start)}${rendered.replace(/[\r\n]+$/, "")}${trailing}`;
}

export function removeBlock(file: ParsedFile, index: number): string {
  const block = blockAt(file, index);
  let end = block.span.end;
  while (end < file.source.length && /[\r\n]/.test(file.source[end]!)) end += 1;
  return file.source.slice(0, block.span.start) + file.source.slice(end);
}

export function appendBlock(file: ParsedFile, request: RequestModel): string {
  const nl = file.newline;
  const rendered = renderRequest(request, nl);
  const trimmed = file.source.replace(/[\r\n]+$/, "");
  return trimmed === "" ? rendered : `${trimmed}${nl}${nl}${rendered}`;
}
