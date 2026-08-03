const BASIC_USER = "ada";
const BASIC_PASSWORD = "lovelace";
const TOKEN = "mock-bearer-token";
const API_KEY_NAME = "x-api-key";
const API_KEY_VALUE = "mock-api-key";

const VERSION = "0.1.0";
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_BIG_BYTES = 5 * 1024 * 1024;
const MAX_SLOW_MS = 10_000;
const MAX_REDIRECT_HOPS = 10;

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-expose-headers": "*",
};

const PREFLIGHT = {
  ...CORS,
  "access-control-allow-methods": "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS",
  "access-control-allow-headers": "*",
  "access-control-max-age": "3600",
};

const json = (value, status = 200, headers = {}) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json", ...CORS, ...headers },
  });

const text = (body, status, contentType, headers = {}) =>
  new Response(body, {
    status,
    headers: { "content-type": contentType, ...CORS, ...headers },
  });

const unauthorized = (realm) =>
  json({ error: "unauthorized" }, 401, { "www-authenticate": realm });

const redirect = (location) =>
  new Response(null, { status: 302, headers: { location, ...CORS } });

function received(request, url, body) {
  const query = {};
  for (const [key, value] of url.searchParams) query[key] = value;
  return {
    method: request.method,
    path: url.pathname,
    query,
    headers: [...request.headers].map(([key, value]) => [key, value]),
    body,
  };
}

function basicCredentials(request) {
  const raw = request.headers.get("authorization");
  if (!raw || !raw.startsWith("Basic ")) return null;
  try {
    const decoded = atob(raw.slice("Basic ".length));
    const at = decoded.indexOf(":");
    if (at < 0) return null;
    return [decoded.slice(0, at), decoded.slice(at + 1)];
  } catch {
    return null;
  }
}

function parseJson(body) {
  try {
    return JSON.parse(body);
  } catch {
    return null;
  }
}

const user = (id) => ({ id, name: "Ada Lovelace", email: "ada@example.com" });

function graphql(body) {
  const envelope = parseJson(body);
  const query = envelope && typeof envelope.query === "string" ? envelope.query : "";
  const variables = (envelope && envelope.variables) || {};

  if (!query) {
    return json({ errors: [{ message: "a graphql request needs a query field" }] }, 400);
  }
  if (query.includes("malformed")) {
    return text('{"data": {"malformed":', 200, "application/json");
  }
  if (query.includes("boom")) {
    return json({
      data: null,
      errors: [
        {
          message: "boom: the resolver refused",
          path: ["boom"],
          extensions: { code: "BOOM" },
        },
      ],
    });
  }
  if (query.includes("createUser")) {
    return json({ data: { createUser: { id: "u-2", name: variables.name ?? "unnamed" } } });
  }
  if (query.includes("users")) {
    return json({ data: { users: [user("u-1"), user("u-2")] } });
  }
  if (query.includes("user")) {
    const id = variables.id === undefined ? "u-1" : String(variables.id);
    return json({ data: { user: user(id) } });
  }
  return json({ data: null, errors: [{ message: `unknown query: ${query}` }] });
}

const ECHO_ROUTES = {
  "/get": "GET",
  "/post": "POST",
  "/put": "PUT",
  "/patch": "PATCH",
  "/delete": "DELETE",
  "/head": "HEAD",
  "/options": "OPTIONS",
};

