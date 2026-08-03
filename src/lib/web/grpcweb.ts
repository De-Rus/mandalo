export const GRPC_CODE: Record<number, string> = {
  0: "Ok",
  1: "Cancelled",
  2: "Unknown",
  3: "InvalidArgument",
  4: "DeadlineExceeded",
  5: "NotFound",
  6: "AlreadyExists",
  7: "PermissionDenied",
  8: "ResourceExhausted",
  9: "FailedPrecondition",
  10: "Aborted",
  11: "OutOfRange",
  12: "Unimplemented",
  13: "Internal",
  14: "Unavailable",
  15: "DataLoss",
  16: "Unauthenticated",
};

export function codeName(status: number): string {
  return GRPC_CODE[status] ?? `Code(${status})`;
}

const COMPRESSED = 0x01;
const TRAILER = 0x80;

export function encodeFrame(payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  new DataView(out.buffer).setUint32(1, payload.length, false);
  out.set(payload, 5);
  return out;
}

export interface Frame {
  trailer: boolean;
  payload: Uint8Array;
}

export function decodeFrames(bytes: Uint8Array): Frame[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const frames: Frame[] = [];
  let at = 0;
  while (at < bytes.length) {
    if (bytes.length - at < 5)
      throw new Error(
        `truncated gRPC-Web response: ${bytes.length - at} trailing byte(s) are not a 5-byte frame header`,
      );
    const flags = bytes[at];
    const length = view.getUint32(at + 1, false);
    if (flags & COMPRESSED)
      throw new Error(
        "the server sent a compressed gRPC-Web frame; Mándalo does not support per-message compression",
      );
    const end = at + 5 + length;
    if (end > bytes.length)
      throw new Error(
        `truncated gRPC-Web frame: header declares ${length} bytes but only ${bytes.length - at - 5} arrived`,
      );
    frames.push({
      trailer: (flags & TRAILER) !== 0,
      payload: bytes.subarray(at + 5, end),
    });
    at = end;
  }
  return frames;
}

export function parseTrailers(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const line of text.split(/\r\n|\n/)) {
    if (line.trim() === "") continue;
    const at = line.indexOf(":");
    if (at === -1)
      throw new Error(`malformed gRPC-Web trailer line: ${line.trim()}`);
    out.set(line.slice(0, at).trim().toLowerCase(), line.slice(at + 1).trim());
  }
  return out;
}

/**
 * gRPC percent-encodes `grpc-message` because trailers are ASCII-only, and it
 * escapes every byte outside 0x20..0x7e — so the decode has to be UTF-8 aware
 * rather than a per-character replace.
 */
export function decodeGrpcMessage(raw: string): string {
  if (!raw.includes("%")) return raw;
  const bytes: number[] = [];
  for (let i = 0; i < raw.length; i += 1) {
    if (raw[i] === "%" && i + 2 < raw.length) {
      const hex = raw.slice(i + 1, i + 3);
      if (/^[0-9a-fA-F]{2}$/.test(hex)) {
        bytes.push(parseInt(hex, 16));
        i += 2;
        continue;
      }
    }
    for (const byte of new TextEncoder().encode(raw[i])) bytes.push(byte);
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

export interface GrpcWebReply {
  message: Uint8Array | null;
  status: number;
  statusMessage: string;
}

/**
 * A gRPC-Web reply carries its status either in a trailer frame at the end of the
 * body or, for a trailers-only reply, in the HTTP headers. Both are normal; a reply
 * with neither is a server that is not speaking gRPC-Web.
 */
export function readReply(
  bytes: Uint8Array,
  headers: Map<string, string>,
): GrpcWebReply {
  const frames = decodeFrames(bytes);
  const messages = frames.filter((f) => !f.trailer);
  const trailerFrames = frames.filter((f) => f.trailer);
  if (trailerFrames.length > 1)
    throw new Error(
      `the server sent ${trailerFrames.length} gRPC-Web trailer frames, expected one`,
    );

  let trailers = new Map<string, string>();
  if (trailerFrames.length === 1)
    trailers = parseTrailers(new TextDecoder().decode(trailerFrames[0].payload));
  else if (headers.has("grpc-status")) trailers = headers;
  else
    throw new Error(
      "the response carried no grpc-status, in a trailer frame or a header: this server is not answering gRPC-Web",
    );

  const raw = trailers.get("grpc-status") ?? "";
  const status = Number(raw);
  if (raw === "" || !Number.isInteger(status))
    throw new Error(`the server sent an unreadable grpc-status: ${raw || "(empty)"}`);

  if (messages.length > 1)
    throw new Error(
      `the server sent ${messages.length} messages for a unary call; streaming methods are not supported yet`,
    );

  return {
    message: messages.length === 1 ? messages[0].payload : null,
    status,
    statusMessage: decodeGrpcMessage(trailers.get("grpc-message") ?? ""),
  };
}
