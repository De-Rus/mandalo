import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent, StreamSpec } from "../stream";
import {
  base64ToBytes,
  bytesToBase64,
  NO_MQTT5,
  NO_RAW_MQTT,
  NO_WS_HEADERS,
  NO_WS_PING,
  payloadOf,
  resolveSpec,
  webStreamClose,
  webStreamList,
  webStreamOpen,
  webStreamSend,
} from "./stream";

class FakeSocket {
  static last: FakeSocket | null = null;
  static readonly OPEN = 1;
  readyState = 0;
  binaryType = "";
  protocol = "";
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((e: { code: number; reason: string; wasClean: boolean }) => void) | null = null;

  constructor(
    readonly url: string,
    readonly subprotocols?: string | string[],
  ) {
    FakeSocket.last = this;
  }

  send(data: unknown): void {
    this.sent.push(data);
  }

  close(code = 1000, reason = ""): void {
    this.readyState = 3;
    this.onclose?.({ code, reason, wasClean: true });
  }

  accept(protocol = ""): void {
    this.readyState = 1;
    this.protocol = protocol;
    this.onopen?.();
  }
}

function collect(spec: StreamSpec): { events: StreamEvent[]; streamId: string } {
  const events: StreamEvent[] = [];
  const { streamId } = webStreamOpen(spec, { onmessage: (e) => events.push(e) });
  return { events, streamId };
}

const original = globalThis.WebSocket;

beforeEach(() => {
  FakeSocket.last = null;
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
});

afterEach(async () => {
  for (const status of webStreamList()) await webStreamClose(status.id).catch(() => {});
  globalThis.WebSocket = original;
  vi.restoreAllMocks();
});

describe("what the browser cannot do, it says out loud", () => {
  it("refuses a websocket that needs headers instead of dropping them", () => {
    expect(() =>
      collect({
        kind: "websocket",
        url: "wss://x.dev/ws",
        headers: [["X-Trace", "1"]],
      }),
    ).toThrow(NO_WS_HEADERS);
  });

  it("refuses bearer auth on a websocket, because that is a header too", () => {
    expect(() =>
      collect({
        kind: "websocket",
        url: "wss://x.dev/ws",
        auth: { type: "bearer", token: "t" },
      }),
    ).toThrow(NO_WS_HEADERS);
  });

  it("accepts an api key in the query, which a browser can carry", () => {
    const { events } = collect({
      kind: "websocket",
      url: "wss://x.dev/ws",
      auth: { type: "apikey", key: "k", value: "v", placement: "query" },
    });
    expect(events[0]).toMatchObject({ type: "connecting" });
    expect(FakeSocket.last?.url).toBe("wss://x.dev/ws?k=v");
  });

  it("refuses client pings", () => {
    expect(() =>
      collect({ kind: "websocket", url: "wss://x.dev/ws", ws: { pingIntervalMs: 5000 } }),
    ).toThrow(NO_WS_PING);
  });

  it("refuses a raw mqtt:// url, which needs a TCP socket", () => {
    expect(() => collect({ kind: "mqtt", url: "mqtt://localhost:1883" })).toThrow(
      NO_RAW_MQTT,
    );
  });

  it("refuses MQTT 5 over the websocket transport", () => {
    expect(() =>
      collect({
        kind: "mqtt",
        url: "ws://localhost:8083/mqtt",
        mqtt: { protocolVersion: "5" },
      }),
    ).toThrow(NO_MQTT5);
  });

  it("holds every kind to its own scheme", () => {
    expect(() => collect({ kind: "websocket", url: "https://x.dev" })).toThrow(
      /ws:\/\/ or wss:\/\//,
    );
    expect(() => collect({ kind: "sse", url: "ws://x.dev" })).toThrow(
      /http:\/\/ or https:\/\//,
    );
  });
});

