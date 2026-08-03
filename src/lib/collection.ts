import {
  deleteRequest,
  errorMessage,
  listRequests,
  saveRequest,
  type Auth,
  type Kind,
  type SavedRequest,
} from "./api";
import {
  emptyRow,
  newDraft,
  uid,
  type AuthDraft,
  type KVRow,
  type RequestDraft,
} from "./draft";
import { activeRows, buildAuth, mergeParams, parseProtoPaths } from "./spec";

const LEGACY_KEY = "mandalo.collection.v1";
export const ACTIVE_KEY = "mandalo.collection.active";

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

export function fromSaved(saved: SavedRequest): RequestDraft {
  if (!isKind(saved.kind))
    throw new Error(
      `Request "${saved.name}" has an unknown kind "${String(saved.kind)}"`,
    );
  const base = newDraft(saved.name);
  const { url, params } =
    saved.kind === "http"
      ? splitParams(saved.url)
      : { url: saved.url, params: [] as [string, string][] };
  return {
    ...base,
    id: saved.id,
    kind: saved.kind,
    method: saved.method,
    url,
    params: toRows(params),
    headers: toRows(saved.headers ?? []),
    body: saved.body ?? "",
    auth: toAuthDraft(saved.auth),
    graphqlQuery: saved.graphql?.query ?? "",
    graphqlVariables: saved.graphql?.variables ?? "",
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

export interface LoadedCollection {
  requests: RequestDraft[];
  warnings: string[];
}

export async function loadRequests(workspace: string): Promise<LoadedCollection> {
  const requests: RequestDraft[] = [];
  const { items, skipped } = await listRequests(workspace);
  const warnings: string[] = [...skipped];
  for (const saved of items) {
    try {
      requests.push(fromSaved(saved));
    } catch (e) {
      warnings.push(errorMessage(e));
    }
  }
  return { requests, warnings };
}

export function persistRequest(
  workspace: string,
  draft: RequestDraft,
): Promise<string> {
  return saveRequest(workspace, toSaved(draft));
}

export function removeRequest(workspace: string, id: string): Promise<void> {
  return deleteRequest(workspace, id);
}

export interface LegacyMigration {
  activeId: string | null;
  skipped: string[];
}

function isLegacyDraft(value: unknown): value is RequestDraft {
  if (typeof value !== "object" || value === null) return false;
  const d = value as Record<string, unknown>;
  return (
    typeof d.id === "string" &&
    typeof d.name === "string" &&
    isKind(d.kind) &&
    Array.isArray(d.params) &&
    Array.isArray(d.headers) &&
    typeof d.auth === "object" &&
    d.auth !== null &&
    typeof d.grpc === "object" &&
    d.grpc !== null
  );
}

function legacyEntryName(value: unknown): string {
  if (typeof value === "object" && value !== null) {
    const name = (value as Record<string, unknown>).name;
    if (typeof name === "string" && name.trim() !== "") return name;
  }
  return "unnamed request";
}

export async function migrateLegacyCollection(
  workspace: string,
): Promise<LegacyMigration | null> {
  const raw = localStorage.getItem(LEGACY_KEY);
  if (!raw) return null;
  let parsed: { requests?: unknown[]; activeId?: string | null };
  try {
    parsed = JSON.parse(raw) as typeof parsed;
  } catch {
    localStorage.removeItem(LEGACY_KEY);
    return null;
  }
  const entries = Array.isArray(parsed.requests) ? parsed.requests : [];
  const skipped: string[] = [];
  try {
    for (const entry of entries) {
      if (isLegacyDraft(entry)) await persistRequest(workspace, entry);
      else skipped.push(legacyEntryName(entry));
    }
  } finally {
    localStorage.removeItem(LEGACY_KEY);
  }
  return {
    activeId: typeof parsed.activeId === "string" ? parsed.activeId : null,
    skipped,
  };
}
