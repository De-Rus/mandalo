import type { EnvironmentView, VarSource } from "./api";
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

/**
 * `value` is a variable whose value is committed to the workspace file.
 * `local` is declared `shared = false`: kept on this machine, not confidential,
 * and not masked — but its value is never sent to the frontend either, so the
 * UI can say where it lives and whether it is set, not what it is.
 * `secret` is masked everywhere and redacted out of every diagnostic.
 */
export type VarState = "value" | "local" | "secret" | "dynamic" | "missing";

export interface VarDescription {
  name: string;
  state: VarState;
  value: string | null;
  env: string | null;
  /** Whether a value is reachable at all — for `local` and `secret`. */
  held: boolean;
  source: VarSource | null;
  hosts: string[];
}

function base(
  name: string,
  state: VarState,
  env: string | null,
): VarDescription {
  return { name, state, value: null, env, held: false, source: null, hosts: [] };
}

export function describeVar(
  name: string,
  env: EnvironmentView | null,
  vars: Record<string, string> = {},
): VarDescription {
  const where = env?.name ?? null;
  if (name.startsWith("$")) return base(name, "dynamic", where);
  const info = env?.vars[name];
  if (info) {
    if (info.secret)
      return {
        ...base(name, "secret", where),
        held: info.set,
        source: info.source ?? null,
        hosts: info.hosts ?? [],
      };
    if (!info.shared)
      return {
        ...base(name, "local", where),
        held: info.set,
        source: info.source ?? null,
        hosts: info.hosts ?? [],
      };
    if (info.value !== null)
      return {
        ...base(name, "value", where),
        value: info.value,
        held: true,
        source: info.source ?? "file",
      };
  }
  if (name in vars)
    return { ...base(name, "value", where), value: vars[name], held: true };
  return base(name, "missing", where);
}

/** True when sending would fail: nothing anywhere holds a value for this. */
export function isUnresolved(description: VarDescription): boolean {
  if (description.state === "missing") return true;
  return (
    (description.state === "secret" || description.state === "local") &&
    !description.held
  );
}

export function unresolvedVars(
  names: string[],
  env: EnvironmentView | null,
  vars: Record<string, string> = {},
): VarDescription[] {
  return names
    .map((name) => describeVar(name, env, vars))
    .filter(isUnresolved);
}

export function varTone(state: VarState): string {
  if (state === "missing") return "var-bad";
  if (state === "secret") return "var-secret";
  if (state === "local") return "var-local";
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
