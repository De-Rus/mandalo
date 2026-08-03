import type { Environment, SavedRequest } from "../api";

export const PROTOCOL_VERSION = 1;

export interface CallMessage {
  v: number;
  type: "call";
  id: string;
  command: string;
  args: Record<string, unknown>;
}

export interface ReplyMessage {
  v: number;
  type: "reply";
  id: string;
  ok: boolean;
  value?: unknown;
  error?: { message: string };
}

export interface EventMessage {
  v: number;
  type: "event";
  event: "document" | "environment";
  payload: unknown;
}

export interface EditorContext {
  workspace: string;
  collection: string;
  requestPath: string;
  fileName: string;
}

export interface DocumentPayload {
  request: SavedRequest | null;
  error: string | null;
  context: EditorContext;
}

export interface EnvironmentPayload {
  items: Environment[];
  selected: string | null;
}

export function isReply(value: unknown): value is ReplyMessage {
  if (value === null || typeof value !== "object") return false;
  const message = value as Partial<ReplyMessage>;
  return message.type === "reply" && typeof message.id === "string";
}

export function isEvent(value: unknown): value is EventMessage {
  if (value === null || typeof value !== "object") return false;
  const message = value as Partial<EventMessage>;
  return message.type === "event" && typeof message.event === "string";
}
