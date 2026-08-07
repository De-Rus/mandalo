import { prettyJson } from "./format";
import type { Payload, StreamEvent } from "./stream";

/**
 * How many rows the log keeps. The engine already caps its own buffer and says
 * so with a `dropped` event; this is the second cap, on the DOM, so a firehose
 * costs a bounded amount of memory and layout instead of the window.
 */
export const LOG_CAP = 2000;

export type RowTone = "incoming" | "outgoing" | "lifecycle" | "error";

export interface LogRow {
  seq: number;
  at: number;
  type: StreamEvent["type"];
  tone: RowTone;
  gutter: string;
  summary: string;
  detail: string | null;
  pretty: string | null;
  chips: string[];
  payload: Payload | null;
  bytes: number | null;
  expandable: boolean;
}

export function formatClock(at: number): string {
  const d = new Date(at);
  const pad = (n: number, width = 2) => String(n).padStart(width, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(
    d.getMilliseconds(),
    3,
  )}`;
}

export function formatUptime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** A preview of raw bytes that can never put a control character in the DOM. */
export function hexPreview(base64: string, limit = 32): string {
  let binary: string;
  try {
    binary = atob(base64);
  } catch {
    return "";
  }
  const shown = Array.from(binary.slice(0, limit))
    .map((c) => c.charCodeAt(0).toString(16).padStart(2, "0"))
    .join(" ");
  return binary.length > limit ? `${shown} …` : shown;
}

function oneLine(text: string, limit = 300): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > limit ? `${flat.slice(0, limit)}…` : flat;
}

function payloadChips(payload: Payload): string[] {
  return payload.kind === "binary" ? ["binary", `${payload.bytes} B`] : [];
}

function metaChips(meta: NonNullable<Extract<StreamEvent, { type: "message" }>["meta"]>): string[] {
  const chips: string[] = [];
  if (meta.topic) chips.push(meta.topic);
  if (meta.event) chips.push(`event ${meta.event}`);
  if (meta.id) chips.push(`id ${meta.id}`);
  if (meta.qos !== undefined) chips.push(`QoS ${meta.qos}`);
  if (meta.retain) chips.push("retained");
  if (meta.frame && meta.frame !== "event" && meta.frame !== "text")
    chips.push(meta.frame);
  return chips;
}

/**
 * One event becomes exactly one row. Lifecycle events are rows too — they are
 * the story of the connection, and a log that hides them cannot be debugged
 * from.
 */
export function toRow(event: StreamEvent, seq: number): LogRow {
  const base = { seq, at: event.at, type: event.type };
  switch (event.type) {
    case "connecting":
      return {
        ...base,
        tone: "lifecycle",
        gutter: "·",
        summary: `Connecting to ${event.url}`,
        detail: event.url,
        pretty: null,
        chips: [],
        payload: null,
        bytes: null,
        expandable: false,
      };
    case "connected": {
      const chips: string[] = [];
      if (event.info.status !== undefined) chips.push(`HTTP ${event.info.status}`);
      if (event.info.protocol) chips.push(event.info.protocol);
      if (event.info.sessionPresent !== undefined)
        chips.push(event.info.sessionPresent ? "session resumed" : "clean session");
      const headers = event.info.headers ?? [];
      return {
        ...base,
        tone: "lifecycle",
        gutter: "·",
        summary: `Connected to ${event.info.url}`,
        detail:
          headers.length > 0
            ? headers.map(([k, v]) => `${k}: ${v}`).join("\n")
            : event.info.url,
        pretty: null,
        chips,
        payload: null,
        bytes: null,
        expandable: headers.length > 0,
      };
    }
    case "message": {
      const payload = event.payload;
      const binary = payload.kind === "binary";
      const detail = binary
        ? `${payload.bytes} bytes\n${hexPreview(payload.base64, 256)}`
        : payload.text;
      const pretty = binary ? null : prettyJson(payload.text);
      const bytes = binary
        ? payload.bytes
        : new TextEncoder().encode(payload.text).length;
      return {
        ...base,
        tone: event.direction,
        gutter: event.direction === "incoming" ? "IN" : "OUT",
        summary: binary ? `${payload.bytes} bytes of binary` : oneLine(payload.text),
        detail,
        pretty,
        chips: [...metaChips(event.meta), ...payloadChips(event.payload)],
        payload: event.payload,
        bytes,
        expandable: true,
      };
    }
    case "reconnecting":
      return {
        ...base,
        tone: "lifecycle",
        gutter: "·",
        summary: `Reconnecting — attempt ${event.attempt} in ${event.delayMs} ms · ${event.reason}`,
        detail: event.reason,
        pretty: null,
        chips: [`attempt ${event.attempt}`, `${event.delayMs} ms`],
        payload: null,
        bytes: null,
        expandable: false,
      };
    case "dropped":
      return {
        ...base,
        tone: "error",
        gutter: "·",
        summary: `Dropped ${event.count} message${event.count === 1 ? "" : "s"} — ${event.reason}`,
        detail: event.reason,
        pretty: null,
        chips: [`${event.count} dropped`],
        payload: null,
        bytes: null,
        expandable: false,
      };
    case "disconnected":
      return {
        ...base,
        tone: "lifecycle",
        gutter: "·",
        summary:
          event.code === undefined
            ? `Disconnected — ${event.reason}`
            : `Disconnected ${event.code} — ${event.reason}`,
        detail: event.reason,
        pretty: null,
        chips: event.code === undefined ? [] : [`code ${event.code}`],
        payload: null,
        bytes: null,
        expandable: false,
      };
    case "error":
      return {
        ...base,
        tone: "error",
        gutter: "!",
        summary: event.message,
        detail: event.message,
        pretty: null,
        chips: [event.code],
        payload: null,
        bytes: null,
        expandable: event.message.length > 120,
      };
  }
}

export interface Appended {
  rows: LogRow[];
  /** How many rows the cap threw away over the life of this log. */
  overflow: number;
}

export function appendRows(
  rows: LogRow[],
  incoming: LogRow[],
  overflow: number,
  cap = LOG_CAP,
): Appended {
  if (incoming.length === 0) return { rows, overflow };
  const all = rows.concat(incoming);
  if (all.length <= cap) return { rows: all, overflow };
  const cut = all.length - cap;
  return { rows: all.slice(cut), overflow: overflow + cut };
}

export type LogFilter = "all" | "incoming" | "outgoing" | "lifecycle";

export function matchesFilter(row: LogRow, filter: LogFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "incoming":
      return row.tone === "incoming";
    case "outgoing":
      return row.tone === "outgoing";
    case "lifecycle":
      return row.tone === "lifecycle" || row.tone === "error";
  }
}

export function matchesQuery(row: LogRow, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") return true;
  if (row.summary.toLowerCase().includes(needle)) return true;
  if (row.detail !== null && row.detail.toLowerCase().includes(needle)) return true;
  return row.chips.some((chip) => chip.toLowerCase().includes(needle));
}

export function visibleRows(
  rows: LogRow[],
  filter: LogFilter,
  query: string,
): LogRow[] {
  if (filter === "all" && query.trim() === "") return rows;
  return rows.filter((row) => matchesFilter(row, filter) && matchesQuery(row, query));
}

export function copyText(row: LogRow): string {
  return row.pretty ?? row.detail ?? row.summary;
}