async function handle(request) {
  const url = new URL(request.url);
  const path = url.pathname;

  if (
    request.method === "OPTIONS" &&
    request.headers.has("access-control-request-method")
  ) {
    return new Response(null, { status: 204, headers: PREFLIGHT });
  }

  const raw = request.method === "GET" || request.method === "HEAD" ? "" : await request.text();
  if (raw.length > MAX_BODY_BYTES) {
    return json({ error: "request body is too large", limit: MAX_BODY_BYTES }, 413);
  }
  const echo = () => json(received(request, url, raw));

  if (path === "/health") return json({ status: "ok", version: VERSION });
  if (path === "/") return text("mandalo mock api — GET /health, GET /get", 200, "text/plain");

  if (path in ECHO_ROUTES) {
    if (request.method !== ECHO_ROUTES[path]) {
      return json({ error: `use ${ECHO_ROUTES[path]} on ${path}` }, 405);
    }
    if (request.method === "HEAD") {
      return new Response(null, { status: 200, headers: { ...CORS } });
    }
    return echo();
  }

  if (path.startsWith("/status/")) {
    const code = Number(path.slice("/status/".length));
    if (!Number.isInteger(code) || code < 100 || code > 599) {
      return json({ error: "not a status code" }, 400);
    }
    if (code === 204 || code === 304 || code < 200) {
      return new Response(null, { status: code, headers: { ...CORS } });
    }
    if (code >= 300 && code < 400) {
      return new Response(null, { status: code, headers: { location: "/get", ...CORS } });
    }
    return json({ status: code }, code);
  }

  if (path.startsWith("/redirect/")) {
    const hops = Math.min(Number(path.slice("/redirect/".length)) || 0, MAX_REDIRECT_HOPS);
    return redirect(hops === 0 ? "/get" : `/redirect/${hops - 1}`);
  }

  // A redirect response, never a server-side fetch — a proxy here would be an open SSRF.
  if (path === "/redirect-to") {
    const target = url.searchParams.get("url");
    return target ? redirect(target) : text("redirect-to needs ?url=", 400, "text/plain");
  }

  if (path === "/slow") {
    const ms = Math.min(Number(url.searchParams.get("ms")) || 0, MAX_SLOW_MS);
    await new Promise((resolve) => setTimeout(resolve, ms));
    return echo();
  }

  if (path === "/binary") {
    return new Response(new Uint8Array([0xff, 0xfe, 0x00, 0x01, 0x02, 0x80]), {
      status: 200,
      headers: { "content-type": "application/octet-stream", ...CORS },
    });
  }

  if (path === "/gzip" || path === "/brotli") {
    const encoding = path === "/gzip" ? "gzip" : "brotli";
    return json({ encoding, ok: true });
  }

  if (path === "/big") {
    const bytes = Math.min(Number(url.searchParams.get("bytes")) || 1024, MAX_BIG_BYTES);
    return json({ filler: "x".repeat(bytes) });
  }

  if (path === "/auth/login") {
    const body = parseJson(raw) || {};
    if (body.username !== BASIC_USER || body.password !== BASIC_PASSWORD) {
      return unauthorized("Login");
    }
    return json({ token: TOKEN, user: { id: "u-1", name: "Ada Lovelace" } });
  }

  if (path === "/auth/basic") {
    const credentials = basicCredentials(request);
    if (!credentials || credentials[0] !== BASIC_USER || credentials[1] !== BASIC_PASSWORD) {
      return unauthorized('Basic realm="mock"');
    }
    return json({ authenticated: true, user: credentials[0], path });
  }

  if (path === "/auth/bearer") {
    const presented = request.headers.get("authorization");
    if (presented !== `Bearer ${TOKEN}`) return unauthorized("Bearer");
    return json({ authenticated: true, token: TOKEN });
  }

  if (path === "/auth/apikey") {
    if (request.headers.get(API_KEY_NAME) === API_KEY_VALUE) {
      return json({ authenticated: true, placement: "header" });
    }
    if (url.searchParams.get(API_KEY_NAME) === API_KEY_VALUE) {
      return json({ authenticated: true, placement: "query" });
    }
    return unauthorized("ApiKey");
  }

  if (path === "/headers/echo") {
    const headers = [...request.headers].map(([key, value]) => [key, value]);
    return json({ headers, count: headers.length });
  }

  if (path === "/cookies/set") {
    return json({ cookie: "mock_session=abc123" }, 200, {
      "set-cookie": "mock_session=abc123; Path=/; HttpOnly",
    });
  }

  if (path === "/json") {
    return json({ id: 7, name: "nova", tags: ["a", "b"], nested: { ok: true } });
  }
  if (path === "/xml") {
    return text(
      '<?xml version="1.0"?><user><id>7</id><name>nova</name></user>',
      200,
      "application/xml",
    );
  }
  if (path === "/text") return text("hola", 200, "text/plain; charset=utf-8");
  if (path === "/empty") return new Response(null, { status: 204, headers: { ...CORS } });

  if (path === "/graphql") {
    if (request.method !== "POST") return json({ error: "use POST on /graphql" }, 405);
    return graphql(raw);
  }

  if (path === "/grpc" || path.startsWith("/mock.v1.")) {
    return json(
      {
        error: "gRPC is not available on the hosted mock",
        how: "run the local mock with `make mock-api`, or use the desktop app",
      },
      501,
    );
  }

  return json({ error: `no such route: ${path}` }, 404);
}

export default {
  async fetch(request) {
    const response = await handle(request);
    for (const [key, value] of Object.entries(CORS)) {
      if (!response.headers.has(key)) response.headers.set(key, value);
    }
    return response;
  },
};
