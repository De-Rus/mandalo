import type { Auth, Capture, Kind, SavedRequest } from "./api";
import {
  emptyRow,
  newDraft,
  uid,
  type AuthDraft,
  type KVRow,
  type RequestDraft,
} from "./draft";
import { activeRows, buildAuth, mergeParams, parseProtoPaths } from "./spec";

export const ACTIVE_KEY = "mandalo.collection.active";

function blankToNull(value: string): string | null {
  return value.trim() === "" ? null : value;
}

export function validCaptures(captures: Capture[]): Capture[] {
  return captures.filter(
    (c) => c.into.trim() !== "" && c.from.trim() !== "",
  );
}

export function toSaved(draft: RequestDraft): SavedRequest {
  return {
    id: draft.id,
    name: draft.name,
    kind: draft.kind,
    method: draft.method,
    url:
      draft.kind === "http"
        ? mergeParams(draft.url, activeRows(draft.params))
        : draft.url,
    description: blankToNull(draft.description),
    headers: activeRows(draft.headers),
    body: draft.body.trim() === "" ? null : draft.body,
    auth: buildAuth(draft.auth),
    graphql:
      draft.kind === "graphql"
        ? { query: draft.graphqlQuery, variables: draft.graphqlVariables }
        : null,
    grpc:
      draft.kind === "grpc"
        ? {
            protoPaths: parseProtoPaths(draft.grpc.protoPaths),
            service: draft.grpc.service,
            method: draft.grpc.method,
            message: draft.grpc.message,
            metadata: activeRows(draft.grpc.metadata),
          }
        : null,
    scripts: {
      pre: blankToNull(draft.preScript),
      post: blankToNull(draft.testScript),
    },
    tests: draft.tests,
    captures: validCaptures(draft.captures),
  };
}

function toRows(tuples: [string, string][]): KVRow[] {
  const rows: KVRow[] = tuples.map(([key, value]) => ({
    id: uid(),
    key,
    value,
    enabled: true,
  }));
  rows.push(emptyRow());
  return rows;
}

function toAuthDraft(auth: Auth | undefined): AuthDraft {
  const base = newDraft().auth;
  if (!auth) return base;
  switch (auth.type) {
    case "bearer":
      return { ...base, type: "bearer", token: auth.token };
    case "basic":
      return {
        ...base,
        type: "basic",
        username: auth.username,
        password: auth.password,
      };
    case "apikey":
      return {
        ...base,
        type: "apikey",
        key: auth.key,
        value: auth.value,
        placement: auth.placement,
      };
    default:
      return base;
  }
}

const KINDS: Kind[] = ["http", "graphql", "grpc"];

function isKind(value: unknown): value is Kind {
  return KINDS.includes(value as Kind);
}

const VAR_TOKEN = /(\{\{[^}]*\}\})/;

function decodeKeepingVars(text: string): string {
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

function splitParams(url: string): { url: string; params: [string, string][] } {
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

export function fromSaved(
  saved: SavedRequest,
  collection = "",
  path: string | null = null,
): RequestDraft {
  if (!isKind(saved.kind))
    throw new Error(
      `Request "${saved.name}" has an unknown kind "${String(saved.kind)}"`,
    );
  const base = newDraft(saved.name);
  const split =
    saved.kind === "http"
      ? splitParams(saved.url)
      : { url: saved.url, params: [] as [string, string][] };
  return {
    ...base,
    id: saved.id,
    kind: saved.kind,
    method: saved.method,
    url: split.url,
    description: saved.description ?? "",
    collection,
    path,
    params: toRows(split.params),
    headers: toRows(saved.headers ?? []),
    body: saved.body ?? "",
    auth: toAuthDraft(saved.auth),
    graphqlQuery: saved.graphql?.query ?? "",
    graphqlVariables: saved.graphql?.variables ?? "",
    preScript: saved.scripts?.pre ?? "",
    testScript: saved.scripts?.post ?? "",
    tests: saved.tests ?? [],
    captures: saved.captures ?? [],
    grpc: saved.grpc
      ? {
          protoPaths: saved.grpc.protoPaths.join("\n"),
          service: saved.grpc.service,
          method: saved.grpc.method,
          message: saved.grpc.message,
          metadata: toRows(saved.grpc.metadata ?? []),
        }
      : base.grpc,
  };
}
