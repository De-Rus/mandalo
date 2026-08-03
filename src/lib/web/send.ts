import type { RequestSpec, ResponseData } from "../api";
import { report } from "./bus";
import { dynamicNames, resolveDynamic } from "./dynamic";

/**
 * Response headers a page can read without the server opting in via
 * Access-Control-Expose-Headers. Everything else is hidden by the browser,
 * not by us.
 */
const SAFELISTED = [
  "cache-control",
  "content-language",
  "content-length",
  "content-type",
  "expires",
  "last-modified",
  "pragma",
];

export function interpolate(
  template: string,
  vars: Record<string, string>,
): string {
  let out = "";
  let rest = template;
  for (;;) {
    const start = rest.indexOf("{{");
    if (start === -1) break;
    out += rest.slice(0, start);
    const after = rest.slice(start + 2);
    const end = after.indexOf("}}");
    if (end === -1) throw new Error(`unclosed {{ in: ${template}`);
    const key = after.slice(0, end).trim();
    if (!(key in vars)) throw new Error(`unresolved variable: ${key}`);
    out += vars[key];
    rest = after.slice(end + 2);
  }
  return out + rest;
}

function pairs(
  list: [string, string][],
  vars: Record<string, string>,
): [string, string][] {
  return list.map(([k, v]) => [interpolate(k, vars), interpolate(v, vars)]);
}

function hasHeader(headers: [string, string][], name: string): boolean {
  return headers.some(([k]) => k.toLowerCase() === name.toLowerCase());
}

function interpolateJson(value: unknown, vars: Record<string, string>): unknown {
  if (typeof value === "string") return interpolate(value, vars);
  if (Array.isArray(value)) return value.map((item) => interpolateJson(item, vars));
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value))
      out[interpolate(key, vars)] = interpolateJson(item, vars);
    return out;
  }
  return value;
}

// serde_json's map is a BTreeMap, so every object the Rust engine serialises
// comes out with its keys sorted. GraphQL bodies go on the wire byte for byte.
function canonicalJson(value: unknown): string {
  return JSON.stringify(sortKeys(value));
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value as Record<string, unknown>).sort())
      out[key] = sortKeys((value as Record<string, unknown>)[key]);
    return out;
  }
  return value;
}

/**
 * A body saved as bare text carries no language, so the Rust engine sniffs one
 * when it loads the request and labels the request with that language's type.
 * The browser reads the same file and has to reach the same verdict, or the
 * same request means two different things on the wire.
 */
function sniffContentType(text: string): string {
  const trimmed = text.replace(/^\s+/, "");
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) return "application/json";
  if (trimmed.startsWith("<?xml")) return "application/xml";
  const lower = trimmed.toLowerCase();
  if (lower.startsWith("<!doctype html") || lower.startsWith("<html")) return "text/html";
  if (trimmed.startsWith("<")) return "application/xml";
  return "text/plain";
}

const METHOD = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;

