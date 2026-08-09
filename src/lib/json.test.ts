import { describe, expect, it } from "vitest";
import { leafPaths, tokenizeJson, tokenizeLines } from "./json";

describe("json tokenizer", () => {
  it("marks object keys apart from string values", () => {
    const tokens = tokenizeJson('{"a":"b"}');
    expect(tokens.map((t) => [t.type, t.text])).toEqual([
      ["punct", "{"],
      ["key", '"a"'],
      ["punct", ":"],
      ["string", '"b"'],
      ["punct", "}"],
    ]);
  });

  it("classifies numbers, booleans and null", () => {
    const types = tokenizeJson("[1,-2.5e3,true,false,null]")
      .filter((t) => t.type !== "punct")
      .map((t) => t.type);
    expect(types).toEqual(["number", "number", "bool", "bool", "null"]);
  });

  it("keeps escaped quotes inside strings", () => {
    const tokens = tokenizeJson('{"a":"say \\"hi\\""}');
    expect(tokens.find((t) => t.type === "string")?.text).toBe('"say \\"hi\\""');
  });

  it("treats a key-looking string in an array as a value", () => {
    const tokens = tokenizeJson('["a","b"]');
    expect(tokens.filter((t) => t.type === "key")).toHaveLength(0);
  });

  it("splits tokens into lines for the gutter", () => {
    const lines = tokenizeLines('{\n  "a": 1\n}');
    expect(lines).toHaveLength(3);
    expect(lines[1].map((t) => t.text).join("")).toBe('  "a": 1');
  });

  it("passes non-JSON text through as plain text", () => {
    expect(tokenizeJson("hello").map((t) => t.type)).toEqual(["text"]);
  });
});

const paths = (value: unknown) =>
  leafPaths(tokenizeLines(JSON.stringify(value, null, 2))).map(
    (leaf) => leaf?.path ?? null,
  );

describe("leafPaths", () => {
  it("names every leaf of a nested object and array", () => {
    expect(
      paths({ data: { token: "t", ids: [1, 2] }, ok: true, none: null }),
    ).toEqual([
      null,
      null,
      "$.data.token",
      null,
      "$.data.ids[0]",
      "$.data.ids[1]",
      null,
      null,
      "$.ok",
      "$.none",
      null,
    ]);
  });

  it("indexes arrays of objects from zero, per array", () => {
    expect(paths({ items: [{ id: 1 }, { id: 2 }] })).toEqual([
      null,
      null,
      null,
      "$.items[0].id",
      null,
      null,
      "$.items[1].id",
      null,
      null,
      null,
    ]);
  });

  it("brackets a key the dot form cannot hold", () => {
    expect(paths({ "api key": 1, "it's": 2 })).toEqual([
      null,
      "$['api key']",
      "$['it\\'s']",
      null,
    ]);
  });

  it("reports the leaf value and its type, not just the path", () => {
    const leaves = leafPaths(tokenizeLines('{\n"a": "x",\n"b": 2\n}'));
    expect(leaves[1]).toEqual({ path: "$.a", raw: '"x"', type: "string" });
    expect(leaves[2]).toEqual({ path: "$.b", raw: "2", type: "number" });
  });

  it("gives no path to a line holding more than one leaf", () => {
    expect(leafPaths(tokenizeLines('{"a":1,"b":2}'))).toEqual([null]);
  });

  it("reads a bare scalar body as the root", () => {
    expect(leafPaths(tokenizeLines('"hello"'))[0]?.path).toBe("$");
  });

  it("keeps its footing across sibling containers", () => {
    expect(paths({ a: { b: 1 }, c: 2 })).toEqual([
      null,
      null,
      "$.a.b",
      null,
      "$.c",
      null,
    ]);
  });
});
