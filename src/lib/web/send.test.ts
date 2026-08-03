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
