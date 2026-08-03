import type { EnvironmentView } from "./api";
import type { RequestDraft } from "./draft";

export interface VarSegment {
  text: string;
  name: string | null;
  resolved: string | null;
}

const TOKEN = /\{\{\s*([^}]*?)\s*\}\}/g;

export function splitVars(
  text: string,
  vars: Record<string, string>,
): VarSegment[] {
  const segments: VarSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(TOKEN)) {
    const start = match.index ?? 0;
    if (start > last)
      segments.push({ text: text.slice(last, start), name: null, resolved: null });
    const name = match[1];
    segments.push({
      text: match[0],
      name,
      resolved: name in vars ? vars[name] : null,
    });
    last = start + match[0].length;
  }
  if (last < text.length)
    segments.push({ text: text.slice(last), name: null, resolved: null });
  return segments;
}

export type VarState = "value" | "secret" | "dynamic" | "missing";

export interface VarDescription {
  name: string;
  state: VarState;
  value: string | null;
  env: string | null;
  secretSet: boolean;
}

export function describeVar(
  name: string,
  env: EnvironmentView | null,
  vars: Record<string, string> = {},
): VarDescription {
  const where = env?.name ?? null;
  if (name.startsWith("$"))
    return { name, state: "dynamic", value: null, env: where, secretSet: false };
  const info = env?.vars[name];
  if (info?.secret)
    return { name, state: "secret", value: null, env: where, secretSet: info.set };
  if (info && info.value !== null)
    return { name, state: "value", value: info.value, env: where, secretSet: false };
  if (name in vars)
    return { name, state: "value", value: vars[name], env: where, secretSet: false };
  return { name, state: "missing", value: null, env: where, secretSet: false };
}

export function varTone(state: VarState): string {
  if (state === "missing") return "var-bad";
  if (state === "secret") return "var-secret";
  if (state === "dynamic") return "var-dyn";
  return "var-ok";
}

/** Display-only: the real, fail-loud substitution happens in the core. */
export function previewResolve(
  text: string,
  vars: Record<string, string>,
): string {
  return splitVars(text, vars)
    .map((segment) => (segment.resolved === null ? segment.text : segment.resolved))
    .join("");
}

export function draftVarNames(draft: RequestDraft): string[] {
  const names: string[] = [];
  const seen = new Set<string>();
  const scan = (text: string): void => {
    for (const segment of splitVars(text, {})) {
      if (segment.name === null || seen.has(segment.name)) continue;
      seen.add(segment.name);
      names.push(segment.name);
    }
  };
  const scanRows = (rows: RequestDraft["headers"]): void => {
    for (const row of rows) {
      if (!row.enabled) continue;
      scan(row.key);
      scan(row.value);
    }
  };
  scan(draft.url);
  if (draft.kind === "grpc") {
    scanRows(draft.grpc.metadata);
    scan(draft.grpc.protoPaths);
    scan(draft.grpc.message);
  } else {
    scanRows(draft.headers);
    if (draft.kind === "http") {
      scanRows(draft.params);
      scan(draft.body);
    } else {
      scan(draft.graphqlQuery);
      scan(draft.graphqlVariables);
    }
  }
  const auth = draft.auth;
  if (auth.type === "bearer") scan(auth.token);
  if (auth.type === "basic") {
    scan(auth.username);
    scan(auth.password);
  }
  if (auth.type === "apikey") {
    scan(auth.key);
    scan(auth.value);
  }
  return names;
}
