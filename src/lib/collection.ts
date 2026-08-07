import type {
  Auth,
  Capture,
  FormDataRowPayload,
  FormRowPayload,
  Kind,
  SavedRequest,
  SavedStream,
} from "./api";
import {
  emptyFormDataRow,
  emptyRow,
  emptySubscription,
  newDraft,
  newStreamDraft,
  uid,
  type AuthDraft,
  type FormDataRowDraft,
  type KVRow,
  type RequestDraft,
  type StreamDraft,
} from "./draft";
import { isStreamKind, parseSubprotocols, STREAM_KINDS } from "./stream";
import {
  activeRows,
  buildAuth,
  parseProtoPaths,
  splitQuery,
  urlWithParams,
} from "./spec";

export const ACTIVE_KEY = "mandalo.collection.active";

function blankToNull(value: string): string | null {
  return value.trim() === "" ? null : value;
}

export function validCaptures(captures: Capture[]): Capture[] {
  return captures.filter(
    (c) => c.into.trim() !== "" && c.from.trim() !== "",
  );
}

function buildBody(draft: RequestDraft): SavedRequest["body"] {
  if (draft.kind !== "http") return null;
  switch (draft.bodyType) {
    case "raw":
      return draft.body.trim() === "" ? null : draft.body;
    case "urlencoded": {
      const rows: FormRowPayload[] = draft.formRows
        .filter((row) => row.key.trim() !== "")
        .map((row) => ({ key: row.key, value: row.value, enabled: row.enabled }));
      return rows.length === 0 ? null : { mode: "urlencoded", rows };
    }
    case "formdata": {
      const rows: FormDataRowPayload[] = draft.formDataRows
        .filter((row) => row.key.trim() !== "")
        .map((row) => ({
          key: row.key,
          enabled: row.enabled,
          ...(row.kind === "file"
            ? {
                files: row.files,
                ...(row.contentType.trim() === "" ? {} : { contentType: row.contentType }),
              }
            : { value: row.value }),
        }));
      return rows.length === 0 ? null : { mode: "formdata", rows };
    }
    case "binary":
      return draft.binaryFile.trim() === ""
        ? null
        : {
            mode: "binary",
            file: draft.binaryFile,
            ...(draft.binaryContentType.trim() === ""
              ? {}
              : { contentType: draft.binaryContentType }),
          };
  }
}

function numberOr(text: string): number | undefined {
  const n = Number(text.trim());
  return text.trim() === "" || Number.isNaN(n) || n < 0 ? undefined : n;
}

function buildStream(draft: RequestDraft): SavedStream | null {
  if (!isStreamKind(draft.kind)) return null;
  const s = draft.stream;
  const subprotocols = parseSubprotocols(s.subprotocols);
  const subscriptions = s.subscriptions
    .filter((row) => row.topic.trim() !== "")
    .map((row) => ({ topic: row.topic.trim(), qos: row.qos }));
  const saved: SavedStream = { autoReconnect: s.autoReconnect };
  if (draft.kind === "websocket") {
    if (subprotocols.length > 0) saved.subprotocols = subprotocols;
    const ping = numberOr(s.pingIntervalMs);
    if (ping !== undefined) saved.pingIntervalMs = ping;
  }
  if (draft.kind === "sse" && s.lastEventId.trim() !== "")
    saved.lastEventId = s.lastEventId.trim();
  if (draft.kind === "mqtt") {
    saved.cleanSession = s.cleanSession;
    saved.protocolVersion = s.protocolVersion;
    if (s.clientId.trim() !== "") saved.clientId = s.clientId.trim();
    if (s.username.trim() !== "") saved.username = s.username;
    if (s.password !== "") saved.password = s.password;
    const keepAlive = numberOr(s.keepAliveSecs);
    if (keepAlive !== undefined) saved.keepAliveSecs = keepAlive;
    if (subscriptions.length > 0) saved.subscriptions = subscriptions;
  }
  for (const [key, text] of [
    ["maxMessageBytes", s.maxMessageBytes],
    ["maxBufferedEvents", s.maxBufferedEvents],
    ["maxReconnectAttempts", s.maxReconnectAttempts],
    ["idleTimeoutMs", s.idleTimeoutMs],
  ] as const) {
    const value = numberOr(text);
    if (value !== undefined) saved[key] = value;
  }
  if (s.messages.length > 0) saved.messages = s.messages;
  return saved;
}

