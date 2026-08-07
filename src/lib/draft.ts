import type { Capture, Kind, TestAssertion } from "./api";
import { defaultAutoReconnect, isStreamKind, type MqttVersion, type SavedMessage } from "./stream";

export interface KVRow {
  id: string;
  key: string;
  value: string;
  enabled: boolean;
}

export type AuthType = "none" | "bearer" | "basic" | "apikey";

export interface AuthDraft {
  type: AuthType;
  /** True when the file marked this auth `# @auth inherited`. */
  inherited: boolean;
  token: string;
  username: string;
  password: string;
  key: string;
  value: string;
  placement: "header" | "query";
}

export type BodyType = "raw" | "urlencoded" | "formdata" | "binary";

export interface FormDataRowDraft {
  id: string;
  key: string;
  kind: "text" | "file";
  value: string;
  files: string[];
  contentType: string;
  enabled: boolean;
}

export interface GrpcDraft {
  protoPaths: string;
  service: string;
  method: string;
  message: string;
  metadata: KVRow[];
}

export interface SubscriptionRow {
  id: string;
  topic: string;
  qos: number;
}

/** Every realtime protocol's options in one flat record, like the body editors. */
export interface StreamDraft {
  subprotocols: string;
  autoReconnect: boolean;
  pingIntervalMs: string;
  lastEventId: string;
  clientId: string;
  username: string;
  password: string;
  cleanSession: boolean;
  keepAliveSecs: string;
  subscriptions: SubscriptionRow[];
  protocolVersion: MqttVersion;
  maxMessageBytes: string;
  maxBufferedEvents: string;
  maxReconnectAttempts: string;
  idleTimeoutMs: string;
  messages: SavedMessage[];
}

export interface RequestDraft {
  id: string;
  name: string;
  kind: Kind;
  method: string;
  url: string;
  description: string;
  collection: string;
  path: string | null;
  params: KVRow[];
  headers: KVRow[];
  bodyType: BodyType;
  body: string;
  formRows: KVRow[];
  formDataRows: FormDataRowDraft[];
  binaryFile: string;
  binaryContentType: string;
  auth: AuthDraft;
  graphqlQuery: string;
  graphqlVariables: string;
  grpc: GrpcDraft;
  stream: StreamDraft;
  preScript: string;
  testScript: string;
  tests: TestAssertion[];
  captures: Capture[];
}

export function uid(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36);
}

export function emptyRow(): KVRow {
  return { id: uid(), key: "", value: "", enabled: true };
}

/** The slice of a draft the body editors own, so they need no wider type. */
export type BodyDraft = Pick<
  RequestDraft,
  "bodyType" | "body" | "formRows" | "formDataRows" | "binaryFile" | "binaryContentType"
>;

export function bodyHasContent(draft: BodyDraft): boolean {
  switch (draft.bodyType) {
    case "raw":
      return draft.body.trim() !== "";
    case "urlencoded":
      return draft.formRows.some((row) => row.key.trim() !== "");
    case "formdata":
      return draft.formDataRows.some((row) => row.key.trim() !== "");
    case "binary":
      return draft.binaryFile.trim() !== "";
  }
}

export function emptyFormDataRow(): FormDataRowDraft {
  return {
    id: uid(),
    key: "",
    kind: "text",
    value: "",
    files: [],
    contentType: "",
    enabled: true,
  };
}

export function emptySubscription(): SubscriptionRow {
  return { id: uid(), topic: "", qos: 0 };
}

export function newStreamDraft(kind: Kind = "websocket"): StreamDraft {
  return {
    subprotocols: "",
    autoReconnect: isStreamKind(kind) ? defaultAutoReconnect(kind) : false,
    pingIntervalMs: "",
    lastEventId: "",
    clientId: "",
    username: "",
    password: "",
    cleanSession: true,
    keepAliveSecs: "",
    subscriptions: [emptySubscription()],
    protocolVersion: "3.1.1",
    maxMessageBytes: "",
    maxBufferedEvents: "",
    maxReconnectAttempts: "",
    idleTimeoutMs: "",
    messages: [],
  };
}

export function newDraft(
  name = "New Request",
  kind: Kind = "http",
  collection = "",
): RequestDraft {
  return {
    id: uid(),
    name,
    kind,
    method: kind === "http" ? "GET" : "POST",
    url: "",
    description: "",
    collection,
    path: null,
    params: [emptyRow()],
    headers: [emptyRow()],
    bodyType: "raw",
    body: "",
    formRows: [emptyRow()],
    formDataRows: [emptyFormDataRow()],
    binaryFile: "",
    binaryContentType: "",
    auth: {
      type: "none",
      inherited: false,
      token: "",
      username: "",
      password: "",
      key: "",
      value: "",
      placement: "header",
    },
    graphqlQuery: "",
    graphqlVariables: "",
    grpc: {
      protoPaths: "",
      service: "",
      method: "",
      message: "",
      metadata: [emptyRow()],
    },
    stream: newStreamDraft(kind),
    preScript: "",
    testScript: "",
    tests: [],
    captures: [],
  };
}
