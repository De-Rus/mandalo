import { describe, expect, it } from "vitest";
import { fromSaved, toSaved } from "./collection";
import { newDraft, uid } from "./draft";

describe("draft <-> SavedRequest mapping", () => {
  it("folds enabled rows to tuples and drops disabled rows", () => {
    const draft = newDraft("Users");
    draft.method = "POST";
    draft.url = "https://x.dev/users";
    draft.headers = [
      { id: uid(), key: "Accept", value: "application/json", enabled: true },
      { id: uid(), key: "X-Debug", value: "1", enabled: false },
      { id: uid(), key: "", value: "", enabled: true },
    ];
    draft.params = [
      { id: uid(), key: "page", value: "2", enabled: true },
      { id: uid(), key: "limit", value: "5", enabled: false },
    ];
    draft.body = "{}";

    const saved = toSaved(draft);
    expect(saved.headers).toEqual([["Accept", "application/json"]]);
    expect(saved.url).toBe("https://x.dev/users?page=2");
    expect(saved.body).toBe("{}");
    expect(saved.graphql).toBeNull();
    expect(saved.grpc).toBeNull();
  });

  it("maps blank body to null", () => {
    const draft = newDraft();
    draft.body = "   ";
    expect(toSaved(draft).body).toBeNull();
  });

  it("roundtrips url and params including {{var}} tokens", () => {
    const draft = newDraft("Params roundtrip");
    draft.url = "https://x.dev/search";
    draft.params = [
      { id: uid(), key: "q", value: "{{term}} extra", enabled: true },
      { id: uid(), key: "page", value: "2", enabled: true },
      { id: uid(), key: "", value: "", enabled: true },
    ];

    const saved = toSaved(draft);
    expect(saved.url).toBe("https://x.dev/search?q={{term}}%20extra&page=2");

    const back = fromSaved(saved);
    expect(back.url).toBe("https://x.dev/search");
    expect(back.params.map((r) => [r.key, r.value])).toEqual([
      ["q", "{{term}} extra"],
      ["page", "2"],
      ["", ""],
    ]);
  });

  it("throws a readable error on an unknown kind", () => {
    const saved = { ...toSaved(newDraft("Broken")), kind: "soap" as never };
    expect(() => fromSaved(saved)).toThrow(
      'Request "Broken" has an unknown kind "soap"',
    );
  });

  it("roundtrips an http request with auth", () => {
    const draft = newDraft("Auth roundtrip");
    draft.method = "PUT";
    draft.url = "https://x.dev/a";
    draft.auth = {
      type: "apikey",
      token: "",
      username: "",
      password: "",
      key: "X-Key",
      value: "s3cret",
      placement: "query",
    };
    draft.headers = [
      { id: uid(), key: "A", value: "1", enabled: true },
      { id: uid(), key: "", value: "", enabled: true },
    ];

    const back = fromSaved(toSaved(draft));
    expect(back.id).toBe(draft.id);
    expect(back.name).toBe("Auth roundtrip");
    expect(back.kind).toBe("http");
    expect(back.method).toBe("PUT");
    expect(back.url).toBe("https://x.dev/a");
    expect(back.auth.type).toBe("apikey");
    expect(back.auth.key).toBe("X-Key");
    expect(back.auth.value).toBe("s3cret");
    expect(back.auth.placement).toBe("query");
    expect(back.headers.map((r) => [r.key, r.value, r.enabled])).toEqual([
      ["A", "1", true],
      ["", "", true],
    ]);
  });

  it("roundtrips basic auth", () => {
    const draft = newDraft();
    draft.auth = { ...draft.auth, type: "basic", username: "u", password: "p" };
    const back = fromSaved(toSaved(draft));
    expect(back.auth.type).toBe("basic");
    expect(back.auth.username).toBe("u");
    expect(back.auth.password).toBe("p");
  });

  it("roundtrips a graphql request", () => {
    const draft = newDraft("GQL");
    draft.kind = "graphql";
    draft.url = "https://x.dev/graphql";
    draft.graphqlQuery = "{ me { id } }";
    draft.graphqlVariables = '{"a":1}';

    const saved = toSaved(draft);
    expect(saved.graphql).toEqual({
      query: "{ me { id } }",
      variables: '{"a":1}',
    });
    const back = fromSaved(saved);
    expect(back.kind).toBe("graphql");
    expect(back.graphqlQuery).toBe("{ me { id } }");
    expect(back.graphqlVariables).toBe('{"a":1}');
  });

  it("roundtrips a grpc request", () => {
    const draft = newDraft("Echo");
    draft.kind = "grpc";
    draft.url = "http://localhost:50051";
    draft.grpc = {
      protoPaths: "/a/echo.proto\n/b/other.proto\n",
      service: "echo.Echo",
      method: "Say",
      message: '{"text":"hi"}',
      metadata: [
        { id: uid(), key: "x-trace", value: "t1", enabled: true },
        { id: uid(), key: "x-off", value: "no", enabled: false },
      ],
    };

    const saved = toSaved(draft);
    expect(saved.grpc).toEqual({
      protoPaths: ["/a/echo.proto", "/b/other.proto"],
      service: "echo.Echo",
      method: "Say",
      message: '{"text":"hi"}',
      metadata: [["x-trace", "t1"]],
    });

    const back = fromSaved(saved);
    expect(back.grpc.protoPaths).toBe("/a/echo.proto\n/b/other.proto");
    expect(back.grpc.service).toBe("echo.Echo");
    expect(back.grpc.metadata.map((r) => [r.key, r.value])).toEqual([
      ["x-trace", "t1"],
      ["", ""],
    ]);
  });
});

