import { beforeEach, describe, expect, it, vi } from "vitest";
import { newDraft, type RequestDraft } from "../lib/draft";
import type { StreamEvent } from "../lib/stream";
import { useStreams } from "./stream";

const invoke = vi.hoisted(() => vi.fn());
const channels = vi.hoisted(() => [] as { onmessage: ((e: unknown) => void) | null }[]);

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: ((e: unknown) => void) | null = null;
    constructor() {
      channels.push(this);
    }
  },
}));

const AT = 1_764_000_000_000;

function draftOf(patch: Partial<RequestDraft> = {}): RequestDraft {
  return { ...newDraft("Socket", "websocket"), url: "wss://x.dev/ws", ...patch };
}

function feed(event: StreamEvent): void {
  channels[channels.length - 1]?.onmessage?.(event);
}

const frames = () => new Promise((resolve) => setTimeout(resolve, 30));

beforeEach(() => {
  invoke.mockReset();
  channels.length = 0;
  useStreams.setState({ sessions: {} });
});

describe("connecting", () => {
  it("opens a stream and remembers its id", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    expect(invoke).toHaveBeenCalledWith(
      "stream_open",
      expect.objectContaining({
        spec: expect.objectContaining({ kind: "websocket", url: "wss://x.dev/ws" }),
      }),
    );
    expect(useStreams.getState().sessions[draft.id].streamId).toBe("s1");
  });

  it("will not open a second connection for the same request", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    await useStreams.getState().connect(draft, {});
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("leaves the template alone — the engine resolves it, as it does for a request", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf({ url: "wss://{{host}}/ws" });
    await useStreams.getState().connect(draft, { host: "x.dev" });
    const spec = invoke.mock.calls[0][1].spec;
    expect(spec.url).toBe("wss://{{host}}/ws");
    expect(spec.vars).toEqual({ host: "x.dev" });
  });

  it("turns a refusal from the engine into an error and a close", async () => {
    invoke.mockRejectedValue("wss://x.dev is blocked: host denied");
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    const session = useStreams.getState().sessions[draft.id];
    expect(session.phase).toBe("closed");
    expect(session.rows.map((r) => r.type)).toEqual(["error", "disconnected"]);
  });
});

describe("the event stream drives the session", () => {
  async function open(): Promise<RequestDraft> {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    return draft;
  }

  it("walks connecting → connected → reconnecting → disconnected", async () => {
    const draft = await open();
    const phases: string[] = [];
    feed({ type: "connecting", at: AT, url: "wss://x.dev/ws" });
    await frames();
    phases.push(useStreams.getState().sessions[draft.id].phase);
    feed({ type: "connected", at: AT, info: { url: "wss://x.dev/ws", protocol: "v2" } });
    await frames();
    phases.push(useStreams.getState().sessions[draft.id].phase);
    feed({ type: "reconnecting", at: AT, attempt: 2, delayMs: 1000, reason: "dropped" });
    await frames();
    phases.push(useStreams.getState().sessions[draft.id].phase);
    feed({ type: "disconnected", at: AT, code: 1006, reason: "gone" });
    await frames();
    phases.push(useStreams.getState().sessions[draft.id].phase);
    expect(phases).toEqual(["connecting", "connected", "reconnecting", "closed"]);

    const session = useStreams.getState().sessions[draft.id];
    expect(session.info).toBeNull();
    expect(session.streamId).toBeNull();
    expect(session.rows).toHaveLength(4);
  });

  it("remembers the negotiated subprotocol and when it came up", async () => {
    const draft = await open();
    feed({
      type: "connected",
      at: AT,
      info: { url: "wss://x.dev/ws", protocol: "chat", sessionPresent: true },
    });
    await frames();
    const session = useStreams.getState().sessions[draft.id];
    expect(session.info?.protocol).toBe("chat");
    expect(session.info?.sessionPresent).toBe(true);
    expect(session.connectedAt).toBe(AT);
  });

  it("tracks the subscriptions the connection actually made", async () => {
    const draft = await open();
    feed({ type: "connected", at: AT, info: { url: "mqtt://x" } });
    feed({
      type: "message",
      at: AT,
      direction: "outgoing",
      payload: { kind: "text", text: "subscribe a/#" },
      meta: { topic: "a/#", frame: "subscribe" },
    });
    await frames();
    expect(useStreams.getState().sessions[draft.id].subscriptions).toEqual(["a/#"]);
    feed({
      type: "message",
      at: AT,
      direction: "outgoing",
      payload: { kind: "text", text: "unsubscribe a/#" },
      meta: { topic: "a/#", frame: "unsubscribe" },
    });
    await frames();
    expect(useStreams.getState().sessions[draft.id].subscriptions).toEqual([]);
  });

  it("caps the rows it keeps and says how many it let go", async () => {
    const draft = await open();
    for (let i = 0; i < 2100; i += 1)
      feed({
        type: "message",
        at: AT,
        direction: "incoming",
        payload: { kind: "text", text: `m${i}` },
        meta: {},
      });
    await frames();
    const session = useStreams.getState().sessions[draft.id];
    expect(session.rows).toHaveLength(2000);
    expect(session.overflow).toBe(100);
    expect(session.total).toBe(2100);
  });
});

