import { describe, expect, it } from "vitest";
import { lintScriptSource } from "./lint";

const VALID = `
pm.test("status is 200", function () { pm.response.to.have.status(200); });
pm.test("the pairs arrived as written", function () {
  pm.expect(pm.response.json().raw).to.equal("username=ada+lovelace&password=lovelace");
});
`;

describe("lintScriptSource", () => {
  it("is silent on a valid post-response script", () => {
    expect(lintScriptSource(VALID, "post")).toEqual([]);
  });

  it("flags a syntax error at its position", () => {
    const source = 'pm.test("x" function () {});';
    const findings = lintScriptSource(source, "post");
    expect(findings).toHaveLength(1);
    expect(findings[0]?.code).toBe("mandalo.scriptSyntax");
    expect(findings[0]?.severity).toBe("error");
    expect(source.slice(findings[0]!.from)).toMatch(/^function/);
  });

  it("flags a pm member that does not exist", () => {
    const source = "pm.responze.json();";
    const findings = lintScriptSource(source, "post");
    expect(findings[0]?.code).toBe("mandalo.pmUnknown");
    expect(source.slice(findings[0]!.from, findings[0]!.to)).toBe("responze");
  });

  it("warns on unavailable members with the engine's reason", () => {
    const findings = lintScriptSource('pm.sendRequest("https://b.dev");', "post");
    expect(findings[0]?.code).toBe("mandalo.pmUnavailable");
    expect(findings[0]?.severity).toBe("warning");
    expect(findings[0]?.message).toContain("scripts cannot make network requests");
  });

  it("warns on pm.response in a pre-request script but not in a post one", () => {
    const source = "pm.response.json();";
    expect(lintScriptSource(source, "pre")[0]?.message).toContain("pre-request");
    expect(lintScriptSource(source, "post")).toEqual([]);
  });

  it("leaves other objects' members alone", () => {
    expect(lintScriptSource("other.sendRequest(); const pmx = { cookies: 1 };", "post")).toEqual(
      [],
    );
  });
});
