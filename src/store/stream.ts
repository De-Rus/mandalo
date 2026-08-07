import { create } from "zustand";
import {
  errorMessage,
  streamClose,
  streamOpen,
  streamSend,
} from "../lib/api";
import type { RequestDraft } from "../lib/draft";
import {
  buildStreamSpec,
  isStreamKind,
  type ConnectionInfo,
  type Outgoing,
  type StreamEvent,
  type StreamKind,
} from "../lib/stream";
import { appendRows, toRow, type LogRow } from "../lib/streamLog";

export type Phase = "closed" | "opening" | "connecting" | "connected" | "reconnecting";

export interface StreamSession {
  kind: StreamKind;
  streamId: string | null;
  phase: Phase;
  connectedAt: number | null;
  info: ConnectionInfo | null;
  attempt: number;
  delayMs: number;
  reason: string | null;
  lastError: string | null;
  rows: LogRow[];
  overflow: number;
  total: number;
  subscriptions: string[];
  sending: boolean;
}

interface StreamState {
  sessions: Record<string, StreamSession>;
  connect: (draft: RequestDraft, vars: Record<string, string>) => Promise<void>;
  disconnect: (id: string) => Promise<void>;
  toggle: (draft: RequestDraft, vars: Record<string, string>) => Promise<void>;
  send: (id: string, message: Outgoing) => Promise<void>;
  clearLog: (id: string) => void;
  forget: (id: string) => Promise<void>;
  closeAll: () => Promise<void>;
}

function blank(kind: StreamKind): StreamSession {
  return {
    kind,
    streamId: null,
    phase: "closed",
    connectedAt: null,
    info: null,
    attempt: 0,
    delayMs: 0,
    reason: null,
    lastError: null,
    rows: [],
    overflow: 0,
    total: 0,
    subscriptions: [],
    sending: false,
  };
}

export function isLive(session: StreamSession | undefined): boolean {
  return session !== undefined && session.phase !== "closed";
}

/**
 * A chatty socket must not cost one React render per frame, so events land in a
 * per-request queue and the store is written once per animation frame.
 */
const pending = new Map<string, StreamEvent[]>();
const frames = new Map<string, number>();

function schedule(id: string): void {
  if (frames.has(id)) return;
  const raf =
    typeof requestAnimationFrame === "function"
      ? requestAnimationFrame
      : (fn: () => void) => setTimeout(fn, 16) as unknown as number;
  frames.set(id, raf(() => flush(id)) as unknown as number);
}

function flush(id: string): void {
  frames.delete(id);
  const queue = pending.get(id);
  pending.delete(id);
  if (!queue || queue.length === 0) return;
  useStreams.setState((s) => {
    const session = s.sessions[id];
    if (!session) return s;
    return { sessions: { ...s.sessions, [id]: reduce(session, queue) } };
  });
}

function subscriptionsAfter(session: StreamSession, event: StreamEvent): string[] {
  if (event.type !== "message" || event.direction !== "outgoing") return session.subscriptions;
  const topic = event.meta.topic;
  if (topic === undefined) return session.subscriptions;
  if (event.meta.frame === "subscribe")
    return session.subscriptions.includes(topic)
      ? session.subscriptions
      : [...session.subscriptions, topic];
  if (event.meta.frame === "unsubscribe")
    return session.subscriptions.filter((t) => t !== topic);
  return session.subscriptions;
}

export function reduce(session: StreamSession, events: StreamEvent[]): StreamSession {
  let next = session;
  const rows: LogRow[] = [];
  let total = session.total;
  for (const event of events) {
    rows.push(toRow(event, total));
    total += 1;
    switch (event.type) {
      case "connecting":
        next = { ...next, phase: "connecting" };
        break;
      case "connected":
        next = {
          ...next,
          phase: "connected",
          connectedAt: event.at,
          info: event.info,
          attempt: 0,
          reason: null,
          lastError: null,
        };
        break;
      case "reconnecting":
        next = {
          ...next,
          phase: "reconnecting",
          connectedAt: null,
          attempt: event.attempt,
          delayMs: event.delayMs,
          reason: event.reason,
        };
        break;
      case "disconnected":
        next = {
          ...next,
          phase: "closed",
          streamId: null,
          connectedAt: null,
          info: null,
          reason: event.reason,
          subscriptions: [],
        };
        break;
      case "error":
        next = { ...next, lastError: event.message };
        break;
      case "message":
        next = { ...next, subscriptions: subscriptionsAfter(next, event) };
        break;
      case "dropped":
        break;
    }
  }
  const appended = appendRows(next.rows, rows, next.overflow);
  return { ...next, rows: appended.rows, overflow: appended.overflow, total };
}

