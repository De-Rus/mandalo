export type Host = "desktop" | "editor" | "browser";

export function currentHost(): Host {
  if (typeof window === "undefined") return "desktop";
  const global = window as unknown as Record<string, unknown>;
  if ("__TAURI_INTERNALS__" in global) return "desktop";
  if (typeof global.acquireVsCodeApi === "function") return "editor";
  return "browser";
}

/** Only a real HTTP/2 client can speak gRPC; a page has fetch and gRPC-Web. */
export function hasNativeGrpc(): boolean {
  return currentHost() !== "browser";
}

const LOOPBACK = new Set([
  "localhost",
  "127.0.0.1",
  "0.0.0.0",
  "::1",
  "[::1]",
]);

export function isLoopback(url: string): boolean {
  const trimmed = url.trim();
  if (trimmed === "") return false;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
    ? trimmed
    : `http://${trimmed}`;
  try {
    return LOOPBACK.has(new URL(withScheme).hostname.toLowerCase());
  } catch {
    return false;
  }
}
