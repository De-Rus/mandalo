import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GrpcSpec } from "../api";
import { MemoryVfs } from "./vfs.testkit";

const encode = vi.fn();
const decode = vi.fn();
const compile = vi.fn();

vi.mock("./protoc", () => ({
  protoc: () => Promise.resolve({ compile, encode, decode }),
}));

const { callUrl, webListGrpcMethods, webSendGrpc } = await import("./grpc");

const PROTO = `syntax = "proto3";
package mock.v1;
message EchoRequest { string text = 1; }
message EchoResponse { string text = 1; }
service Mock { rpc Say(EchoRequest) returns (EchoResponse); }
`;

function text(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

function frame(payload: Uint8Array, flags = 0): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  out[0] = flags;
  new DataView(out.buffer).setUint32(1, payload.length, false);
  out.set(payload, 5);
  return out;
}

function body(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

function reply(bytes: Uint8Array, init: ResponseInit = {}): Response {
  return new Response(bytes, {
    status: 200,
    headers: { "content-type": "application/grpc-web+proto" },
    ...init,
  });
}

function spec(over: Partial<GrpcSpec> = {}): GrpcSpec {
  return {
    url: "http://localhost:50051",
    protoPaths: ["/tmp/mandalo-mock/mock.proto"],
    service: "mock.v1.Mock",
    method: "Say",
    message: '{"text": "hola"}',
    metadata: [],
    vars: {},
    ...over,
  };
}

let vfs: MemoryVfs;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(async () => {
  vfs = new MemoryVfs();
  await vfs.write("protos/mock.proto", PROTO);
  encode.mockReset().mockReturnValue(new Uint8Array([0x0a, 0x04]));
  decode.mockReset().mockReturnValue('{\n  "text": "hola"\n}');
  compile.mockReset().mockReturnValue([]);
  fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("callUrl", () => {
  it("appends the fully qualified method path", () => {
    expect(callUrl("http://localhost:50051", "mock.v1.Mock", "Say")).toBe(
      "http://localhost:50051/mock.v1.Mock/Say",
    );
  });

  it("does not double the slash on a base that ends with one", () => {
    expect(callUrl("https://api.dev/grpc/", "mock.v1.Mock", "Say")).toBe(
      "https://api.dev/grpc/mock.v1.Mock/Say",
    );
  });

  it("fails loud on a scheme fetch cannot use", () => {
    expect(() => callUrl("grpc://localhost:50051", "S", "M")).toThrow(
      "grpc url must start with http:// or https://: grpc://localhost:50051",
    );
  });
});

describe("webListGrpcMethods", () => {
  it("compiles the resolved proto files", async () => {
    compile.mockReturnValue([{ service: "mock.v1.Mock", method: "Say" }]);
    const listed = await webListGrpcMethods(vfs, [
      "/tmp/mandalo-mock/mock.proto",
    ]);
    expect(listed).toHaveLength(1);
    expect(compile).toHaveBeenCalledWith([
      { path: "protos/mock.proto", contents: PROTO },
    ]);
  });

  it("fails loud, and says where it looked, when the proto is not in the workspace", async () => {
    await expect(webListGrpcMethods(vfs, ["/etc/other.proto"])).rejects.toThrow(
      /Could not find the proto file "\/etc\/other.proto".*protos\/other.proto/s,
    );
  });
});

describe("webSendGrpc", () => {
  it("posts a framed gRPC-Web request and decodes the reply", async () => {
    fetchMock.mockResolvedValue(
      reply(body(frame(text("payload")), frame(text("grpc-status:0\r\n"), 0x80))),
    );

    const out = await webSendGrpc(vfs, spec({ metadata: [["x-trace", "t"]] }));

    expect(out.body).toBe('{\n  "text": "hola"\n}');
    expect(out.durationMs).toBeGreaterThanOrEqual(0);

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://localhost:50051/mock.v1.Mock/Say");
    expect(init.method).toBe("POST");
    expect(init.headers).toEqual([
      ["content-type", "application/grpc-web+proto"],
      ["accept", "application/grpc-web+proto"],
      ["x-grpc-web", "1"],
      ["x-trace", "t"],
    ]);
    expect([...(init.body as Uint8Array)]).toEqual([0, 0, 0, 0, 2, 0x0a, 0x04]);
  });

  it("interpolates the url, message, metadata and proto path", async () => {
    fetchMock.mockResolvedValue(
      reply(body(frame(text("p")), frame(text("grpc-status:0\r\n"), 0x80))),
    );
    await webSendGrpc(
      vfs,
      spec({
        url: "{{grpcUrl}}",
        message: '{"text": "{{who}}"}',
        metadata: [["x-trace", "{{who}}"]],
        protoPaths: ["{{protoPath}}"],
        vars: {
          grpcUrl: "http://localhost:50051",
          who: "ada",
          protoPath: "/tmp/mandalo-mock/mock.proto",
        },
      }),
    );
    expect(encode).toHaveBeenCalledWith(
      [{ path: "protos/mock.proto", contents: PROTO }],
      "mock.v1.Mock",
      "Say",
      '{"text": "ada"}',
    );
    expect(fetchMock.mock.calls[0][1].headers).toContainEqual([
      "x-trace",
      "ada",
    ]);
  });

  it("fails loud on an unresolved variable", async () => {
    await expect(
      webSendGrpc(vfs, spec({ url: "{{nope}}" })),
    ).rejects.toThrow("unresolved variable: nope");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("surfaces a non-zero grpc-status the way the desktop does", async () => {
    fetchMock.mockResolvedValue(
      reply(
        frame(
          text("grpc-status:9\r\ngrpc-message:the mock refused: x\r\n"),
          0x80,
        ),
      ),
    );
    await expect(webSendGrpc(vfs, spec({ method: "Fail" }))).rejects.toThrow(
      "grpc error FailedPrecondition: the mock refused: x",
    );
  });

  it("explains gRPC-Web and CORS when the browser drops the request", async () => {
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"));
    await expect(webSendGrpc(vfs, spec())).rejects.toThrow(
      /Could not reach localhost:50051 over gRPC-Web.*tonic-web or Envoy.*x-grpc-web.*Failed to fetch/s,
    );
  });

  it("says the host is not serving gRPC-Web when it answers a non-200", async () => {
    fetchMock.mockResolvedValue(
      new Response(null, { status: 404, statusText: "Not Found" }),
    );
    await expect(webSendGrpc(vfs, spec())).rejects.toThrow(
      /answered HTTP 404 Not Found for \/mock.v1.Mock\/Say.*not serving gRPC-Web/s,
    );
  });

  it("points at the desktop app when the server answers native gRPC", async () => {
    fetchMock.mockResolvedValue(
      reply(new Uint8Array(), {
        headers: { "content-type": "application/grpc" },
      }),
    );
    await expect(webSendGrpc(vfs, spec())).rejects.toThrow(
      /content-type "application\/grpc", not application\/grpc-web.*HTTP\/2 trailers/s,
    );
  });

  it("fails loud when a streaming method is asked for", async () => {
    encode.mockImplementation(() => {
      throw new Error("streaming methods not supported yet");
    });
    await expect(webSendGrpc(vfs, spec({ method: "Ticks" }))).rejects.toThrow(
      "streaming methods not supported yet",
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects a non-ascii metadata value without echoing it", async () => {
    const err = await webSendGrpc(
      vfs,
      spec({ metadata: [["x-secret", "top\u007fsecret"]] }),
    ).catch((e: Error) => e.message);
    expect(err).toBe("invalid metadata value for key x-secret (ascii only)");
  });

  it("rejects a metadata key that is not a header token", async () => {
    await expect(
      webSendGrpc(vfs, spec({ metadata: [["bad key", "v"]] })),
    ).rejects.toThrow("invalid metadata key (ascii only): bad key");
  });

  it("fails loud when the reply claims success but carries no message", async () => {
    fetchMock.mockResolvedValue(
      reply(frame(text("grpc-status:0\r\n"), 0x80)),
    );
    await expect(webSendGrpc(vfs, spec())).rejects.toThrow(
      "the server reported grpc-status 0 but sent no message frame",
    );
  });
});
