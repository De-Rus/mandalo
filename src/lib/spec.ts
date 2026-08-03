import type { Auth, GrpcSpec, RequestSpec } from "./api";
import type { AuthDraft, KVRow, RequestDraft } from "./draft";

export function activeRows(rows: KVRow[]): [string, string][] {
  return rows
    .filter((r) => r.enabled && r.key.trim() !== "")
    .map((r) => [r.key.trim(), r.value] as [string, string]);
}

export function buildAuth(draft: AuthDraft): Auth {
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

export function mergeParams(url: string, params: [string, string][]): string {
  if (params.length === 0) return url;
  const query = params
    .map(([k, v]) => `${encodeKeepingVars(k)}=${encodeKeepingVars(v)}`)
    .join("&");
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}${query}`;
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
