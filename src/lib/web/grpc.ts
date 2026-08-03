import type {
  GrpcMethodInfo,
  GrpcResponse,
  GrpcSpec,
  MessageShape,
} from "../api";
import { codeName, encodeFrame, readReply } from "./grpcweb";
import { protoc } from "./protoc";
import { collect } from "./protos";
import { interpolate } from "./send";
import type { Vfs } from "./vfs";

const CONTENT_TYPE = "application/grpc-web+proto";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function callUrl(base: string, service: string, method: string): string {
  if (!base.startsWith("http://") && !base.startsWith("https://"))
    throw new Error(`grpc url must start with http:// or https://: ${base}`);
  return `${base.replace(/\/+$/, "")}/${service}/${method}`;
}

function checkMetadata(pairs: [string, string][]): void {
  for (const [key, value] of pairs) {
    if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(key))
      throw new Error(`invalid metadata key (ascii only): ${key}`);
    if (!/^[\x20-\x7e]*$/.test(value))
      throw new Error(`invalid metadata value for key ${key} (ascii only)`);
  }
}

export async function webListGrpcMethods(
  vfs: Vfs,
  protoPaths: string[],
): Promise<GrpcMethodInfo[]> {
  const files = await collect(vfs, protoPaths);
  return (await protoc()).compile(files);
}

export async function webDescribeMessage(
  vfs: Vfs,
  protoPaths: string[],
  typeName: string,
): Promise<MessageShape> {
  const compiler = await protoc();
  if (!compiler.describe)
    throw new Error(
      "This build of the proto compiler cannot describe a message type yet, so Mándalo has no example message to offer.",
    );
  return compiler.describe(await collect(vfs, protoPaths), typeName);
}

export async function webSendGrpc(
  vfs: Vfs,
  spec: GrpcSpec,
): Promise<GrpcResponse> {
  const vars = spec.vars;
  const base = interpolate(spec.url, vars);
  const message = interpolate(spec.message, vars);
  const metadata: [string, string][] = spec.metadata.map(([k, v]) => [
    k,
    interpolate(v, vars),
  ]);
  checkMetadata(metadata);
  const protoPaths = spec.protoPaths.map((p) => interpolate(p, vars));

  const url = callUrl(base, spec.service, spec.method);
  const files = await collect(vfs, protoPaths);
  const compiler = await protoc();
  const payload = compiler.encode(files, spec.service, spec.method, message);

  const started = performance.now();
  let res: Response;
  try {
    res = await fetch(url, {
      method: "POST",
      headers: [
        ["content-type", CONTENT_TYPE],
        ["accept", CONTENT_TYPE],
        ["x-grpc-web", "1"],
        ...metadata,
      ],
      body: encodeFrame(payload),
      credentials: "omit",
      referrerPolicy: "no-referrer",
      cache: "no-store",
    });
  } catch (e) {
    throw new Error(
      `Could not reach ${hostOf(url)} over gRPC-Web. A page cannot speak plain gRPC, only gRPC-Web over fetch, and that needs the server to (1) run a gRPC-Web layer such as tonic-web or Envoy and (2) answer the CORS preflight allowing the content-type and x-grpc-web request headers and exposing grpc-status and grpc-message. The browser will not say which of those failed. Detail: ${e instanceof Error ? e.message : String(e)}`,
    );
  }

  if (res.status !== 200)
    throw new Error(
      `${hostOf(url)} answered HTTP ${res.status} ${res.statusText} for ${new URL(url).pathname}. A gRPC-Web endpoint answers 200 and puts the real outcome in grpc-status, so this host is not serving gRPC-Web at that path.`,
    );

  const contentType = (res.headers.get("content-type") ?? "").toLowerCase();
  if (contentType !== "" && !contentType.startsWith("application/grpc-web"))
    throw new Error(
      `${hostOf(url)} answered with content-type "${contentType}", not application/grpc-web. Native gRPC (application/grpc) needs HTTP/2 trailers, which a browser will not give a page — run this request in the desktop app, or put a gRPC-Web layer in front of the server.`,
    );

  const headers = new Map<string, string>();
  res.headers.forEach((value, key) => headers.set(key.toLowerCase(), value));
  const bytes = new Uint8Array(await res.arrayBuffer());
  const durationMs = Math.round(performance.now() - started);

  const reply = readReply(bytes, headers);
  if (reply.status !== 0)
    throw new Error(
      `grpc error ${codeName(reply.status)}: ${reply.statusMessage}`,
    );
  if (reply.message === null)
    throw new Error(
      "the server reported grpc-status 0 but sent no message frame",
    );

  return {
    body: compiler.decode(files, spec.service, spec.method, reply.message),
    durationMs,
  };
}
