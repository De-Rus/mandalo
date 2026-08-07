import { afterEach, describe, expect, it, vi } from "vitest";
import { closeSync, publish, subscribe, syncSupported } from "./sync";

class FakeChannel {
  static open: FakeChannel[] = [];
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;

  constructor(readonly name: string) {
    FakeChannel.open.push(this);
  }

  postMessage(data: unknown): void {
    for (const other of FakeChannel.open)
      if (other !== this && other.name === this.name)
        other.onmessage?.({ data } as MessageEvent<unknown>);
  }

  close(): void {
    FakeChannel.open = FakeChannel.open.filter((c) => c !== this);
  }
}

function install(): void {
  FakeChannel.open = [];
  vi.stubGlobal("BroadcastChannel", FakeChannel);
}

afterEach(() => {
  closeSync();
  vi.unstubAllGlobals();
});

describe("broadcasting what changed", () => {
  it("delivers the changed path to another tab, not just 'something changed'", () => {
    install();
    const other = new FakeChannel("mandalo.workspace");
    const seen: unknown[] = [];
    other.onmessage = (e) => seen.push(e.data);

    publish({
      workspace: "browser://Browser storage",
      scope: "request",
      collection: "mock",
      path: "http/get.http",
    });

    expect(seen).toEqual([
      {
        workspace: "browser://Browser storage",
        scope: "request",
        collection: "mock",
        path: "http/get.http",
      },
    ]);
  });

  it("does not echo a tab's own change back to itself", () => {
    install();
    const seen: unknown[] = [];
    subscribe((change) => seen.push(change));

    publish({ workspace: "w", scope: "tree" });

    expect(seen).toEqual([]);
  });

  it("hands an incoming change to every listener in this tab", () => {
    install();
    const seen: unknown[] = [];
    subscribe((change) => seen.push(change));
    const other = new FakeChannel("mandalo.workspace");

    other.postMessage({ workspace: "w", scope: "tree" });

    expect(seen).toEqual([{ workspace: "w", scope: "tree" }]);
  });

  it("degrades to a no-op where BroadcastChannel is missing", () => {
    vi.stubGlobal("BroadcastChannel", undefined);

    expect(syncSupported()).toBe(false);
    expect(() => publish({ workspace: "w", scope: "tree" })).not.toThrow();
    expect(() => subscribe(() => {})()).not.toThrow();
  });
});