describe("resolving a spec", () => {
  it("interpolates the url, the headers and the credentials", () => {
    const resolved = resolveSpec({
      kind: "sse",
      url: "https://{{host}}/events",
      headers: [["X-{{tag}}", "{{tag}}-1"]],
      auth: { type: "bearer", token: "{{token}}" },
      vars: { host: "x.dev", tag: "trace", token: "s3cret" },
    });
    expect(resolved.url).toBe("https://x.dev/events");
    expect(resolved.headers).toEqual([
      ["X-trace", "trace-1"],
      ["Authorization", "Bearer s3cret"],
    ]);
  });

  it("lets auth replace a hand-written authorization header", () => {
    const resolved = resolveSpec({
      kind: "sse",
      url: "https://x.dev/e",
      headers: [["authorization", "stale"]],
      auth: { type: "bearer", token: "fresh" },
      vars: {},
    });
    expect(resolved.headers).toEqual([["Authorization", "Bearer fresh"]]);
  });

  it("fails loud on an unresolved variable", () => {
    expect(() =>
      resolveSpec({ kind: "sse", url: "https://{{host}}/e", vars: {} }),
    ).toThrow(/unresolved variable: host/);
  });
});

describe("a websocket connection", () => {
  const spec: StreamSpec = {
    kind: "websocket",
    url: "wss://x.dev/ws",
    ws: { subprotocols: ["v2"] },
  };

  it("reports connecting, then connected with the negotiated subprotocol", () => {
    const { events } = collect(spec);
    FakeSocket.last?.accept("v2");
    expect(events.map((e) => e.type)).toEqual(["connecting", "connected"]);
    expect(events[1]).toMatchObject({ info: { protocol: "v2" } });
    expect(FakeSocket.last?.subprotocols).toEqual(["v2"]);
  });

  it("turns an inbound text frame into an incoming message", () => {
    const { events } = collect(spec);
    FakeSocket.last?.accept();
    FakeSocket.last?.onmessage?.({ data: "hello" });
    expect(events[2]).toMatchObject({
      type: "message",
      direction: "incoming",
      payload: { kind: "text", text: "hello" },
    });
  });

  it("keeps a non-utf8 frame as binary, never as mangled text", () => {
    const { events } = collect(spec);
    FakeSocket.last?.accept();
    FakeSocket.last?.onmessage?.({ data: new Uint8Array([0xff, 0x00, 0xfe]).buffer });
    expect(events[2]).toMatchObject({
      payload: { kind: "binary", base64: "/wD+", bytes: 3 },
    });
  });

  it("echoes what it sent as an outgoing message", async () => {
    const { events, streamId } = collect(spec);
    FakeSocket.last?.accept();
    await webStreamSend(streamId, { kind: "text", text: "ping" });
    expect(FakeSocket.last?.sent).toEqual(["ping"]);
    expect(events[2]).toMatchObject({ type: "message", direction: "outgoing" });
  });

  it("refuses an mqtt operation on a websocket", async () => {
    const { streamId } = collect(spec);
    FakeSocket.last?.accept();
    await expect(
      webStreamSend(streamId, { kind: "publish", topic: "a", payload: "b" }),
    ).rejects.toThrow(/cannot send publish/);
  });

  it("refuses to send before the socket is open", async () => {
    const { streamId } = collect(spec);
    await expect(webStreamSend(streamId, { kind: "text", text: "early" })).rejects.toThrow(
      /not open/,
    );
  });

  it("reports the close code and reason the server gave", () => {
    const { events } = collect(spec);
    FakeSocket.last?.accept();
    FakeSocket.last?.close(4001, "go away");
    expect(events[events.length - 1]).toMatchObject({
      type: "disconnected",
      code: 4001,
      reason: "go away",
    });
  });

  it("treats a close frame as the end, even with reconnect on — as the engine does", () => {
    const { events } = collect({ ...spec, ws: { autoReconnect: true } });
    FakeSocket.last?.accept();
    FakeSocket.last?.close(4001, "go away");
    expect(events.map((e) => e.type)).toEqual([
      "connecting",
      "connected",
      "disconnected",
    ]);
  });

  it("retries a connection that broke without a close frame", () => {
    const { events } = collect({ ...spec, ws: { autoReconnect: true } });
    const socket = FakeSocket.last as FakeSocket;
    socket.accept();
    socket.readyState = 3;
    socket.onclose?.({ code: 1006, reason: "", wasClean: false });
    expect(events[events.length - 1]).toMatchObject({ type: "reconnecting", attempt: 1 });
  });

  it("forgets a stream once it has disconnected", () => {
    collect(spec);
    expect(webStreamList()).toHaveLength(1);
    FakeSocket.last?.accept();
    FakeSocket.last?.close(1000, "done");
    expect(webStreamList()).toHaveLength(0);
  });

  it("closes deliberately when asked, and says who closed it", async () => {
    const { events, streamId } = collect(spec);
    FakeSocket.last?.accept();
    await webStreamClose(streamId);
    expect(events[events.length - 1]).toMatchObject({
      type: "disconnected",
      reason: "closed by you",
    });
    expect(webStreamList()).toHaveLength(0);
  });

  it("drops a frame over the size limit with a loud error rather than rendering it", () => {
    const { events } = collect({ ...spec, limits: { maxMessageBytes: 4 } });
    FakeSocket.last?.accept();
    FakeSocket.last?.onmessage?.({ data: "far too long" });
    expect(events[2]).toMatchObject({ type: "error", code: "E_STREAM_LIMIT" });
  });
});

