import { describe, expect, it } from "vitest";
import type { StreamEvent } from "./stream";
import {
  appendRows,
  formatUptime,
  hexPreview,
  LOG_CAP,
  matchesFilter,
  toRow,
  visibleRows,
  type LogRow,
} from "./streamLog";

const AT = 1_764_000_000_000;

const EVENTS: Record<string, StreamEvent> = {
  connecting: { type: "connecting", at: AT, url: "wss://x.dev/ws" },
  connected: {
    type: "connected",
    at: AT,
    info: { url: "wss://x.dev/ws", protocol: "v2", status: 101 },
  },
  incoming: {
    type: "message",
    at: AT,
    direction: "incoming",
    payload: { kind: "text", text: '{"id": 7}' },
    meta: { event: "tick", id: "9" },
  },
  outgoing: {
    type: "message",
    at: AT,
    direction: "outgoing",
    payload: { kind: "text", text: "hello" },
    meta: { topic: "a/b", qos: 1, retain: true },
  },
  binary: {
    type: "message",
    at: AT,
    direction: "incoming",
    payload: { kind: "binary", base64: "/wD+", bytes: 3 },
    meta: {},
  },
  reconnecting: {
    type: "reconnecting",
    at: AT,
    attempt: 3,
    delayMs: 2000,
    reason: "the server went away",
  },
  dropped: { type: "dropped", at: AT, count: 12, reason: "the event buffer was full" },
  disconnected: { type: "disconnected", at: AT, code: 4001, reason: "bye" },
  error: { type: "error", at: AT, message: "cannot reach x.dev", code: "E_STREAM_CONNECT" },
};

describe("event to row", () => {
  it("gives every event kind a row", () => {
    for (const [name, event] of Object.entries(EVENTS)) {
      const row = toRow(event, 0);
      expect(row.type, name).toBe(event.type);
      expect(row.summary.length, name).toBeGreaterThan(0);
      expect(row.at, name).toBe(AT);
    }
  });

  it("keeps direction in the tone and the gutter", () => {
    expect(toRow(EVENTS.incoming, 0).tone).toBe("incoming");
    expect(toRow(EVENTS.incoming, 0).gutter).toBe("IN");
    expect(toRow(EVENTS.outgoing, 1).tone).toBe("outgoing");
    expect(toRow(EVENTS.outgoing, 1).gutter).toBe("OUT");
  });

  it("marks lifecycle events quieter than traffic", () => {
    for (const name of ["connecting", "connected", "reconnecting", "disconnected"])
      expect(toRow(EVENTS[name], 0).tone, name).toBe("lifecycle");
    for (const name of ["dropped", "error"])
      expect(toRow(EVENTS[name], 0).tone, name).toBe("error");
  });

  it("says the attempt and the delay on a reconnect", () => {
    const row = toRow(EVENTS.reconnecting, 0);
    expect(row.summary).toContain("attempt 3");
    expect(row.summary).toContain("2000 ms");
    expect(row.chips).toEqual(["attempt 3", "2000 ms"]);
  });

  it("says how many were dropped, and why", () => {
    const row = toRow(EVENTS.dropped, 0);
    expect(row.summary).toContain("Dropped 12 messages");
    expect(row.summary).toContain("the event buffer was full");
  });

  it("says the close code and reason", () => {
    expect(toRow(EVENTS.disconnected, 0).summary).toBe("Disconnected 4001 — bye");
    expect(
      toRow({ type: "disconnected", at: AT, reason: "gone" }, 0).summary,
    ).toBe("Disconnected — gone");
  });

  it("carries the negotiated protocol on connected", () => {
    expect(toRow(EVENTS.connected, 0).chips).toEqual(["HTTP 101", "v2"]);
  });

  it("pretty-prints JSON only when the payload is JSON", () => {
    expect(toRow(EVENTS.incoming, 0).pretty).toBe('{\n  "id": 7\n}');
    expect(toRow(EVENTS.outgoing, 0).pretty).toBeNull();
  });

  it("shows a binary frame by size and hex, never as bytes", () => {
    const row = toRow(EVENTS.binary, 0);
    expect(row.summary).toBe("3 bytes of binary");
    expect(row.detail).toContain("ff 00 fe");
    expect(row.bytes).toBe(3);
  });

  it("turns mqtt meta into chips", () => {
    expect(toRow(EVENTS.outgoing, 0).chips).toEqual(["a/b", "QoS 1", "retained"]);
  });

  it("collapses a multi-line payload to one line, and keeps the whole thing to expand", () => {
    const row = toRow(
      {
        type: "message",
        at: AT,
        direction: "incoming",
        payload: { kind: "text", text: "one\ntwo\nthree" },
        meta: {},
      },
      0,
    );
    expect(row.summary).toBe("one two three");
    expect(row.detail).toBe("one\ntwo\nthree");
    expect(row.expandable).toBe(true);
  });
});

