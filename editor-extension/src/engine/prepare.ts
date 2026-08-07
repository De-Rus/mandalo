import type { Auth, RequestModel } from "../../../src/lib/format/model";
import { apply, applyJson, canonicalJson } from "./interpolate";
import type { ScriptRequest } from "./sandbox";

export class UnsupportedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "UnsupportedError";
  }
}

export class RequestBuildError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RequestBuildError";
  }
}

export interface PreparedRequest {
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
}

const METHODS = new Set([
  "GET",
  "HEAD",
  "POST",
  "PUT",
  "DELETE",
  "CONNECT",
  "OPTIONS",
  "TRACE",
  "PATCH",
]);

function base64(text: string): string {
  return Buffer.from(text, "utf8").toString("base64");
}

// reqwest builds query strings with serde_urlencoded, which encodes a space as `+`
// where encodeURIComponent emits `%20`. Same target, different bytes on the wire.
function formEncode(value: string): string {
  return encodeURIComponent(value).replace(/%20/g, "+").replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
}

/**
 * A body written as bare text carries no language, so the Rust engine sniffs one when
 * it loads the request and labels it with that language's content type. The twin is
 * `RawLanguage::sniff` in crates/core/src/body.rs — without it undici would default
 * every raw body to `text/plain;charset=UTF-8`.
 */
export function sniffContentType(text: string): string {
  const trimmed = text.replace(/^\s+/, "");
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) return "application/json";
  if (trimmed.startsWith("<?xml")) return "application/xml";
  const lower = trimmed.toLowerCase();
  if (lower.startsWith("<!doctype html") || lower.startsWith("<html")) return "text/html";
  if (trimmed.startsWith("<")) return "application/xml";
  return "text/plain";
}

export function prepare(
  model: RequestModel,
  wire: ScriptRequest,
  vars: Record<string, string>,
): PreparedRequest {
  const url = apply(wire.url, vars);
  let headers: [string, string][] = wire.headers.map(([k, v]) => [apply(k, vars), apply(v, vars)]);

  let method: string;
  let body: string | null;
  let contentType: string | null = null;

  if (model.kind === "http") {
    method = wire.method.toUpperCase();
    if (!METHODS.has(method)) throw new RequestBuildError(`invalid method: ${wire.method}`);
    const raw = wire.body === null || wire.body === "" ? null : wire.body;
    body = raw === null ? null : apply(raw, vars);
    contentType = raw === null ? null : sniffContentType(raw);
  } else if (model.kind === "graphql") {
    if (!model.graphql) {
      throw new RequestBuildError("graphql request is missing the graphql body");
    }
    const query = apply(model.graphql.query, vars);
    const raw = model.graphql.variables.trim();
    let variables: unknown = {};
    if (raw !== "") {
      let parsed: unknown;
      try {
        parsed = JSON.parse(raw);
      } catch (error) {
        throw new RequestBuildError(`invalid graphql variables JSON: ${(error as Error).message}`);
      }
      variables = applyJson(parsed, vars);
    }
    method = "POST";
    body = canonicalJson({ query, variables });
    contentType = "application/json";
  } else {
    throw new UnsupportedError(`unsupported request kind: ${model.kind}`);
  }

  const auth: Auth = model.auth;
  if (auth.type === "bearer" || auth.type === "basic") {
    headers = headers.filter(([k]) => k.toLowerCase() !== "authorization");
  }
  const userSetContentType = headers.some(([k]) => k.toLowerCase() === "content-type");
  if (contentType !== null && !userSetContentType) headers = [...headers, ["Content-Type", contentType]];

  let finalUrl = url;
  switch (auth.type) {
    case "bearer":
      headers = [...headers, ["Authorization", `Bearer ${apply(auth.token, vars)}`]];
      break;
    case "basic": {
      const user = apply(auth.username, vars);
      const pass = apply(auth.password, vars);
      headers = [...headers, ["Authorization", `Basic ${base64(`${user}:${pass}`)}`]];
      break;
    }
    case "apikey": {
      const key = apply(auth.key, vars);
      const value = apply(auth.value, vars);
      if (auth.placement === "header") headers = [...headers, [key, value]];
      else if (auth.placement === "query") {
        const sep = finalUrl.includes("?") ? "&" : "?";
        finalUrl = `${finalUrl}${sep}${formEncode(key)}=${formEncode(value)}`;
      } else {
        throw new UnsupportedError(`unsupported apikey placement: ${auth.placement}`);
      }
      break;
    }
    default:
      break;
  }

  return { method, url: finalUrl, headers, body };
}
