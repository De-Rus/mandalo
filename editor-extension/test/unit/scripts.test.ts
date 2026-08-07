import { describe, expect, it } from "vitest";
import { collectScriptBlocks, lintScripts } from "../../src/core/scripts";
import { lintRequestDocument } from "../../src/core/rules";

const FORM_LOGIN = `### Form login
POST {{baseUrl}}/body/urlencoded
Content-Type: application/x-www-form-urlencoded

username=ada+lovelace&password=lovelace

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
pm.test("the pairs arrived as written", function () {
  pm.expect(pm.response.json().raw).to.equal("username=ada+lovelace&password=lovelace");
});
%}
`;

function slice(raw: string, finding: { offset: number; length: number }): string {
  return raw.slice(finding.offset, finding.offset + finding.length);
}

describe("collectScriptBlocks", () => {
  it("finds a post block with its absolute offset", () => {
    const blocks = collectScriptBlocks(FORM_LOGIN);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]?.kind).toBe("post");
    expect(FORM_LOGIN.slice(blocks[0]!.offset, blocks[0]!.offset + 8)).toBe('\npm.test');
  });

  it("finds pre blocks and single-line blocks", () => {
    const raw = '< {% pm.environment.set("t", "1"); %}\nGET https://a.dev/x\n\n> {%\npm.test("x", function () {});\n%}\n';
    const blocks = collectScriptBlocks(raw);
    expect(blocks.map((block) => block.kind)).toEqual(["pre", "post"]);
    expect(blocks[0]?.source).toBe(' pm.environment.set("t", "1"); ');
  });

  it("ignores an unclosed block (the parser owns that error)", () => {
    expect(collectScriptBlocks('> {%\npm.test("x");\n')).toEqual([]);
  });
});

describe("lintScripts", () => {
  it("is silent on a valid script", () => {
    expect(lintScripts(FORM_LOGIN)).toEqual([]);
  });

  it("flags a syntax error at its exact position", () => {
    const raw = '### R\nGET https://a.dev/x\n\n> {%\npm.test("x" function () {});\n%}\n';
    const findings = lintScripts(raw);
    expect(findings).toHaveLength(1);
    expect(findings[0]?.code).toBe("mandalo.scriptSyntax");
    expect(findings[0]?.severity).toBe("error");
    expect(raw.slice(findings[0]!.offset)).toMatch(/^function/);
  });

  it("flags a pm member that does not exist", () => {
    const raw = '### R\nGET https://a.dev/x\n\n> {%\npm.responze.json();\n%}\n';
    const findings = lintScripts(raw);
    expect(findings[0]?.code).toBe("mandalo.pmUnknown");
    expect(findings[0]?.message).toContain("pm.responze is not part of the pm API");
    expect(slice(raw, findings[0]!)).toBe("responze");
  });

  it("warns on known-but-unavailable pm members with the engine's reason", () => {
    const raw = '### R\nGET https://a.dev/x\n\n> {%\npm.sendRequest("https://b.dev");\n%}\n';
    const findings = lintScripts(raw);
    expect(findings[0]?.code).toBe("mandalo.pmUnavailable");
    expect(findings[0]?.severity).toBe("warning");
    expect(findings[0]?.message).toContain("scripts cannot make network requests");
  });

  it("warns on pm.response inside a pre-request script", () => {
    const raw = '< {% pm.response.json(); %}\nGET https://a.dev/x\n';
    const findings = lintScripts(raw);
    expect(findings[0]?.message).toContain("not available in a pre-request script");
  });

  it("does not confuse another object's members with pm's", () => {
    const raw = '> {% other.sendRequest(); pm.request.headers.get("x"); %}\nGET https://a.dev/x\n';
    expect(lintScripts('### R\nGET https://a.dev/x\n\n' + raw)).toEqual([]);
  });
});

describe("lintRequestDocument script integration", () => {
  it("reports script findings alongside request findings", () => {
    const raw = '### R\nGET {{missing}}/x\n\n> {%\npm.visualizer.set();\n%}\n';
    const findings = lintRequestDocument(raw, "http", { envName: "prod", envVars: {} });
    const codes = findings.map((finding) => finding.code).sort();
    expect(codes).toEqual(["mandalo.pmUnavailable", "mandalo.unresolvedVar"]);
  });
});
