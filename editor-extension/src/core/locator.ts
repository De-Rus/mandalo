export interface Span {
  offset: number;
  length: number;
}

export const WHOLE_FIRST_LINE: Span = { offset: 0, length: 1 };

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function findTableHeader(raw: string, header: string, occurrence = 0): number {
  const pattern = new RegExp(`^[ \\t]*${escapeRegex(header)}[ \\t]*$`, "gm");
  let seen = 0;
  for (const match of raw.matchAll(pattern)) {
    if (seen === occurrence) return match.index;
    seen += 1;
  }
  return -1;
}

export function findKeySpan(raw: string, key: string, from = 0): Span | undefined {
  const pattern = new RegExp(`^[ \\t]*${escapeRegex(key)}[ \\t]*=[ \\t]*(.*)$`, "gm");
  pattern.lastIndex = from;
  const match = pattern.exec(raw);
  if (!match) return undefined;
  const valueStart = match.index + match[0].length - match[1]!.length;
  const length = match[1]!.trimEnd().length;
  return { offset: valueStart, length: Math.max(length, 1) };
}

export function spanForEntryKey(
  raw: string,
  header: string,
  occurrence: number,
  key: string,
): Span {
  const headerOffset = findTableHeader(raw, header, occurrence);
  if (headerOffset < 0) return WHOLE_FIRST_LINE;
  const span = findKeySpan(raw, key, headerOffset);
  if (span) return span;
  return { offset: headerOffset, length: header.length };
}

export function spanForKey(raw: string, key: string): Span {
  return findKeySpan(raw, key) ?? WHOLE_FIRST_LINE;
}
