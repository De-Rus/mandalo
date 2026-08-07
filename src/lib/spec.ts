import type { Auth, GrpcSpec, RequestSpec } from "./api";
import { emptyRow, uid, type AuthDraft, type KVRow, type RequestDraft } from "./draft";

export function activeRows(rows: KVRow[]): [string, string][] {
  return rows
    .filter((r) => r.enabled && r.key.trim() !== "")
    .map((r) => [r.key.trim(), r.value] as [string, string]);
}

export function buildAuth(draft: AuthDraft): Auth {
  const auth = buildPlainAuth(draft);
  return draft.inherited && auth.type !== "none" ? { type: "inherited", auth } : auth;
}

function buildPlainAuth(draft: AuthDraft): Auth {
  switch (draft.type) {
    case "bearer":
      return { type: "bearer", token: draft.token };
    case "basic":
      return { type: "basic", username: draft.username, password: draft.password };
    case "apikey":
      return {
        type: "apikey",
        key: draft.key,
        value: draft.value,
        placement: draft.placement,
      };
    default:
      return { type: "none" };
  }
}

const VAR_TOKEN = /(\{\{[^}]*\}\})/;

export function encodeKeepingVars(text: string): string {
  return text
    .split(VAR_TOKEN)
    .map((part) => (VAR_TOKEN.test(part) ? part : encodeURIComponent(part)))
    .join("");
}

export function decodeKeepingVars(text: string): string {
  return text
    .split(VAR_TOKEN)
    .map((part) => {
      if (VAR_TOKEN.test(part)) return part;
      try {
        return decodeURIComponent(part);
      } catch {
        return part;
      }
    })
    .join("");
}

export function mergeParams(url: string, params: [string, string][]): string {
  if (params.length === 0) return url;
  const query = params
    .map(([k, v]) => `${encodeKeepingVars(k)}=${encodeKeepingVars(v)}`)
    .join("&");
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}${query}`;
}

export function splitQuery(url: string): {
  url: string;
  params: [string, string][];
} {
  const qIndex = url.indexOf("?");
  if (qIndex === -1) return { url, params: [] };
  const params = url
    .slice(qIndex + 1)
    .split("&")
    .filter((pair) => pair !== "")
    .map((pair): [string, string] => {
      const eq = pair.indexOf("=");
      if (eq === -1) return [decodeKeepingVars(pair), ""];
      return [
        decodeKeepingVars(pair.slice(0, eq)),
        decodeKeepingVars(pair.slice(eq + 1)),
      ];
    });
  return { url: url.slice(0, qIndex), params };
}

/** What the URL bar shows: the path plus every enabled param row, re-encoded. */
export function urlWithParams(url: string, params: KVRow[]): string {
  return mergeParams(url, activeRows(params));
}

function isBlank(row: KVRow): boolean {
  return row.key.trim() === "" && row.value === "";
}

/**
 * WHY: the URL text and the params table are one value, so an edit to either
 * must land in both. Rows the query string cannot carry — disabled ones, and
 * value-only ones — keep their slot instead of being erased by a URL edit.
 */
export function applyUrl(
  input: string,
  params: KVRow[],
): { url: string; params: KVRow[] } {
  const split = splitQuery(input);
  const rows: KVRow[] = [];
  let next = 0;
  for (const row of params) {
    if (isBlank(row)) continue;
    if (!row.enabled || row.key.trim() === "") {
      rows.push(row);
      continue;
    }
    const pair = split.params[next];
    if (pair === undefined) continue;
    next += 1;
    rows.push(
      pair[0] === row.key && pair[1] === row.value
        ? row
        : { ...row, key: pair[0], value: pair[1] },
    );
  }
  for (; next < split.params.length; next += 1)
    rows.push({
      id: uid(),
      key: split.params[next][0],
      value: split.params[next][1],
      enabled: true,
    });
  rows.push(emptyRow());
  return { url: split.url, params: rows };
}

export function buildRequestSpec(
  draft: RequestDraft,
  vars: Record<string, string>,
): RequestSpec {
  const headers = activeRows(draft.headers);
  const auth = buildAuth(draft.auth);
  if (draft.kind === "graphql") {
    return {
      kind: "graphql",
      method: "POST",
      url: draft.url,
      headers,
      body: null,
      auth,
      graphql: { query: draft.graphqlQuery, variables: draft.graphqlVariables },
      vars,
    };
  }
  return {
    kind: "http",
    method: draft.method,
    url: mergeParams(draft.url, activeRows(draft.params)),
    headers,
    body: draft.body.trim() === "" ? null : draft.body,
    auth,
    graphql: null,
    vars,
  };
}

export function parseProtoPaths(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");
}

export function buildGrpcRequest(
  draft: RequestDraft,
  vars: Record<string, string>,
): GrpcSpec {
  return {
    url: draft.url,
    protoPaths: parseProtoPaths(draft.grpc.protoPaths),
    service: draft.grpc.service,
    method: draft.grpc.method,
    message: draft.grpc.message,
    metadata: activeRows(draft.grpc.metadata),
    vars,
  };
}