describe("an sse connection", () => {
  function streamOf(chunks: string[]): ReadableStream<Uint8Array> {
    return new ReadableStream({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk));
        controller.close();
      },
    });
  }

  const settle = () => new Promise((resolve) => setTimeout(resolve, 10));

  it("parses events off the body and reports the frames", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(streamOf(["event: tick\nid: 7\ndata: now\n\n"]), {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        }),
      ),
    );
    const { events } = collect({ kind: "sse", url: "https://x.dev/e", sse: { autoReconnect: false } });
    await settle();
    expect(events.map((e) => e.type)).toEqual([
      "connecting",
      "connected",
      "message",
      "disconnected",
    ]);
    expect(events[2]).toMatchObject({
      payload: { kind: "text", text: "now" },
      meta: { event: "tick", id: "7", frame: "event" },
    });
  });

  it("refuses an endpoint that is not an event stream", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
      ),
    );
    const { events } = collect({ kind: "sse", url: "https://x.dev/e" });
    await settle();
    expect(events[1]).toMatchObject({ type: "error", code: "E_STREAM_PROTOCOL" });
    expect(events[1].type === "error" && events[1].message).toContain(
      "not text/event-stream",
    );
  });

  it("names the credentials when the server rejects them", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("", { status: 401 })));
    const { events } = collect({ kind: "sse", url: "https://x.dev/e" });
    await settle();
    expect(events[1]).toMatchObject({ type: "error", code: "E_STREAM_AUTH" });
  });

  it("resumes from the last event id it was given", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(streamOf(["data: a\n\n"]), {
        status: 200,
        headers: { "content-type": "text/event-stream" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    collect({
      kind: "sse",
      url: "https://x.dev/e",
      sse: { lastEventId: "42", autoReconnect: false },
    });
    await settle();
    const headers = fetchMock.mock.calls[0][1].headers as Headers;
    expect(headers.get("Last-Event-ID")).toBe("42");
    expect(headers.get("Accept")).toBe("text/event-stream");
  });

  it("has nothing to send", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(streamOf([]), {
          status: 200,
          headers: { "content-type": "text/event-stream" },
        }),
      ),
    );
    const { streamId } = collect({ kind: "sse", url: "https://x.dev/e" });
    await expect(webStreamSend(streamId, { kind: "text", text: "x" })).rejects.toThrow(
      /nothing to send/,
    );
  });
});

describe("payload encoding", () => {
  it("round-trips base64", () => {
    const bytes = new Uint8Array([0, 1, 254, 255]);
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
  });

  it("calls valid utf-8 text and everything else binary", () => {
    expect(payloadOf(new TextEncoder().encode("hi"))).toEqual({ kind: "text", text: "hi" });
    expect(payloadOf(new Uint8Array([0xff]))).toMatchObject({ kind: "binary", bytes: 1 });
  });
});