function put(id: string, patch: Partial<StreamSession>): void {
  useStreams.setState((s) => {
    const session = s.sessions[id];
    if (!session) return s;
    return { sessions: { ...s.sessions, [id]: { ...session, ...patch } } };
  });
}

function localEvent(id: string, event: StreamEvent): void {
  const queue = pending.get(id) ?? [];
  queue.push(event);
  pending.set(id, queue);
  flush(id);
}

export const useStreams = create<StreamState>((set, get) => ({
  sessions: {},

  connect: async (draft, vars) => {
    if (!isStreamKind(draft.kind)) return;
    const id = draft.id;
    const existing = get().sessions[id];
    if (existing && existing.phase !== "closed") return;
    const session: StreamSession = {
      ...(existing ?? blank(draft.kind)),
      kind: draft.kind,
      phase: "opening",
      streamId: null,
      reason: null,
      lastError: null,
      attempt: 0,
      subscriptions: [],
    };
    set((s) => ({ sessions: { ...s.sessions, [id]: session } }));

    let spec;
    try {
      spec = buildStreamSpec(draft, vars);
    } catch (e) {
      localEvent(id, {
        type: "error",
        at: Date.now(),
        message: errorMessage(e),
        code: "E_STREAM_SPEC",
      });
      put(id, { phase: "closed" });
      return;
    }

    try {
      const streamId = await streamOpen(spec, (event) => {
        const queue = pending.get(id) ?? [];
        queue.push(event);
        pending.set(id, queue);
        schedule(id);
      });
      const still = get().sessions[id];
      if (!still || still.phase === "closed") {
        await streamClose(streamId).catch(() => {});
        return;
      }
      put(id, { streamId });
    } catch (e) {
      localEvent(id, {
        type: "error",
        at: Date.now(),
        message: errorMessage(e),
        code: "E_STREAM_CONNECT",
      });
      localEvent(id, {
        type: "disconnected",
        at: Date.now(),
        reason: "the connection was never established",
      });
    }
  },

  disconnect: async (id) => {
    const session = get().sessions[id];
    if (!session) return;
    const streamId = session.streamId;
    put(id, { phase: "closed", streamId: null, connectedAt: null, info: null });
    if (!streamId) {
      localEvent(id, {
        type: "disconnected",
        at: Date.now(),
        reason: "closed by you before it connected",
      });
      return;
    }
    try {
      await streamClose(streamId);
    } catch (e) {
      localEvent(id, {
        type: "error",
        at: Date.now(),
        message: errorMessage(e),
        code: "E_STREAM_CLOSE",
      });
    }
  },

  toggle: async (draft, vars) => {
    const session = get().sessions[draft.id];
    if (isLive(session)) await get().disconnect(draft.id);
    else await get().connect(draft, vars);
  },

  send: async (id, message) => {
    const session = get().sessions[id];
    if (!session?.streamId) return;
    put(id, { sending: true });
    try {
      await streamSend(session.streamId, message);
    } catch (e) {
      localEvent(id, {
        type: "error",
        at: Date.now(),
        message: errorMessage(e),
        code: "E_STREAM_SEND",
      });
    } finally {
      put(id, { sending: false });
    }
  },

  clearLog: (id) =>
    set((s) => {
      const session = s.sessions[id];
      if (!session) return s;
      return {
        sessions: { ...s.sessions, [id]: { ...session, rows: [], overflow: 0 } },
      };
    }),

  forget: async (id) => {
    await get().disconnect(id);
    pending.delete(id);
    set((s) => {
      if (!(id in s.sessions)) return s;
      const sessions = { ...s.sessions };
      delete sessions[id];
      return { sessions };
    });
  },

  closeAll: async () => {
    const ids = Object.keys(get().sessions);
    await Promise.all(ids.map((id) => get().disconnect(id)));
  },
}));

export function useStreamSession(id: string | null): StreamSession | undefined {
  return useStreams((s) => (id ? s.sessions[id] : undefined));
}

export function liveStreamIds(): string[] {
  const { sessions } = useStreams.getState();
  return Object.keys(sessions).filter((id) => isLive(sessions[id]));
}

if (typeof window !== "undefined")
  window.addEventListener("beforeunload", () => {
    void useStreams.getState().closeAll();
  });
