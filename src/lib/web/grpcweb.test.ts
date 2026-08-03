import { describe, expect, it } from "vitest";
import {
  codeName,
  decodeFrames,
  decodeGrpcMessage,
  encodeFrame,
  parseTrailers,
  readReply,
} from "./grpcweb";

const text = (s: string) => new TextEncoder().encode(s);

function frame(payload: Uint8Array, flags = 0): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  out[0] = flags;
  new DataView(out.buffer).setUint32(1, payload.length, false);
  out.set(payload, 5);
  return out;
}

function join(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

describe("encodeFrame", () => {
  it("prefixes the payload with a flag byte and a big-endian length", () => {
    expect([...encodeFrame(new Uint8Array([0x0a, 0x04]))]).toEqual([
      0, 0, 0, 0, 2, 0x0a, 0x04,
    ]);
  });

  it("frames an empty message as a bare header", () => {
    expect([...encodeFrame(new Uint8Array())]).toEqual([0, 0, 0, 0, 0]);
  });

  it("writes lengths above one byte big-endian", () => {
    const long = new Uint8Array(300);
    expect([...encodeFrame(long).slice(0, 5)]).toEqual([0, 0, 0, 1, 44]);
  });
});

describe("decodeFrames", () => {
  it("splits a message frame from the trailer frame", () => {
    const frames = decodeFrames(
      join(frame(text("hi")), frame(text("grpc-status:0\r\n"), 0x80)),
    );
    expect(frames.map((f) => f.trailer)).toEqual([false, true]);
    expect(new TextDecoder().decode(frames[0].payload)).toBe("hi");
  });

  it("reads a trailers-only body", () => {
    const frames = decodeFrames(frame(text("grpc-status:5\r\n"), 0x80));
    expect(frames).toHaveLength(1);
    expect(frames[0].trailer).toBe(true);
  });

  it("reads every frame of a multi-frame body", () => {
    const frames = decodeFrames(
      join(
        frame(text("a")),
        frame(text("b")),
        frame(text("grpc-status:0\r\n"), 0x80),
      ),
    );
    expect(frames).toHaveLength(3);
  });

  it("fails loud on a truncated header", () => {
    expect(() => decodeFrames(new Uint8Array([0, 0, 0]))).toThrow(
      /truncated gRPC-Web response/,
    );
  });

  it("fails loud when the declared length overruns the body", () => {
    expect(() => decodeFrames(new Uint8Array([0, 0, 0, 0, 9, 1, 2]))).toThrow(
      /truncated gRPC-Web frame: header declares 9 bytes but only 2 arrived/,
    );
  });

  it("fails loud on a compressed frame", () => {
    expect(() => decodeFrames(frame(text("x"), 0x01))).toThrow(
      /compressed gRPC-Web frame/,
    );
  });
});

describe("parseTrailers", () => {
  it("reads CRLF-separated header lines, lowercasing names", () => {
    const trailers = parseTrailers("Grpc-Status: 9\r\ngrpc-message: nope\r\n");
    expect(trailers.get("grpc-status")).toBe("9");
    expect(trailers.get("grpc-message")).toBe("nope");
  });

  it("keeps colons inside the value", () => {
    expect(parseTrailers("grpc-message: a: b").get("grpc-message")).toBe("a: b");
  });

  it("fails loud on a line without a colon", () => {
    expect(() => parseTrailers("garbage\r\n")).toThrow(/malformed/);
  });
});

describe("decodeGrpcMessage", () => {
  it("leaves a plain message alone", () => {
    expect(decodeGrpcMessage("the mock refused")).toBe("the mock refused");
  });

  it("decodes percent-escaped UTF-8", () => {
    expect(decodeGrpcMessage("no%20se%20pudo%3A%20%C3%B1")).toBe(
      "no se pudo: ñ",
    );
  });

  it("keeps a stray percent that is not an escape", () => {
    expect(decodeGrpcMessage("100% off")).toBe("100% off");
  });
});

describe("readReply", () => {
  const noHeaders = new Map<string, string>();

  it("returns the message and status 0 for a successful call", () => {
    const reply = readReply(
      join(frame(text("payload")), frame(text("grpc-status:0\r\n"), 0x80)),
      noHeaders,
    );
    expect(reply.status).toBe(0);
    expect(new TextDecoder().decode(reply.message as Uint8Array)).toBe(
      "payload",
    );
  });

  it("surfaces a non-zero grpc-status with its message", () => {
    const reply = readReply(
      frame(text("grpc-status:9\r\ngrpc-message:the mock refused: x\r\n"), 0x80),
      noHeaders,
    );
    expect(reply.status).toBe(9);
    expect(codeName(reply.status)).toBe("FailedPrecondition");
    expect(reply.statusMessage).toBe("the mock refused: x");
    expect(reply.message).toBeNull();
  });

  it("falls back to the HTTP headers when there is no trailer frame", () => {
    const reply = readReply(
      new Uint8Array(),
      new Map([
        ["grpc-status", "12"],
        ["grpc-message", "method not implemented by the mock"],
      ]),
    );
    expect(codeName(reply.status)).toBe("Unimplemented");
    expect(reply.statusMessage).toBe("method not implemented by the mock");
  });

  it("fails loud when nothing carries a grpc-status", () => {
    expect(() => readReply(frame(text("hi")), noHeaders)).toThrow(
      /not answering gRPC-Web/,
    );
  });

  it("fails loud on an unreadable grpc-status", () => {
    expect(() =>
      readReply(frame(text("grpc-status:nope\r\n"), 0x80), noHeaders),
    ).toThrow(/unreadable grpc-status/);
  });

  it("fails loud when a unary call gets a stream of messages", () => {
    expect(() =>
      readReply(
        join(
          frame(text("a")),
          frame(text("b")),
          frame(text("grpc-status:0\r\n"), 0x80),
        ),
        noHeaders,
      ),
    ).toThrow(/streaming methods are not supported yet/);
  });

  it("fails loud on two trailer frames", () => {
    expect(() =>
      readReply(
        join(
          frame(text("grpc-status:0\r\n"), 0x80),
          frame(text("grpc-status:0\r\n"), 0x80),
        ),
        noHeaders,
      ),
    ).toThrow(/2 gRPC-Web trailer frames/);
  });
});

describe("codeName", () => {
  it("names the canonical codes the way the desktop prints them", () => {
    expect(codeName(0)).toBe("Ok");
    expect(codeName(5)).toBe("NotFound");
    expect(codeName(16)).toBe("Unauthenticated");
  });

  it("keeps an unknown number visible instead of guessing", () => {
    expect(codeName(42)).toBe("Code(42)");
  });
});
