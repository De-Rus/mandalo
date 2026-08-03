import { describe, expect, it } from "vitest";
import { newDraft, type KVRow } from "./draft";
import {
  activeRows,
  buildGrpcRequest,
  buildRequestSpec,
  mergeParams,
  parseProtoPaths,
} from "./spec";

function row(key: string, value: string, enabled = true): KVRow {
  return { id: key + value, key, value, enabled };
}

describe("activeRows", () => {
  it("filters disabled and empty-key rows", () => {
    const rows = [
      row("a", "1"),
      row("b", "2", false),
      row("", "orphan"),
      row("  c  ", "3"),
    ];
    expect(activeRows(rows)).toEqual([
      ["a", "1"],
      ["c", "3"],
    ]);
  });
});

describe("mergeParams", () => {
  it("appends query params to a bare URL", () => {
    expect(mergeParams("https://x.dev/a", [["q", "hi there"]])).toBe(
      "https://x.dev/a?q=hi%20there",
    );
  });

  it("appends with & when the URL already has a query", () => {
    expect(mergeParams("https://x.dev/a?p=1", [["q", "2"]])).toBe(
      "https://x.dev/a?p=1&q=2",
    );
  });

  it("leaves the URL alone with no params", () => {
    expect(mergeParams("https://x.dev/a", [])).toBe("https://x.dev/a");
  });

  it("preserves {{var}} tokens verbatim while encoding around them", () => {
    expect(
      mergeParams("https://x.dev/a", [["q", "{{term}} extra"]]),
    ).toBe("https://x.dev/a?q={{term}}%20extra");
    expect(mergeParams("https://x.dev/a", [["{{key}}", "{{value}}"]])).toBe(
      "https://x.dev/a?{{key}}={{value}}",
    );
  });
});

describe("buildRequestSpec", () => {
  it("assembles an http spec, filtering disabled rows and merging params", () => {
    const draft = newDraft();
    draft.kind = "http";
    draft.method = "POST";
    draft.url = "https://api.dev/users";
    draft.params = [row("page", "2"), row("debug", "1", false), row("", "")];
    draft.headers = [row("X-One", "a"), row("X-Off", "b", false)];
    draft.body = '{"name":"nova"}';
    draft.auth.type = "bearer";
    draft.auth.token = "tok";

    const spec = buildRequestSpec(draft, { base: "x" });

    expect(spec).toEqual({
      kind: "http",
      method: "POST",
      url: "https://api.dev/users?page=2",
      headers: [["X-One", "a"]],
      body: '{"name":"nova"}',
      auth: { type: "bearer", token: "tok" },
      graphql: null,
      vars: { base: "x" },
    });
  });

  it("sends null body when the body is blank", () => {
    const draft = newDraft();
    draft.url = "https://api.dev";
    draft.body = "   \n";
    expect(buildRequestSpec(draft, {}).body).toBeNull();
  });

  it("assembles a graphql spec without touching params", () => {
    const draft = newDraft();
    draft.kind = "graphql";
    draft.url = "https://api.dev/graphql";
    draft.params = [row("ignored", "1")];
    draft.graphqlQuery = "query { ok }";
    draft.graphqlVariables = '{"a":1}';

    const spec = buildRequestSpec(draft, {});

    expect(spec.kind).toBe("graphql");
    expect(spec.method).toBe("POST");
    expect(spec.url).toBe("https://api.dev/graphql");
    expect(spec.body).toBeNull();
    expect(spec.graphql).toEqual({ query: "query { ok }", variables: '{"a":1}' });
  });

  it("maps apikey auth", () => {
    const draft = newDraft();
    draft.url = "https://api.dev";
    draft.auth = { ...draft.auth, type: "apikey", key: "X-Key", value: "v", placement: "query" };
    expect(buildRequestSpec(draft, {}).auth).toEqual({
      type: "apikey",
      key: "X-Key",
      value: "v",
      placement: "query",
    });
  });
});

describe("parseProtoPaths", () => {
  it("splits lines, trims, and drops blanks", () => {
    expect(parseProtoPaths(" /a.proto \n\n/b.proto\n  ")).toEqual([
      "/a.proto",
      "/b.proto",
    ]);
  });
});

describe("buildGrpcRequest", () => {
  it("assembles a grpc request with active metadata only", () => {
    const draft = newDraft();
    draft.kind = "grpc";
    draft.url = "http://localhost:50051";
    draft.grpc = {
      protoPaths: "/a.proto\n/b.proto",
      service: "pkg.Users",
      method: "Get",
      message: '{"id":1}',
      metadata: [row("authorization", "t"), row("off", "x", false), row("", "")],
    };

    expect(buildGrpcRequest(draft, { env: "1" })).toEqual({
      url: "http://localhost:50051",
      protoPaths: ["/a.proto", "/b.proto"],
      service: "pkg.Users",
      method: "Get",
      message: '{"id":1}',
      metadata: [["authorization", "t"]],
      vars: { env: "1" },
    });
  });
});
