import type { ResponseData } from "../core/cli";
import type { PreparedRequest } from "./prepare";

const TIMEOUT_MS = 30_000;

export class NetworkError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NetworkError";
  }
}

function readHeaders(response: Response): [string, string][] {
  const out: [string, string][] = [];
  response.headers.forEach((value, key) => {
    if (key.toLowerCase() === "set-cookie") return;
    out.push([key, value]);
  });
  for (const cookie of response.headers.getSetCookie()) out.push(["set-cookie", cookie]);
  return out;
}

export async function send(request: PreparedRequest): Promise<ResponseData> {
  const started = performance.now();
  let response: Response;
  try {
    const bodyless = request.method === "GET" || request.method === "HEAD";
    response = await fetch(request.url, {
      method: request.method,
      headers: request.headers,
      ...(bodyless || request.body === null ? {} : { body: request.body }),
      redirect: "follow",
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
  } catch (error) {
    throw new NetworkError(describe(error));
  }

  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(await response.arrayBuffer());
  } catch (error) {
    throw new NetworkError(describe(error));
  }
  const durationMs = Math.round(performance.now() - started);
  const body = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  const binary = !isUtf8(bytes);

  return {
    status: response.status,
    statusText: response.statusText,
    headers: readHeaders(response),
    body,
    binary,
    durationMs,
    sizeBytes: bytes.byteLength,
  };
}

function isUtf8(bytes: Uint8Array): boolean {
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return true;
  } catch {
    return false;
  }
}

// Node wraps connection failures in layers that can carry an empty message
// (an ECONNREFUSED surfaces as an AggregateError with message "") — dig until
// something names the failure.
function causeDetail(error: unknown): string | null {
  if (!(error instanceof Error)) return null;
  if (error instanceof AggregateError && error.errors.length > 0) {
    return causeDetail(error.errors[0]);
  }
  if (error.message !== "") return error.message;
  const code = (error as NodeJS.ErrnoException).code;
  if (typeof code === "string" && code !== "") return code;
  return causeDetail(error.cause);
}

function describe(error: unknown): string {
  if (error instanceof DOMException && error.name === "TimeoutError") {
    return `request timed out after ${TIMEOUT_MS}ms`;
  }
  const message = error instanceof Error ? error.message : String(error);
  const cause = error instanceof Error ? causeDetail(error.cause) : null;
  return cause === null ? message : `${message}: ${cause}`;
}
