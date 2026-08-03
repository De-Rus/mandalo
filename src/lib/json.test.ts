import { describe, expect, it } from "vitest";
import { tokenizeJson, tokenizeLines } from "./json";

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
