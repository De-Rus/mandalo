import { describe, expect, it } from "vitest";
import { collectVarReferences } from "../../src/core/validate";

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
