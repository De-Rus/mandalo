import { describe, expect, it } from "vitest";
import { applyRequestEdit, findKey, scanSegments } from "../../src/core/edit";
import type { RequestModel } from "../../src/core/model";
import { parseRequest, renderRequest } from "../../src/core/parse";

const LOGIN = `# The login call. Keep the comment.
schema_version = 1
id = "550e8400-e29b-41d4-a716-446655440000"
name = "Login"
kind    =    "http"
method = "POST"
url = "{{base}}/auth/login"
headers = [["Content-Type", "application/json"]]
body = "{\\"user\\":\\"me\\"}"

[auth]
type = "bearer"
token = "{{token}}"

[scripts]
pre = "pm.environment.set('ts', Date.now())"
post = "pm.test('ok', () => pm.response.to.have.status(200))"

[[tests]]
kind = "status"
op = "eq"
value = 200

[[captures]]
from = "body.$.token"
into = "token"
scope = "run"
`;

function edited(raw: string, patch: (model: RequestModel) => void): string {
  const model = parseRequest(raw);
  patch(model);
  return applyRequestEdit(raw, model);
}

function changedLines(before: string, after: string): { removed: string[]; added: string[] } {
  const a = before.split("\n");
  const b = after.split("\n");
  return {
    removed: a.filter((line) => !b.includes(line)),
    added: b.filter((line) => !a.includes(line)),
  };
}

describe("scanSegments", () => {
  it("splits the document at table headers", () => {
    const headers = scanSegments(LOGIN).map((segment) => segment.header);
    expect(headers).toEqual([null, "[auth]", "[scripts]", "[[tests]]", "[[captures]]"]);
  });

  it("never splits on a bracket that lives inside a string", () => {
    const raw = 'name = "x"\nbody = """\n[not-a-table]\n"""\n\n[auth]\ntype = "none"\n';
    expect(scanSegments(raw).map((s) => s.header)).toEqual([null, "[auth]"]);
  });

  it("ignores a bracket inside a comment", () => {
    const raw = 'name = "x"\n# [auth] is described below\nurl = "u"\n';
    expect(scanSegments(raw).map((s) => s.header)).toEqual([null]);
  });
});

describe("findKey", () => {
  it("tolerates padding around the equals sign", () => {
    const span = findKey(LOGIN, "kind", 0, LOGIN.length);
    expect(LOGIN.slice(span!.valueStart, span!.end)).toBe('"http"');
  });

  it("does not match a key that is only a prefix", () => {
    const raw = 'method_hint = "a"\nmethod = "GET"\n';
    const span = findKey(raw, "method", 0, raw.length);
    expect(raw.slice(span!.valueStart, span!.end)).toBe('"GET"');
  });

  it("spans a multi-line array value", () => {
    const raw = 'headers = [\n  ["A", "1"],\n  ["B", "2"],\n]\nname = "x"\n';
    const span = findKey(raw, "headers", 0, raw.length);
    expect(raw.slice(span!.valueStart, span!.end)).toBe('[\n  ["A", "1"],\n  ["B", "2"],\n]');
  });
});

