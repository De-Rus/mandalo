import { describe, expect, it } from "vitest";
import { SseParser, SseTooBig, type SseFrame } from "./sseParser";

const bytes = (text: string) => new TextEncoder().encode(text);

function frames(chunks: string[], max = 1024): SseFrame[] {
  const parser = new SseParser();
  const out: SseFrame[] = [];
  for (const chunk of chunks) out.push(...parser.feed(bytes(chunk), max));
  return out;
}

/**
 * These are the Rust parser's own cases. The browser build reads the same
 * endpoints as the desktop one; a disagreement here is two products.
 */
describe("the browser sse parser matches the engine", () => {
  it("dispatches on a blank line and on nothing else", () => {
    expect(frames(["data: one\n"])).toEqual([]);
    const out = frames(["data: one\n\n"]);
    expect(out).toHaveLength(1);
    expect(out[0].data).toBe("one");
    expect(out[0].event).toBeNull();
  });

  it("joins multi-line data with newlines", () => {
    expect(frames(["data: one\ndata: two\ndata:three\n\n"])[0].data).toBe(
      "one\ntwo\nthree",
    );
  });

  it("reads event, id and comments", () => {
    const out = frames([": keep alive\nevent: tick\nid: 7\ndata: now\n\n"]);
    expect(out[0].event).toBe("tick");
    expect(out[0].id).toBe("7");
    expect(out[0].data).toBe("now");
  });

  it("reads retry without dispatching on it", () => {
    const parser = new SseParser();
    expect(parser.feed(bytes("retry: 2500\n\n"), 1024)).toEqual([]);
    expect(parser.retryMs()).toBe(2500);
  });

  it("parses a field split across chunks", () => {
    expect(frames(["da", "ta: hel", "lo\n", "\n"])[0].data).toBe("hello");
  });

  it("terminates lines on CRLF and on a lone CR", () => {
    expect(frames(["data: a\r\n\r\n"])[0].data).toBe("a");
    expect(frames(["data: b\r\rdata: c"])[0].data).toBe("b");
    expect(frames(["data: c\r", "\n\n"])[0].data).toBe("c");
  });

  it("keeps the last event id for events that carry none", () => {
    const parser = new SseParser();
    parser.feed(bytes("id: 1\ndata: a\n\n"), 1024);
    const out = parser.feed(bytes("data: b\n\n"), 1024);
    expect(parser.lastId()).toBe("1");
    expect(out[0].id).toBe("1");
  });

  it("tolerates a valueless field and an unknown one", () => {
    expect(frames(["data\ndata:\nfoo: bar\ndata: x\n\n"])[0].data).toBe("\n\nx");
  });

  it("strips a leading byte order mark once", () => {
    expect(frames(["﻿data: a\n\n"])[0].data).toBe("a");
  });

  it("fails loud on an event over the limit", () => {
    const parser = new SseParser();
    expect(() =>
      parser.feed(bytes("data: aaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\n"), 8),
    ).toThrow(SseTooBig);
  });

  it("ignores an id that carries a NUL", () => {
    const parser = new SseParser();
    const out = parser.feed(bytes("id: a\0b\ndata: x\n\n"), 1024);
    expect(out[0].id).toBeNull();
    expect(parser.lastId()).toBeNull();
  });

  it("ignores a retry that is not a whole number", () => {
    const parser = new SseParser();
    parser.feed(bytes("retry: soon\n\n"), 1024);
    expect(parser.retryMs()).toBeNull();
  });
});
