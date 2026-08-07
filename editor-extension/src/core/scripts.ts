import { lintScriptSource } from "../../../src/lib/script/lint";
import type { Finding } from "./rules";

export interface ScriptBlock {
  kind: "pre" | "post";
  offset: number;
  source: string;
}

export function collectScriptBlocks(raw: string): ScriptBlock[] {
  const blocks: ScriptBlock[] = [];
  let lineStart = 0;
  while (lineStart <= raw.length) {
    const lineEnd = raw.indexOf("\n", lineStart);
    const line = raw.slice(lineStart, lineEnd === -1 ? raw.length : lineEnd);
    const trimmed = line.trimStart();
    const kind = trimmed.startsWith(">") ? "post" : trimmed.startsWith("<") ? "pre" : undefined;
    if (kind !== undefined && trimmed.slice(1).trimStart().startsWith("{%")) {
      const open = raw.indexOf("{%", lineStart) + 2;
      const close = raw.indexOf("%}", open);
      if (close === -1) return blocks;
      blocks.push({ kind, offset: open, source: raw.slice(open, close) });
      lineStart = raw.indexOf("\n", close) === -1 ? raw.length + 1 : raw.indexOf("\n", close) + 1;
      continue;
    }
    if (lineEnd === -1) break;
    lineStart = lineEnd + 1;
  }
  return blocks;
}

export function lintScripts(raw: string): Finding[] {
  const findings: Finding[] = [];
  for (const block of collectScriptBlocks(raw)) {
    for (const finding of lintScriptSource(block.source, block.kind)) {
      findings.push({
        offset: block.offset + finding.from,
        length: finding.to - finding.from,
        message: finding.message,
        severity: finding.severity,
        code: finding.code,
      });
    }
  }
  return findings;
}
