import { describe, expect, it } from "vitest";
import { lintRequestDocument } from "../../src/core/rules";

const GOOD = `### Login
POST {{base}}/auth/login
Content-Type: application/json

{ "user": "ada" }

> {%
pm.test("status is 200", function () { pm.response.to.have.status(200); });
%}
`;

function slice(raw: string, finding: { offset: number; length: number }): string {
  return raw.slice(finding.offset, finding.offset + finding.length);
}

describe("lintRequestDocument", () => {
  it("is silent on a healthy request", () => {
    expect(lintRequestDocument(GOOD, "http", { envName: "prod", envVars: { base: "x" } })).toEqual(
      [],
    );
  });

  it("flags an unknown method on its own line", () => {
    const raw = "### R\nFETCH https://a.dev/x\n";
    const findings = lintRequestDocument(raw, "http");
    expect(findings).toHaveLength(1);
    expect(findings[0]?.code).toBe("mandalo.parse");
    expect(findings[0]?.message).toContain('"FETCH" is not an HTTP method');
    expect(slice(raw, findings[0]!)).toBe("FETCH https://a.dev/x");
  });

  it("flags a header line that is missing its colon", () => {
    const raw = "### R\nGET https://a.dev/x\nAccept json\n";
    const findings = lintRequestDocument(raw, "http");
    expect(findings[0]?.message).toContain("expected `Name: value`");
    expect(slice(raw, findings[0]!)).toBe("Accept json");
  });

  it("flags a script block that is never closed", () => {
    const raw = '### R\nGET https://a.dev/x\n\n> {%\npm.test("x", function () {});\n';
    expect(lintRequestDocument(raw, "http")[0]?.message).toContain("never closed with `%}`");
  });

  it("refuses a gRPC call line that names no target", () => {
    const raw = "### Say\nmock.v1.Mock/Say\n";
    expect(lintRequestDocument(raw, "grpc")[0]?.message).toContain("names no target");
  });

  it("refuses a reserved gRPC metadata key", () => {
    const raw = "### Say\nlocalhost:1/p.S/M\nservice: other\n";
    expect(lintRequestDocument(raw, "grpc")[0]?.message).toContain("reads like a Mándalo directive");
  });

  it("warns once per unresolved {{var}} occurrence", () => {
    const raw = "### R\nGET {{base}}/{{missing}}/{{missing}}\n";
    const findings = lintRequestDocument(raw, "http", { envName: "prod", envVars: { base: "x" } });
    expect(findings).toHaveLength(2);
    expect(findings[0]?.severity).toBe("warning");
    expect(findings[0]?.variable).toBe("missing");
    expect(findings[0]?.message).toContain('environment "prod"');
  });

  it("counts a file's own @var as defined", () => {
    const raw = "@host = api.dev\n\n### R\nGET https://{{host}}/x\n";
    expect(lintRequestDocument(raw, "http", { envName: "prod", envVars: {} })).toEqual([]);
  });

  it("stays quiet about vars when no environment is selected", () => {
    expect(lintRequestDocument("### R\nGET {{anything}}\n", "http")).toEqual([]);
  });

  it("reports one error and stops, because a broken file has no further meaning", () => {
    const raw = "### R\nFETCH {{gone}}/x\n";
    const findings = lintRequestDocument(raw, "http", { envName: "prod", envVars: {} });
    expect(findings.map((finding) => finding.code)).toEqual(["mandalo.parse"]);
  });
});