function streamToDraft(
  saved: SavedStream | null | undefined,
  kind: Kind,
): StreamDraft {
  const base = newStreamDraft(kind);
  if (!saved) return base;
  const text = (value: number | undefined) =>
    value === undefined ? "" : String(value);
  const subscriptions = (saved.subscriptions ?? []).map((row) => ({
    id: uid(),
    topic: row.topic,
    qos: row.qos ?? 0,
  }));
  subscriptions.push(emptySubscription());
  return {
    ...base,
    subprotocols: (saved.subprotocols ?? []).join("\n"),
    autoReconnect: saved.autoReconnect ?? base.autoReconnect,
    pingIntervalMs: text(saved.pingIntervalMs),
    lastEventId: saved.lastEventId ?? "",
    clientId: saved.clientId ?? "",
    username: saved.username ?? "",
    password: saved.password ?? "",
    cleanSession: saved.cleanSession ?? base.cleanSession,
    keepAliveSecs: text(saved.keepAliveSecs),
    subscriptions,
    protocolVersion: saved.protocolVersion ?? base.protocolVersion,
    maxMessageBytes: text(saved.maxMessageBytes),
    maxBufferedEvents: text(saved.maxBufferedEvents),
    maxReconnectAttempts: text(saved.maxReconnectAttempts),
    idleTimeoutMs: text(saved.idleTimeoutMs),
    messages: saved.messages ?? [],
  };
}

export function toSaved(draft: RequestDraft): SavedRequest {
  return {
    id: draft.id,
    name: draft.name,
    kind: draft.kind,
    method: draft.method,
    url:
      draft.kind === "http" ? urlWithParams(draft.url, draft.params) : draft.url,
    description: blankToNull(draft.description),
    headers: activeRows(draft.headers),
    body: buildBody(draft),
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
    stream: buildStream(draft),
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
  if (auth.type === "inherited") {
    return { ...toAuthDraft(auth.auth), inherited: true };
  }
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

const KINDS: Kind[] = ["http", "graphql", "grpc", ...STREAM_KINDS];

function isKind(value: unknown): value is Kind {
  return KINDS.includes(value as Kind);
}

interface BodyDraftFields {
  bodyType: RequestDraft["bodyType"];
  body: string;
  formRows: KVRow[];
  formDataRows: FormDataRowDraft[];
  binaryFile: string;
  binaryContentType: string;
  graphqlQuery?: string;
  graphqlVariables?: string;
}

/** Repeated file keys from older parses fold into one multi-file field. */
function coalesceFormDataDrafts(rows: FormDataRowPayload[]): FormDataRowDraft[] {
  const out: FormDataRowDraft[] = [];
  for (const row of rows) {
    const draft: FormDataRowDraft = {
      id: uid(),
      key: row.key,
      kind: (row.files?.length ?? 0) > 0 ? "file" : "text",
      value: row.value ?? "",
      files: [...(row.files ?? [])],
      contentType: row.contentType ?? "",
      enabled: row.enabled ?? true,
    };
    const last = out[out.length - 1];
    if (
      last !== undefined &&
      last.kind === "file" &&
      draft.kind === "file" &&
      last.key === draft.key &&
      last.contentType === draft.contentType &&
      last.enabled === draft.enabled
    ) {
      last.files.push(...draft.files);
      continue;
    }
    out.push(draft);
  }
  return out.length > 0 ? out : [emptyFormDataRow()];
}

function bodyToDraft(body: SavedRequest["body"]): BodyDraftFields {
  const base: BodyDraftFields = {
    bodyType: "raw",
    body: "",
    formRows: [emptyRow()],
    formDataRows: [emptyFormDataRow()],
    binaryFile: "",
    binaryContentType: "",
  };
  if (body === null || body === undefined) return base;
  if (typeof body === "string") return { ...base, body };
  switch (body.mode) {
    case "raw":
      return { ...base, body: body.text };
    case "urlencoded":
      return {
        ...base,
        bodyType: "urlencoded",
        formRows: body.rows.map((row) => ({
          id: uid(),
          key: row.key,
          value: row.value ?? "",
          enabled: row.enabled ?? true,
        })),
      };
    case "formdata":
      return {
        ...base,
        bodyType: "formdata",
        formDataRows: coalesceFormDataDrafts(body.rows),
      };
    case "binary":
      return {
        ...base,
        bodyType: "binary",
        binaryFile: body.file,
        binaryContentType: body.contentType ?? "",
      };
    case "graphql":
      return { ...base, graphqlQuery: body.query, graphqlVariables: body.variables };
  }
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
      ? splitQuery(saved.url)
      : { url: saved.url, params: [] as [string, string][] };
  const bodyFields = bodyToDraft(saved.body);
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
    bodyType: bodyFields.bodyType,
    body: bodyFields.body,
    formRows: bodyFields.formRows,
    formDataRows: bodyFields.formDataRows,
    binaryFile: bodyFields.binaryFile,
    binaryContentType: bodyFields.binaryContentType,
    auth: toAuthDraft(saved.auth),
    graphqlQuery: saved.graphql?.query ?? bodyFields.graphqlQuery ?? "",
    graphqlVariables: saved.graphql?.variables ?? bodyFields.graphqlVariables ?? "",
    stream: streamToDraft(saved.stream, saved.kind),
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