// reqwest builds query strings with serde_urlencoded, which encodes a space as
// `+` where encodeURIComponent emits `%20`.
function formEncode(value: string): string {
  return encodeURIComponent(value)
    .replace(/%20/g, "+")
    .replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export interface Prepared {
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
}

export function prepare(spec: RequestSpec): Prepared {
  const vars = spec.vars;
  let url = interpolate(spec.url, vars);
  let headers = pairs(spec.headers, vars);
  let method = spec.method.toUpperCase();
  let body: string | null;
  let contentType: string | null;

  if (spec.kind === "graphql" && !spec.graphql)
    throw new Error("graphql request is missing the graphql body");

  if (spec.graphql) {
    const rawVars = spec.graphql.variables.trim();
    let variables: unknown = {};
    if (rawVars !== "") {
      let parsed: unknown;
      try {
        parsed = JSON.parse(rawVars);
      } catch (e) {
        throw new Error(
          `invalid graphql variables JSON: ${e instanceof Error ? e.message : String(e)}`,
        );
      }
      variables = interpolateJson(parsed, vars);
    }
    method = "POST";
    body = canonicalJson({
      query: interpolate(spec.graphql.query, vars),
      variables,
    });
    contentType = "application/json";
  } else {
    if (!METHOD.test(method)) throw new Error(`invalid method: ${spec.method}`);
    const raw = spec.body === null || spec.body === "" ? null : spec.body;
    body = raw === null ? null : interpolate(raw, vars);
    contentType = raw === null ? null : sniffContentType(raw);
  }

  if (spec.auth.type === "bearer" || spec.auth.type === "basic")
    headers = headers.filter(([k]) => k.toLowerCase() !== "authorization");

  if (contentType !== null && !hasHeader(headers, "content-type"))
    headers = [...headers, ["Content-Type", contentType]];

  switch (spec.auth.type) {
    case "bearer":
      headers = [
        ...headers,
        ["Authorization", `Bearer ${interpolate(spec.auth.token, vars)}`],
      ];
      break;
    case "basic": {
      const user = interpolate(spec.auth.username, vars);
      const pass = interpolate(spec.auth.password, vars);
      headers = [...headers, ["Authorization", `Basic ${btoa(`${user}:${pass}`)}`]];
      break;
    }
    case "apikey": {
      const key = interpolate(spec.auth.key, vars);
      const value = interpolate(spec.auth.value, vars);
      if (spec.auth.placement === "header") headers = [...headers, [key, value]];
      else if (spec.auth.placement === "query") {
        const sep = url.includes("?") ? "&" : "?";
        url = `${url}${sep}${formEncode(key)}=${formEncode(value)}`;
      } else
        throw new Error(`unsupported apikey placement: ${spec.auth.placement}`);
      break;
    }
    default:
      break;
  }

  return { method, url, headers, body };
}

/** `prepare` after the `{{$…}}` generators the Rust prelude owns have run. */
export async function prepareRequest(spec: RequestSpec): Promise<Prepared> {
  const names = dynamicNames(spec);
  if (names.length === 0) return prepare(spec);
  const generated = await resolveDynamic(names);
  return prepare({ ...spec, vars: { ...generated, ...spec.vars } });
}

/**
 * A request that never reaches the response phase is reported to JS as an
 * opaque TypeError with no reason attached — the CORS decision is deliberately
 * not exposed to script. So we say both things that it can be, rather than
 * guessing one and being confidently wrong.
 */
function preflightIsNeeded(req: Prepared): boolean {
  const simple = ["GET", "HEAD", "POST"];
  if (!simple.includes(req.method.toUpperCase())) return true;
  return req.headers.some(([key, value]) => {
    const k = key.toLowerCase();
    if (k === "accept" || k === "accept-language" || k === "content-language")
      return false;
    if (k === "content-type")
      return ![
        "application/x-www-form-urlencoded",
        "multipart/form-data",
        "text/plain",
      ].includes(value.split(";")[0].trim().toLowerCase());
    return true;
  });
}

/**
 * CORS only governs a cross-origin call the page actually attempted. A rejected
 * fetch to our own origin, or to something that is not an http(s) URL at all,
 * failed for an ordinary reason, and blaming CORS there sends the user hunting
 * for a server header that was never involved.
 */
function crossOrigin(url: string): boolean {
  try {
    const parsed = new URL(url, location.href);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
    return parsed.origin !== location.origin;
  } catch {
    return false;
  }
}

export async function webSend(spec: RequestSpec): Promise<ResponseData> {
  const req = await prepareRequest(spec);
  const started = performance.now();
  let res: Response;
  try {
    res = await fetch(req.url, {
      method: req.method,
      headers: req.headers,
      body:
        req.method === "GET" || req.method === "HEAD"
          ? undefined
          : (req.body ?? undefined),
      redirect: "follow",
      credentials: "omit",
      referrerPolicy: "no-referrer",
      cache: "no-store",
    });
  } catch (e) {
    const cors = crossOrigin(req.url);
    report({
      kind: "unreachable",
      host: hostOf(req.url),
      url: req.url,
      cors,
      preflight: cors && preflightIsNeeded(req),
      detail: e instanceof Error ? e.message : String(e),
    });
    throw new Error(
      cors
        ? `Could not reach ${hostOf(req.url)}. The browser refused or dropped the request without telling scripts why — see the panel for the two things this can mean.`
        : `Could not reach ${hostOf(req.url)}. The request never left the browser, and CORS is not the reason — see the panel.`,
    );
  }

  const text = await res.text();
  const durationMs = Math.round(performance.now() - started);
  const headers: [string, string][] = [];
  res.headers.forEach((value, key) => headers.push([key, value]));
  const exposed = headers.some(([k]) => !SAFELISTED.includes(k.toLowerCase()));
  report({
    kind: "sent",
    host: hostOf(req.url),
    url: req.url,
    headerCount: headers.length,
    exposed,
  });
  return {
    status: res.status,
    statusText: res.statusText,
    headers,
    body: text,
    binary: false,
    durationMs,
    sizeBytes: new Blob([text]).size,
  };
}
