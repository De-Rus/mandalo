import { afterEach, describe, expect, it, vi } from "vitest";
import type { RequestSpec } from "../api";
import { snapshot } from "./bus";
import { prepare, webSend } from "./send";

function spec(patch: Partial<RequestSpec>): RequestSpec {
  return {
    kind: "http",
    method: "GET",
    url: "https://api.acme.com/a",
    headers: [],
    body: null,
    auth: { type: "none" },
    graphql: null,
    vars: {},
    ...patch,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("what the browser puts on the wire", () => {
  it("lets auth replace the user's Authorization header instead of doubling it", () => {
    const headers = prepare(
      spec({
        headers: [["Authorization", "stale"]],
        auth: { type: "bearer", token: "fresh" },
      }),
    ).headers.filter(([key]) => key.toLowerCase() === "authorization");

    expect(headers).toEqual([["Authorization", "Bearer fresh"]]);
  });

  it("keeps the user's Authorization header when auth is none or apikey", () => {
    for (const auth of [
      { type: "none" } as const,
      { type: "apikey", key: "X-Api-Key", value: "abc", placement: "header" } as const,
    ]) {
      const headers = prepare(spec({ headers: [["Authorization", "custom scheme"]], auth }))
        .headers.filter(([key]) => key.toLowerCase() === "authorization");
      expect(headers, auth.type).toEqual([["Authorization", "custom scheme"]]);
    }
  });

  it("never calls a body JSON just because it is a body", () => {
    const contentType = (body: string): string | undefined =>
      prepare(spec({ method: "POST", body })).headers.find(
        ([key]) => key.toLowerCase() === "content-type",
      )?.[1];

    expect(contentType("plain words")).toBe("text/plain");
    expect(contentType("<user/>")).toBe("application/xml");
    expect(contentType('{"a": 1}')).toBe("application/json");
    expect(prepare(spec({ method: "POST" })).headers).toEqual([]);
  });
});

describe("what the browser says when a request never leaves", () => {
  it("explains CORS for a cross-origin call", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));

    await expect(webSend(spec({}))).rejects.toThrow(/two things this can mean/);
    expect(snapshot()).toMatchObject({ kind: "unreachable", cors: true });
  });

  it("does not blame CORS for a same-origin call", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));

    await expect(webSend(spec({ url: `${location.origin}/a` }))).rejects.toThrow(
      /CORS is not the reason/,
    );
    expect(snapshot()).toMatchObject({ kind: "unreachable", cors: false, preflight: false });
  });

  it("does not blame CORS for a URL no page could fetch", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));

    await expect(webSend(spec({ url: "ftp://files.acme.com/a" }))).rejects.toThrow(
      /CORS is not the reason/,
    );
    expect(snapshot()).toMatchObject({ kind: "unreachable", cors: false });
  });
});

describe("what a browser primitive would get wrong", () => {
  // btoa is Latin-1: `añá:pß` comes out as different credentials, and anything above
  // U+00FF throws. Rust and the extension base64 the UTF-8 bytes.
  it("base64s basic credentials from UTF-8, not Latin-1", () => {
    const prepared = prepare(
      spec({ auth: { type: "basic", username: "añá", password: "pß" } }),
    );

    expect(prepared.headers).toEqual([["Authorization", "Basic YcOxw6E6cMOf"]]);
  });

  it("survives a credential above U+00FF, which btoa cannot encode at all", () => {
    const prepared = prepare(
      spec({ auth: { type: "basic", username: "日本", password: "語" } }),
    );

    const [, value] = prepared.headers[0] as [string, string];
    expect(value).toBe("Basic 5pel5pysOuiqng==");
    expect(new TextDecoder().decode(Uint8Array.from(atob(value.slice(6)), (c) => c.charCodeAt(0)))).toBe(
      "日本:語",
    );
  });

  // `key in vars` walks the prototype chain, so every Object.prototype member would
  // resolve to a value nobody put in the environment.
  it("resolves only own properties, so {{constructor}} fails loud", () => {
    for (const name of ["constructor", "toString", "hasOwnProperty", "__proto__"]) {
      expect(() => prepare(spec({ url: `https://a.dev/{{${name}}}` }))).toThrow(
        `unresolved variable: ${name}`,
      );
    }
  });

  it("still resolves a variable that shadows a prototype member", () => {
    const prepared = prepare(
      spec({ url: "https://a.dev/{{constructor}}", vars: { constructor: "ok" } }),
    );

    expect(prepared.url).toBe("https://a.dev/ok");
  });
});
