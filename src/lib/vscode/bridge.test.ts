import { describe, expect, it, vi } from "vitest";
import { Bridge, type Channel } from "./bridge";
import { PROTOCOL_VERSION, type CallMessage, type EventMessage } from "./protocol";

function harness() {
  const sent: CallMessage[] = [];
  const channel: Channel = { postMessage: (m) => void sent.push(m as CallMessage) };
  return { sent, bridge: new Bridge(channel) };
}

function ok(id: string, value: unknown) {
  return { v: PROTOCOL_VERSION, type: "reply", id, ok: true, value };
}

function bad(id: string, message: string) {
  return { v: PROTOCOL_VERSION, type: "reply", id, ok: false, error: { message } };
}

describe("Bridge", () => {
  it("sends a versioned call carrying the command and args", async () => {
    const { sent, bridge } = harness();
    const promise = bridge.call<string>("load_document", { a: 1 });
    expect(sent[0]).toMatchObject({
      v: PROTOCOL_VERSION,
      type: "call",
      command: "load_document",
      args: { a: 1 },
    });
    bridge.receive(ok(sent[0]!.id, "done"));
    await expect(promise).resolves.toBe("done");
  });

  it("matches each reply to its own caller, whatever the order", async () => {
    const { sent, bridge } = harness();
    const first = bridge.call<number>("one");
    const second = bridge.call<number>("two");
    expect(sent[0]!.id).not.toBe(sent[1]!.id);
    bridge.receive(ok(sent[1]!.id, 2));
    bridge.receive(ok(sent[0]!.id, 1));
    await expect(Promise.all([first, second])).resolves.toEqual([1, 2]);
  });

  it("rejects with the host's message", async () => {
    const { sent, bridge } = harness();
    const promise = bridge.call("send_request");
    bridge.receive(bad(sent[0]!.id, "connection refused"));
    await expect(promise).rejects.toThrow("connection refused");
  });

  it("still rejects when the host sends an error without a message", async () => {
    const { sent, bridge } = harness();
    const promise = bridge.call("x");
    bridge.receive({ v: PROTOCOL_VERSION, type: "reply", id: sent[0]!.id, ok: false });
    await expect(promise).rejects.toThrow(/extension host/);
  });

  it("forgets a call once it is answered", async () => {
    const { sent, bridge } = harness();
    const promise = bridge.call("x");
    expect(bridge.inFlight).toBe(1);
    bridge.receive(ok(sent[0]!.id, null));
    await promise;
    expect(bridge.inFlight).toBe(0);
  });

  it("ignores a reply for an id it never issued", () => {
    const { bridge } = harness();
    expect(() => bridge.receive(ok("ghost", 1))).not.toThrow();
  });

  it("ignores noise on the channel", () => {
    const { bridge } = harness();
    expect(() => {
      bridge.receive(null);
      bridge.receive("hello");
      bridge.receive({ type: "something-else" });
    }).not.toThrow();
  });

  it("rejects the call when the channel itself throws", async () => {
    const bridge = new Bridge({
      postMessage: () => {
        throw new Error("webview is gone");
      },
    });
    await expect(bridge.call("x")).rejects.toThrow("webview is gone");
    expect(bridge.inFlight).toBe(0);
  });

  it("fans events out to every listener until they unsubscribe", () => {
    const { bridge } = harness();
    const seen: EventMessage[] = [];
    const off = bridge.on((e) => seen.push(e));
    const other = vi.fn();
    bridge.on(other);
    const message: EventMessage = { v: PROTOCOL_VERSION, type: "event", event: "document", payload: { a: 1 } };
    bridge.receive(message);
    off();
    bridge.receive(message);
    expect(seen).toEqual([message]);
    expect(other).toHaveBeenCalledTimes(2);
  });

  it("never resolves a pending call with an event", async () => {
    const { bridge } = harness();
    const settled = vi.fn();
    void bridge.call("x").then(settled, settled);
    bridge.receive({ v: PROTOCOL_VERSION, type: "event", event: "document", payload: null });
    await Promise.resolve();
    expect(settled).not.toHaveBeenCalled();
    expect(bridge.inFlight).toBe(1);
  });
});