describe("closing", () => {
  it("closes the engine stream and reflects it", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    await useStreams.getState().disconnect(draft.id);
    expect(invoke).toHaveBeenCalledWith("stream_close", { streamId: "s1" });
    expect(useStreams.getState().sessions[draft.id].phase).toBe("closed");
  });

  it("closes a connection that never got an id", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    const opening = useStreams.getState().connect(draft, {});
    await useStreams.getState().disconnect(draft.id);
    await opening;
    const session = useStreams.getState().sessions[draft.id];
    expect(session.phase).toBe("closed");
    expect(invoke).toHaveBeenCalledWith("stream_close", { streamId: "s1" });
  });

  it("forgets the session entirely when the request goes away", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    await useStreams.getState().forget(draft.id);
    expect(invoke).toHaveBeenCalledWith("stream_close", { streamId: "s1" });
    expect(useStreams.getState().sessions[draft.id]).toBeUndefined();
  });

  it("closes every stream at once", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const a = draftOf();
    const b = draftOf({ id: "second" });
    await useStreams.getState().connect(a, {});
    await useStreams.getState().connect(b, {});
    await useStreams.getState().closeAll();
    expect(
      Object.values(useStreams.getState().sessions).every((s) => s.phase === "closed"),
    ).toBe(true);
  });

  it("toggles between connected and closed", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().toggle(draft, {});
    expect(useStreams.getState().sessions[draft.id].phase).toBe("opening");
    await useStreams.getState().toggle(draft, {});
    expect(useStreams.getState().sessions[draft.id].phase).toBe("closed");
  });
});

describe("sending", () => {
  it("sends on the open stream", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    invoke.mockResolvedValue(undefined);
    await useStreams.getState().send(draft.id, { kind: "text", text: "hi" });
    expect(invoke).toHaveBeenCalledWith("stream_send", {
      streamId: "s1",
      payload: { kind: "text", text: "hi" },
    });
  });

  it("turns a refused send into an error row and keeps the connection", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf();
    await useStreams.getState().connect(draft, {});
    feed({ type: "connected", at: AT, info: { url: "wss://x.dev/ws" } });
    await frames();
    invoke.mockRejectedValue("the message is too big");
    await useStreams.getState().send(draft.id, { kind: "text", text: "hi" });
    const session = useStreams.getState().sessions[draft.id];
    expect(session.phase).toBe("connected");
    expect(session.rows[session.rows.length - 1].summary).toBe("the message is too big");
  });
});

describe("the spec a connect builds", () => {
  it("carries the protocol options the draft holds", async () => {
    invoke.mockResolvedValue({ streamId: "s1" });
    const draft = draftOf({
      kind: "mqtt",
      url: "mqtt://{{broker}}:1883",
      stream: {
        ...newDraft().stream,
        clientId: "probe",
        cleanSession: false,
        keepAliveSecs: "30",
        subscriptions: [{ id: "1", topic: "sensors/#", qos: 1 }],
      },
    });
    await useStreams.getState().connect(draft, { broker: "localhost" });
    const spec = invoke.mock.calls[0][1].spec;
    expect(spec.url).toBe("mqtt://{{broker}}:1883");
    expect(spec.vars).toEqual({ broker: "localhost" });
    expect(spec.mqtt).toEqual({
      cleanSession: false,
      protocolVersion: "3.1.1",
      clientId: "probe",
      keepAliveSecs: 30,
      subscriptions: [{ topic: "sensors/#", qos: 1 }],
    });
  });
});
