import type { Auth } from "./api";
import { buildAuth } from "./spec";
import { activeRows } from "./spec";
import type { RequestDraft, StreamDraft } from "./draft";

export type StreamKind = "websocket" | "sse" | "mqtt";

export const STREAM_KINDS: StreamKind[] = ["websocket", "sse", "mqtt"];

export function isStreamKind(kind: string): kind is StreamKind {
  return (STREAM_KINDS as string[]).includes(kind);
}

export type Payload =
  | { kind: "text"; text: string }
  | { kind: "binary"; base64: string; bytes: number };

export type Direction = "incoming" | "outgoing";

export interface MessageMeta {
  event?: string;
  id?: string;
  topic?: string;
  qos?: number;
  retain?: boolean;
  frame?: string;
}

export interface ConnectionInfo {
  url: string;
  status?: number;
  protocol?: string;
  headers?: [string, string][];
  sessionPresent?: boolean;
}

export type StreamEvent =
  | { type: "connecting"; at: number; url: string }
  | { type: "connected"; at: number; info: ConnectionInfo }
  | {
      type: "message";
      at: number;
      direction: Direction;
      payload: Payload;
      meta: MessageMeta;
    }
  | { type: "reconnecting"; at: number; attempt: number; delayMs: number; reason: string }
  | { type: "dropped"; at: number; count: number; reason: string }
  | { type: "disconnected"; at: number; code?: number; reason: string }
  | { type: "error"; at: number; message: string; code: string };

export type Outgoing =
  | { kind: "text"; text: string }
  | { kind: "binary"; base64: string }
  | { kind: "publish"; topic: string; payload: string; qos?: number; retain?: boolean }
  | { kind: "subscribe"; topic: string; qos?: number }
  | { kind: "unsubscribe"; topic: string }
  | { kind: "ping" };

export interface StreamLimits {
  maxMessageBytes?: number;
  maxBufferedEvents?: number;
  maxBufferedBytes?: number;
  maxReconnectAttempts?: number;
  connectTimeoutMs?: number;
  idleTimeoutMs?: number;
  backoffBaseMs?: number;
  backoffMaxMs?: number;
}

export interface WsOptions {
  subprotocols?: string[];
  autoReconnect?: boolean;
  pingIntervalMs?: number;
}

export interface SseOptions {
  lastEventId?: string;
  autoReconnect?: boolean;
}

export type MqttVersion = "3.1.1" | "5";

export interface Subscription {
  topic: string;
  qos?: number;
}

export interface MqttOptions {
  clientId?: string;
  username?: string;
  password?: string;
  cleanSession?: boolean;
  keepAliveSecs?: number;
  subscriptions?: Subscription[];
  protocolVersion?: MqttVersion;
}

export interface StreamSpec {
  kind: StreamKind;
  url: string;
  headers?: [string, string][];
  auth?: Auth;
  vars?: Record<string, string>;
  limits?: StreamLimits;
  ws?: WsOptions;
  sse?: SseOptions;
  mqtt?: MqttOptions;
}

export interface StreamStatus {
  id: string;
  kind: StreamKind;
  url: string;
  open: boolean;
}

/** A message the user named and saved with the request, one click from sending. */
export interface SavedMessage {
  id: string;
  name: string;
  message: Outgoing;
}

export const STREAM_LABELS: Record<StreamKind, string> = {
  websocket: "WebSocket",
  sse: "SSE",
  mqtt: "MQTT",
};

/**
 * What the engine does when nothing says otherwise: a websocket stays down, an
 * event stream comes back, and mqtt always reconnects. A file that mentions no
 * reconnect policy must mean the same thing here as it does Rust-side.
 */
export function defaultAutoReconnect(kind: StreamKind): boolean {
  return kind !== "websocket";
}

export function placeholderUrl(kind: StreamKind): string {
  switch (kind) {
    case "websocket":
      return "wss://{{host}}/ws/echo";
    case "sse":
      return "{{baseUrl}}/sse/basic";
    case "mqtt":
      return "mqtt://localhost:1883";
  }
}

function positive(value: string): number | undefined {
  const n = Number(value.trim());
  return value.trim() === "" || Number.isNaN(n) || n < 0 ? undefined : n;
}

export function parseSubprotocols(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter((s) => s !== "");
}

function limitsOf(stream: StreamDraft): StreamLimits | undefined {
  const limits: StreamLimits = {};
  const message = positive(stream.maxMessageBytes);
  const events = positive(stream.maxBufferedEvents);
  const attempts = positive(stream.maxReconnectAttempts);
  const idle = positive(stream.idleTimeoutMs);
  if (message !== undefined) limits.maxMessageBytes = message;
  if (events !== undefined) limits.maxBufferedEvents = events;
  if (attempts !== undefined) limits.maxReconnectAttempts = attempts;
  if (idle !== undefined) limits.idleTimeoutMs = idle;
  return Object.keys(limits).length === 0 ? undefined : limits;
}

/**
 * The wire spec for one connect. Every options object the engine defaults is
 * omitted when the draft says nothing, so a spec never pins a value the user did
 * not choose.
 */
export function buildStreamSpec(
  draft: RequestDraft,
  vars: Record<string, string>,
): StreamSpec {
  if (!isStreamKind(draft.kind))
    throw new Error(`${draft.kind} is not a realtime protocol`);
  const stream = draft.stream;
  const spec: StreamSpec = {
    kind: draft.kind,
    url: draft.url,
    headers: activeRows(draft.headers),
    auth: buildAuth(draft.auth),
    vars,
  };
  const limits = limitsOf(stream);
  if (limits) spec.limits = limits;

  if (draft.kind === "websocket") {
    const subprotocols = parseSubprotocols(stream.subprotocols);
    spec.ws = {
      autoReconnect: stream.autoReconnect,
      ...(subprotocols.length > 0 ? { subprotocols } : {}),
      ...(positive(stream.pingIntervalMs) !== undefined
        ? { pingIntervalMs: positive(stream.pingIntervalMs) }
        : {}),
    };
  }
  if (draft.kind === "sse") {
    spec.sse = {
      autoReconnect: stream.autoReconnect,
      ...(stream.lastEventId.trim() === ""
        ? {}
        : { lastEventId: stream.lastEventId.trim() }),
    };
  }
  if (draft.kind === "mqtt") {
    const subscriptions = stream.subscriptions
      .filter((row) => row.topic.trim() !== "")
      .map((row) => ({ topic: row.topic.trim(), qos: row.qos }));
    spec.mqtt = {
      cleanSession: stream.cleanSession,
      protocolVersion: stream.protocolVersion,
      ...(stream.clientId.trim() === "" ? {} : { clientId: stream.clientId.trim() }),
      ...(stream.username.trim() === "" ? {} : { username: stream.username }),
      ...(stream.password === "" ? {} : { password: stream.password }),
      ...(positive(stream.keepAliveSecs) !== undefined
        ? { keepAliveSecs: positive(stream.keepAliveSecs) }
        : {}),
      ...(subscriptions.length > 0 ? { subscriptions } : {}),
    };
  }
  return spec;
}