describe("applyRequestEdit", () => {
  it("is a byte-for-byte no-op when nothing changed", () => {
    expect(applyRequestEdit(LOGIN, parseRequest(LOGIN))).toBe(LOGIN);
  });

  it("touches exactly one line when the URL changes", () => {
    const after = edited(LOGIN, (model) => {
      model.url = "{{base}}/auth/token";
    });
    expect(changedLines(LOGIN, after)).toEqual({
      removed: ['url = "{{base}}/auth/login"'],
      added: ['url = "{{base}}/auth/token"'],
    });
  });

  it("keeps the author's alignment on the very line it rewrites", () => {
    const raw = 'schema_version = 1\nid   = "x"\nname = "Ping"\nkind = "http"\nmethod  = "GET"\nurl     = "/a"\nheaders = []\n';
    const after = edited(raw, (model) => {
      model.url = "/b";
    });
    expect(after).toContain('url     = "/b"');
    expect(after).toContain('method  = "GET"');
  });

  it("keeps comments, blank lines and odd spacing outside the edit", () => {
    const after = edited(LOGIN, (model) => {
      model.method = "PUT";
    });
    expect(after).toContain("# The login call. Keep the comment.");
    expect(after).toContain('kind    =    "http"');
    expect(after.split("\n").length).toBe(LOGIN.split("\n").length);
  });

  it("rewrites only the [auth] block when auth changes", () => {
    const after = edited(LOGIN, (model) => {
      model.auth = { type: "none" };
    });
    expect(changedLines(LOGIN, after)).toEqual({
      removed: ['type = "bearer"', 'token = "{{token}}"'],
      added: ['type = "none"'],
    });
  });

  it("leaves the tests block alone when only a capture changes", () => {
    const after = edited(LOGIN, (model) => {
      model.captures = [{ from: "body.$.jwt", into: "token", scope: "session" }];
    });
    expect(after).toContain('[[tests]]\nkind = "status"\nop = "eq"\nvalue = 200');
    expect(after).toContain('from = "body.$.jwt"');
    expect(after).not.toContain('from = "body.$.token"');
  });

  it("adds a key that the file never had, without reordering the rest", () => {
    const after = edited(LOGIN, (model) => {
      model.description = "Exchanges credentials for a token";
    });
    expect(after).toContain('description = "Exchanges credentials for a token"');
    expect(after.indexOf("description =")).toBeLessThan(after.indexOf("[auth]"));
    expect(after).toContain('body = "{\\"user\\":\\"me\\"}"');
  });

  it("drops a key that lost its value", () => {
    const after = edited(LOGIN, (model) => {
      delete model.body;
    });
    expect(after).not.toContain("body =");
    expect(after).toContain('url = "{{base}}/auth/login"');
  });

  it("appends a block the file did not have", () => {
    const raw = 'schema_version = 1\nid = "x"\nname = "Ping"\nkind = "http"\nmethod = "GET"\nurl = "u"\nheaders = []\n';
    const after = edited(raw, (model) => {
      model.graphql = { query: "query { me }", variables: "{}" };
    });
    expect(after.startsWith(raw)).toBe(true);
    expect(after).toContain('[graphql]\nquery = "query { me }"\nvariables = "{}"');
  });

  it("removes a block whose value disappeared", () => {
    const after = edited(LOGIN, (model) => {
      model.scripts = {};
    });
    expect(after).not.toContain("[scripts]");
    expect(after).toContain("[[tests]]");
    expect(after).toContain("[auth]");
  });

  it("survives the round trip: edit, reparse, edit back", () => {
    const once = edited(LOGIN, (model) => {
      model.name = "Sign in";
      model.headers = [
        ["Content-Type", "application/json"],
        ["X-Trace", "1"],
      ];
    });
    const back = edited(once, (model) => {
      model.name = "Login";
      model.headers = [["Content-Type", "application/json"]];
    });
    expect(back).toBe(LOGIN);
  });

  it("produces a file the parser reads back identically", () => {
    const after = edited(LOGIN, (model) => {
      model.tests = [
        { kind: "status", op: "eq", value: 201 },
        { kind: "json", op: "exists", path: "$.token" },
      ];
      model.url = "{{base}}/v2/auth/login";
    });
    const reparsed = parseRequest(after);
    expect(reparsed.url).toBe("{{base}}/v2/auth/login");
    expect(reparsed.tests).toEqual([
      { kind: "status", op: "eq", value: 201 },
      { kind: "json", op: "exists", path: "$.token" },
    ]);
    expect(applyRequestEdit(after, reparsed)).toBe(after);
  });

  it("keeps CRLF files on CRLF", () => {
    const raw = LOGIN.replace(/\n/g, "\r\n");
    const after = edited(raw, (model) => {
      model.url = "{{base}}/x";
    });
    expect(after).not.toMatch(/[^\r]\n/);
  });

  it("stays far smaller than a full re-render", () => {
    const after = edited(LOGIN, (model) => {
      model.url = "{{base}}/auth/token";
    });
    const rendered = renderRequest(parseRequest(LOGIN));
    expect(changedLines(LOGIN, after).added).toHaveLength(1);
    expect(changedLines(LOGIN, after).removed).toHaveLength(1);
    expect(rendered).not.toContain("# The login call. Keep the comment.");
    expect(rendered).not.toContain('kind    =    "http"');
    expect(after).toContain("# The login call. Keep the comment.");
    expect(after).toContain('kind    =    "http"');
  });
});
