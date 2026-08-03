const VAR_PATTERN = /\{\{\s*([^{}\s][^{}]*?)\s*\}\}/g;

export interface VarReference {
  name: string;
  offset: number;
  length: number;
}

export function collectVarReferences(raw: string): VarReference[] {
  const found: VarReference[] = [];
  for (const match of raw.matchAll(VAR_PATTERN)) {
    found.push({ name: match[1]!.trim(), offset: match.index, length: match[0].length });
  }
  return found;
}