describe("the log cap", () => {
  const rowsOf = (n: number, from = 0): LogRow[] =>
    Array.from({ length: n }, (_, i) => toRow(EVENTS.incoming, from + i));

  it("keeps everything under the cap", () => {
    const { rows, overflow } = appendRows([], rowsOf(10), 0, 100);
    expect(rows).toHaveLength(10);
    expect(overflow).toBe(0);
  });

  it("drops the oldest rows and counts them", () => {
    const first = appendRows([], rowsOf(100), 0, 100);
    const second = appendRows(first.rows, rowsOf(25, 100), first.overflow, 100);
    expect(second.rows).toHaveLength(100);
    expect(second.overflow).toBe(25);
    expect(second.rows[0].seq).toBe(25);
    expect(second.rows[99].seq).toBe(124);
  });

  it("survives a burst larger than the whole cap", () => {
    const { rows, overflow } = appendRows([], rowsOf(5000), 0, LOG_CAP);
    expect(rows).toHaveLength(LOG_CAP);
    expect(overflow).toBe(5000 - LOG_CAP);
    expect(rows[rows.length - 1].seq).toBe(4999);
  });

  it("does nothing when nothing arrived", () => {
    const rows = rowsOf(3);
    expect(appendRows(rows, [], 7, 100)).toEqual({ rows, overflow: 7 });
  });
});

describe("filtering and search", () => {
  const rows = [
    toRow(EVENTS.incoming, 0),
    toRow(EVENTS.outgoing, 1),
    toRow(EVENTS.connected, 2),
    toRow(EVENTS.error, 3),
  ];

  it("keeps lifecycle and error rows under the connection filter", () => {
    expect(rows.filter((r) => matchesFilter(r, "lifecycle")).map((r) => r.seq)).toEqual([
      2, 3,
    ]);
  });

  it("filters by direction", () => {
    expect(visibleRows(rows, "incoming", "").map((r) => r.seq)).toEqual([0]);
    expect(visibleRows(rows, "outgoing", "").map((r) => r.seq)).toEqual([1]);
  });

  it("searches the summary, the detail and the chips", () => {
    expect(visibleRows(rows, "all", "a/b").map((r) => r.seq)).toEqual([1]);
    expect(visibleRows(rows, "all", "\"id\"").map((r) => r.seq)).toEqual([0]);
    expect(visibleRows(rows, "all", "nothing here")).toEqual([]);
  });
});

describe("formatting", () => {
  it("reads an uptime as a clock", () => {
    expect(formatUptime(0)).toBe("0:00");
    expect(formatUptime(65_000)).toBe("1:05");
    expect(formatUptime(3_725_000)).toBe("1:02:05");
  });

  it("previews bytes as hex and stops at the limit", () => {
    expect(hexPreview("/wD+")).toBe("ff 00 fe");
    expect(hexPreview("/wD+", 2)).toBe("ff 00 …");
    expect(hexPreview("not base64 at all!!")).toBe("");
  });
});
