import { describe, expect, it } from "vitest";
import { lintRequestDocument } from "../../src/core/rules";

const GOOD = `
name = "Login"
kind = "http"
method = "POST"
url = "{{base}}/auth/login"

[[tests]]
kind = "status"
op = "eq"
value = 200

[[captures]]
from = "body.$.token"
into = "token"
scope = "run"
`;

describe("lintRequestDocument", () => {
  it("is silent on a healthy request", () => {
    expect(lintRequestDocument(GOOD, { envName: "prod", envVars: { base: "x" } })).toEqual([]);
  });

  it("flags an unknown kind at the kind value", () => {
    const raw = 'name = "R"\nkind = "websocket"\nurl = "u"';
    const findings = lintRequestDocument(raw);
    expect(findings).toHaveLength(1);
    expect(findings[0]?.code).toBe("mandalo.kind");
    expect(raw.slice(findings[0]!.offset, findings[0]!.offset + findings[0]!.length)).toBe('"websocket"');
  });

  it("flags a bad capture source at the from value", () => {
    const raw = `name = "R"\nurl = "u"\n\n[[captures]]\nfrom = "cookie.session"\ninto = "s"\nscope = "run"\n`;
    const findings = lintRequestDocument(raw);
    expect(findings[0]?.code).toBe("mandalo.capture");
    expect(raw.slice(findings[0]!.offset, findings[0]!.offset + findings[0]!.length)).toBe(
      '"cookie.session"',
    );
  });

  it("points a bad scope at the scope value, not at from", () => {
    const raw = `name = "R"\nurl = "u"\n\n[[captures]]\nfrom = "status"\ninto = "s"\nscope = "forever"\n`;
    const findings = lintRequestDocument(raw);
    expect(raw.slice(findings[0]!.offset, findings[0]!.offset + findings[0]!.length)).toBe('"forever"');
  });

  it("locates the op of the offending [[tests]] entry", () => {
    const raw = `name = "R"\nurl = "u"\n\n[[tests]]\nkind = "status"\nop = "eq"\nvalue = 200\n\n[[tests]]\nkind = "duration"\nop = "eq"\nvalue = 5\n`;
    const findings = lintRequestDocument(raw);
    expect(findings).toHaveLength(1);
    const secondOp = raw.lastIndexOf('op = "eq"');
    expect(findings[0]!.offset).toBeGreaterThan(secondOp);
  });

  it("warns once per unresolved {{var}} occurrence", () => {
    const raw = 'name = "R"\nurl = "{{base}}/{{missing}}/{{missing}}"';
    const findings = lintRequestDocument(raw, { envName: "prod", envVars: { base: "x" } });
    expect(findings).toHaveLength(2);
    expect(findings[0]?.severity).toBe("warning");
    expect(findings[0]?.variable).toBe("missing");
    expect(findings[0]?.message).toContain('environment "prod"');
  });

  it("stays quiet about vars when no environment is selected", () => {
    expect(lintRequestDocument('name = "R"\nurl = "{{anything}}"')).toEqual([]);
  });

  it("reports a parse failure as a single error", () => {
    const findings = lintRequestDocument("name = ");
    expect(findings).toHaveLength(1);
    expect(findings[0]?.code).toBe("mandalo.parse");
    expect(findings[0]?.severity).toBe("error");
  });

  it("collects several problems from one document", () => {
    const raw = `name = "R"\nkind = "websocket"\nurl = "{{gone}}"\n\n[[tests]]\nkind = "status"\nop = "matches"\nvalue = 1\n\n[[captures]]\nfrom = "cookie.x"\ninto = "a b"\nscope = "never"\n`;
    const findings = lintRequestDocument(raw, { envName: "prod", envVars: {} });
    expect(findings.map((f) => f.code).sort()).toEqual([
      "mandalo.capture",
      "mandalo.capture",
      "mandalo.capture",
      "mandalo.kind",
      "mandalo.test",
      "mandalo.unresolvedVar",
    ]);
  });
});
