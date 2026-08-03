import { describe, expect, it } from "vitest";
import { collectVarReferences, parseCaptureSource, validateCapture, validateTest } from "../../src/core/validate";

describe("parseCaptureSource", () => {
  it("accepts the three documented grammars", () => {
    expect(parseCaptureSource("status")).toEqual({ source: "status" });
    expect(parseCaptureSource("header.X-Request-Id")).toEqual({ source: "header", name: "X-Request-Id" });
    expect(parseCaptureSource("body.$.token")).toEqual({ source: "body", path: "$.token" });
    expect(parseCaptureSource("body.$.data.items[0].id")).toEqual({
      source: "body",
      path: "$.data.items[0].id",
    });
    expect(parseCaptureSource("body.$['a.b']")).toEqual({ source: "body", path: "$['a.b']" });
  });

  it("rejects an unknown source", () => {
    expect(() => parseCaptureSource("cookie.session")).toThrow(/invalid capture source/);
  });

  it("rejects an empty header name", () => {
    expect(() => parseCaptureSource("header.")).toThrow(/empty header name/);
  });

  it("rejects a body path that does not start with $", () => {
    expect(() => parseCaptureSource("body.token")).toThrow(/must start with \$/);
  });

  it("rejects broken JSONPath", () => {
    expect(() => parseCaptureSource("body.$.items[0")).toThrow(/unbalanced "\["/);
    expect(() => parseCaptureSource("body.$.items0]")).toThrow(/unbalanced "\]"/);
    expect(() => parseCaptureSource("body.$..token")).toThrow(/empty path segment/);
    expect(() => parseCaptureSource("body.$['a")).toThrow(/unterminated quote/);
  });
});

describe("validateCapture", () => {
  it("passes a well-formed capture", () => {
    expect(validateCapture({ from: "body.$.token", into: "token", scope: "run" })).toEqual([]);
  });

  it("flags a bad target name", () => {
    expect(validateCapture({ from: "status", into: "session id", scope: "run" })).toEqual([
      expect.stringContaining("invalid capture target"),
    ]);
  });

  it("flags an unknown scope", () => {
    expect(validateCapture({ from: "status", into: "s", scope: "forever" })).toEqual([
      expect.stringContaining("unknown capture scope"),
    ]);
  });

  it("reports every problem at once", () => {
    expect(validateCapture({ from: "cookie.x", into: "", scope: "forever" })).toHaveLength(3);
  });
});

describe("validateTest", () => {
  it("accepts valid kind/op combinations", () => {
    expect(validateTest({ kind: "status", op: "eq", value: 200 })).toEqual([]);
    expect(validateTest({ kind: "duration", op: "lt", value: 500 })).toEqual([]);
    expect(validateTest({ kind: "header", op: "exists", name: "X" })).toEqual([]);
    expect(validateTest({ kind: "json", op: "len", path: "$.items", value: 3 })).toEqual([]);
  });

  it("rejects an unknown kind", () => {
    expect(validateTest({ kind: "socket", op: "eq" })).toEqual([
      expect.stringContaining('unknown test kind "socket"'),
    ]);
  });

  it("rejects an op the kind does not support", () => {
    expect(validateTest({ kind: "status", op: "matches", value: 200 })[0]).toContain(
      'does not support op "matches"',
    );
    expect(validateTest({ kind: "duration", op: "eq", value: 5 })[0]).toContain("does not support");
  });

  it("requires a value except for exists/absent", () => {
    expect(validateTest({ kind: "status", op: "eq" })[0]).toContain('requires a "value"');
    expect(validateTest({ kind: "header", op: "exists", name: "X", value: "y" })[0]).toContain(
      'takes no "value"',
    );
  });

  it("requires path on json and name on header", () => {
    expect(validateTest({ kind: "json", op: "eq", value: 1 })[0]).toContain('require a "path"');
    expect(validateTest({ kind: "header", op: "eq", value: "a" })[0]).toContain('require a "name"');
  });

  it("requires numeric values for status and duration", () => {
    expect(validateTest({ kind: "status", op: "eq", value: "200" })[0]).toContain("must be a number");
  });
});

describe("collectVarReferences", () => {
  it("finds every reference with its offset", () => {
    const raw = 'url = "{{base}}/x/{{id}}"';
    const refs = collectVarReferences(raw);
    expect(refs.map((r) => r.name)).toEqual(["base", "id"]);
    expect(raw.slice(refs[0]!.offset, refs[0]!.offset + refs[0]!.length)).toBe("{{base}}");
  });

  it("trims padding inside the braces", () => {
    expect(collectVarReferences("{{ token }}")[0]?.name).toBe("token");
  });

  it("ignores empty braces", () => {
    expect(collectVarReferences("{{}} {{ }}")).toEqual([]);
  });
});
