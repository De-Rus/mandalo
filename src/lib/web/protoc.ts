import type { GrpcMethodInfo, MessageShape } from "../api";
import type { ProtoFile } from "./protos";

export interface Protoc {
  compile(files: ProtoFile[]): GrpcMethodInfo[];
  /** PENDING: the grpc-wasm crate does not export `describe` yet. */
  describe?(files: ProtoFile[], typeName: string): MessageShape;
  encode(
    files: ProtoFile[],
    service: string,
    method: string,
    json: string,
  ): Uint8Array;
  decode(
    files: ProtoFile[],
    service: string,
    method: string,
    bytes: Uint8Array,
  ): string;
}

let pending: Promise<Protoc> | null = null;

async function load(): Promise<Protoc> {
  const mod = await import("./protoc/protoc.js");
  await mod.default();
  return mod as unknown as Protoc;
}

/**
 * The proto compiler is the same protox/prost-reflect code the desktop runs, built
 * to WebAssembly — a megabyte that only a gRPC request needs, so it is fetched on
 * the first one and never as part of the app bundle.
 */
export function protoc(): Promise<Protoc> {
  if (!pending)
    pending = load().catch((e) => {
      pending = null;
      throw new Error(
        `Could not load the proto compiler (protoc.wasm): ${e instanceof Error ? e.message : String(e)}`,
      );
    });
  return pending;
}