describe("scripts, assertions and captures", () => {
  it("persists scripts as null when blank and keeps them when set", () => {
    const draft = newDraft("Scripted");
    expect(toSaved(draft).scripts).toEqual({ pre: null, post: null });

    draft.preScript = "pm.environment.set('a', '1');";
    draft.testScript = "pm.test('ok', () => {});";
    const saved = toSaved(draft);
    expect(saved.scripts).toEqual({
      pre: "pm.environment.set('a', '1');",
      post: "pm.test('ok', () => {});",
    });
    const back = fromSaved(saved);
    expect(back.preScript).toBe("pm.environment.set('a', '1');");
    expect(back.testScript).toBe("pm.test('ok', () => {});");
  });

  it("roundtrips declarative assertions", () => {
    const draft = newDraft("Asserted");
    draft.tests = [
      { kind: "status", op: "eq", value: 201 },
      { kind: "json", path: "$.id", op: "exists" },
    ];
    const back = fromSaved(toSaved(draft));
    expect(back.tests).toEqual(draft.tests);
  });

  it("drops captures without a target variable", () => {
    const draft = newDraft("Captured");
    draft.captures = [
      { from: "body.$.id", into: "userId", scope: "session" },
      { from: "status", into: "  ", scope: "run" },
    ];
    expect(toSaved(draft).captures).toEqual([
      { from: "body.$.id", into: "userId", scope: "session" },
    ]);
  });

  it("preserves tests and captures the UI no longer authors", () => {
    const saved = {
      ...toSaved(newDraft("Legacy")),
      tests: [
        { kind: "status" as const, op: "eq" as const, value: 201 },
        {
          kind: "json" as const,
          path: "$.id",
          op: "exists" as const,
        },
      ],
      captures: [
        { from: "body.$.token", into: "token", scope: "persist" as const },
      ],
    };

    const draft = fromSaved(saved, "acme", "legacy.toml");
    draft.name = "Legacy renamed";
    const back = toSaved(draft);

    expect(back.tests).toEqual(saved.tests);
    expect(back.captures).toEqual(saved.captures);
  });

  it("carries the collection and path through fromSaved", () => {
    const saved = toSaved(newDraft("Located"));
    const back = fromSaved(saved, "acme", "users/located.toml");
    expect(back.collection).toBe("acme");
    expect(back.path).toBe("users/located.toml");
  });

  it("maps a blank description to null", () => {
    const draft = newDraft();
    draft.description = "   ";
    expect(toSaved(draft).description).toBeNull();
    draft.description = "Fetches users";
    expect(toSaved(draft).description).toBe("Fetches users");
  });
});
