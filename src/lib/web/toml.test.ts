import { describe, expect, it } from "vitest";
import type { SavedRequest } from "../api";
import {
  decodeEnvDoc,
  decodeManifest,
  decodeRequest,
  encodeCollectionManifest,
  encodeEnvDoc,
  encodeRequest,
  encodeWorkspaceManifest,
  parseToml,
  stringifyToml,
} from "./toml";

describe("parseToml scalars", () => {
  it("parses bare and quoted keys", () => {
    const t = parseToml(`a = 1\n"b c" = 2\n'd.e' = 3\nf-g_H = 4\n`);
    expect(t).toEqual({ a: 1, "b c": 2, "d.e": 3, "f-g_H": 4 });
  });

  it("parses booleans", () => {
    expect(parseToml("a = true\nb = false\n")).toEqual({ a: true, b: false });
  });

  it("parses integers with separators, signs, hex, octal and binary", () => {
    const t = parseToml(
      ["a = 1_000_000", "b = -17", "c = +42", "d = 0xDEAD_beef", "e = 0o755", "f = 0b1010"].join(
        "\n",
      ),
    );
    expect(t).toEqual({ a: 1000000, b: -17, c: 42, d: 0xdeadbeef, e: 493, f: 10 });
  });

  it("parses floats including exponents and infinities", () => {
    const t = parseToml("a = 3.14\nb = -0.5\nc = 1e6\nd = 6.626e-34\ne = inf\nf = -inf\n");
    expect(t.a).toBe(3.14);
    expect(t.b).toBe(-0.5);
    expect(t.c).toBe(1e6);
    expect(t.d).toBe(6.626e-34);
    expect(t.e).toBe(Infinity);
    expect(t.f).toBe(-Infinity);
    expect(Number.isNaN(parseToml("a = nan\n").a as number)).toBe(true);
  });

  it("returns dates and times as strings", () => {
    const t = parseToml(
      [
        "a = 1979-05-27T07:32:00Z",
        "b = 1979-05-27",
        "c = 07:32:00.999",
        "d = 1979-05-27 07:32:00+01:00",
      ].join("\n"),
    );
    expect(t).toEqual({
      a: "1979-05-27T07:32:00Z",
      b: "1979-05-27",
      c: "07:32:00.999",
      d: "1979-05-27 07:32:00+01:00",
    });
  });
});

describe("parseToml strings", () => {
  it("parses basic strings with every escape", () => {
    const t = parseToml(
      `a = "line\\nbreak\\ttab\\r\\"quote\\"\\\\slash\\b\\f"\nb = "\\u00e9 \\U0001F600"\n`,
    );
    expect(t.a).toBe('line\nbreak\ttab\r"quote"\\slash\b\f');
    expect(t.b).toBe("\u00e9 \u{1F600}");
  });

  it("parses literal strings without interpreting escapes", () => {
    expect(parseToml(`a = 'C:\\Users\\n'\n`).a).toBe("C:\\Users\\n");
  });

  it("parses multi-line basic strings and trims the first newline", () => {
    const t = parseToml('a = """\nfirst\nsecond\n"""\n');
    expect(t.a).toBe("first\nsecond\n");
  });

  it("keeps content when a multi-line string does not start with a newline", () => {
    expect(parseToml('a = """first\nsecond"""\n').a).toBe("first\nsecond");
  });

  it("honours the line-ending backslash continuation", () => {
    const t = parseToml('a = """\nthe quick \\\n     brown fox\n"""\n');
    expect(t.a).toBe("the quick brown fox\n");
  });

  it("allows one or two quotes next to the multi-line delimiter", () => {
    expect(parseToml('a = """he said ""hi""""\n').a).toBe('he said ""hi"');
    expect(parseToml('a = """say "hi" now"""\n').a).toBe('say "hi" now');
  });

  it("parses multi-line literal strings verbatim", () => {
    const t = parseToml("a = '''\n\\not\\an\\escape\n'''\n");
    expect(t.a).toBe("\\not\\an\\escape\n");
    expect(parseToml("a = ''''quoted''''\n").a).toBe("'quoted'");
  });

  it("parses escaped quotes inside multi-line strings", () => {
    expect(parseToml('a = """\nsay \\"hi\\"\n"""\n').a).toBe('say "hi"\n');
  });
});

describe("parseToml structure", () => {
  it("parses tables, nested tables and dotted keys", () => {
    const t = parseToml(
      ["[server]", 'host = "x.dev"', "[server.tls]", "enabled = true", "a.b.c = 1"].join("\n"),
    );
    expect(t).toEqual({
      server: { host: "x.dev", tls: { enabled: true, a: { b: { c: 1 } } } },
    });
  });

  it("parses arrays of tables and keeps order", () => {
    const t = parseToml(
      ["[[tests]]", 'kind = "status"', "value = 200", "", "[[tests]]", 'kind = "duration"'].join(
        "\n",
      ),
    );
    expect(t.tests).toEqual([{ kind: "status", value: 200 }, { kind: "duration" }]);
  });

  it("routes sub-tables into the last array-of-tables element", () => {
    const t = parseToml(
      ["[[a]]", "n = 1", "[a.deep]", "x = 1", "[[a]]", "n = 2", "[a.deep]", "x = 2"].join("\n"),
    );
    expect(t.a).toEqual([
      { n: 1, deep: { x: 1 } },
      { n: 2, deep: { x: 2 } },
    ]);
  });

  it("parses nested, multi-line and trailing-comma arrays", () => {
    const t = parseToml(
      [
        "a = [1, 2, 3]",
        "b = [",
        '  ["x", "y"],  # a pair',
        '  ["z", "w"],',
        "]",
        "c = []",
        "d = [[1, [2]], []]",
      ].join("\n"),
    );
    expect(t.a).toEqual([1, 2, 3]);
    expect(t.b).toEqual([
      ["x", "y"],
      ["z", "w"],
    ]);
    expect(t.c).toEqual([]);
    expect(t.d).toEqual([[1, [2]], []]);
  });

  it("parses inline tables including nested and dotted forms", () => {
    const t = parseToml('a = { x = 1, y = "two", z = { deep = true } }\nb = { p.q = 5 }\nc = {}\n');
    expect(t.a).toEqual({ x: 1, y: "two", z: { deep: true } });
    expect(t.b).toEqual({ p: { q: 5 } });
    expect(t.c).toEqual({});
  });

  it("ignores comments and blank lines everywhere", () => {
    const t = parseToml(
      ["# leading", "", "a = 1 # trailing", "  # indented", "[t] # after header", "b = 2"].join(
        "\n",
      ),
    );
    expect(t).toEqual({ a: 1, t: { b: 2 } });
  });

  it("accepts CRLF line endings", () => {
    expect(parseToml('a = 1\r\n[t]\r\nb = "x"\r\n')).toEqual({ a: 1, t: { b: "x" } });
  });

  it("accepts an empty document", () => {
    expect(parseToml("")).toEqual({});
    expect(parseToml("# only a comment\n")).toEqual({});
  });
});

describe("parseToml fails loud", () => {
  const bad = (text: string) => () => parseToml(text);

  it("reports the line number", () => {
    expect(bad("a = 1\nb = 2\nc = 2\nc = 3\n")).toThrow(/line 4/);
  });

  it("rejects duplicate keys", () => {
    expect(bad('a = 1\na = 2\n')).toThrow(/duplicate key: a/);
    expect(bad("[t]\nx = 1\nx = 2\n")).toThrow(/duplicate key: x/);
    expect(bad("a.b = 1\na.b = 2\n")).toThrow(/duplicate key: a\.b/);
    expect(bad("a = { x = 1, x = 2 }\n")).toThrow(/duplicate key in inline table: x/);
  });

  it("rejects redefined tables", () => {
    expect(bad("[t]\na = 1\n[t]\nb = 2\n")).toThrow(/table t is defined more than once/);
    expect(bad("[a.b]\nx = 1\n[a.b]\n")).toThrow(/table a\.b is defined more than once/);
    expect(bad("a.b = 1\n[a]\n")).toThrow(/dotted key/);
    expect(bad("[[t]]\n[t]\n")).toThrow(/array of tables/);
    expect(bad("[t]\n[[t]]\n")).toThrow(/not an array of tables/);
  });

  it("rejects a table that collides with a value", () => {
    expect(bad('a = "x"\n[a]\n')).toThrow(/already defined as a value/);
    expect(bad('a = "x"\n[a.b]\n')).toThrow(/non-table value/);
    expect(bad("[t]\nx = 1\nx.y = 2\n")).toThrow(/redefines an existing value/);
  });

  it("rejects unterminated strings", () => {
    expect(bad('a = "oops\nb = 1\n')).toThrow(/unterminated string/);
    expect(bad("a = 'oops\n")).toThrow(/unterminated literal string/);
    expect(bad('a = """oops\n')).toThrow(/unterminated multi-line string/);
    expect(bad("a = '''oops\n")).toThrow(/unterminated multi-line literal string/);
  });

  it("rejects unterminated arrays and inline tables", () => {
    expect(bad("a = [1, 2\n")).toThrow(/unterminated array/);
    expect(bad("a = { x = 1\n")).toThrow(/unterminated inline table/);
    expect(bad("a = { x = 1, }\n")).toThrow(/trailing comma/);
  });

  it("rejects invalid escapes", () => {
    expect(bad('a = "\\q"\n')).toThrow(/invalid escape/);
    expect(bad('a = "\\uZZZZ"\n')).toThrow(/invalid unicode escape/);
  });

  it("rejects malformed statements", () => {
    expect(bad("a\n")).toThrow(/expected '=' after key a/);
    expect(bad("= 1\n")).toThrow(/expected a key/);
    expect(bad("[t\n")).toThrow(/expected ']'/);
    expect(bad("[[t]\n")).toThrow(/expected ']]'/);
    expect(bad("a = 1 b = 2\n")).toThrow(/unexpected trailing text/);
    expect(bad("a = ?\n")).toThrow(/expected a value/);
    expect(bad("a =\n")).toThrow(/expected a value/);
  });
});

describe("stringifyToml", () => {
  const roundTrip = (v: Record<string, unknown>) =>
    parseToml(stringifyToml(v as never));

  it("emits scalars before tables and arrays of tables", () => {
    const text = stringifyToml({
      t: { x: 1 },
      list: [{ n: 1 }, { n: 2 }],
      a: 1,
      b: "two",
    });
    const order = text
      .split("\n")
      .filter((l) => l.length > 0)
      .map((l) => l.split(" ")[0]);
    expect(order).toEqual(["a", "b", "[t]", "x", "[[list]]", "n", "[[list]]", "n"]);
    expect(parseToml(text)).toEqual({ a: 1, b: "two", t: { x: 1 }, list: [{ n: 1 }, { n: 2 }] });
  });

  it("emits multi-line strings for values containing newlines", () => {
    const text = stringifyToml({ s: 'a\nb "quoted" \\ c\n' });
    expect(text.startsWith('s = """\n')).toBe(true);
    expect(parseToml(text).s).toBe('a\nb "quoted" \\ c\n');
  });

  it("escapes control characters and quotes in basic strings", () => {
    const text = stringifyToml({ s: 'tab\there "q" \\ back\u0000' });
    expect(text).toContain("\\t");
    expect(text).toContain("\\u0000");
    expect(parseToml(text).s).toBe('tab\there "q" \\ back\u0000');
  });

  it("quotes keys that are not bare", () => {
    const text = stringifyToml({ "a b": 1, "c.d": { e: 2 } });
    expect(text).toContain('"a b" = 1');
    expect(text).toContain('["c.d"]');
    expect(parseToml(text)).toEqual({ "a b": 1, "c.d": { e: 2 } });
  });

  it("round-trips nested arrays, empty arrays and inline tables inside arrays", () => {
    const value = {
      pairs: [
        ["a", "b"],
        ["c", "d"],
      ],
      empty: [],
      mixed: [1, "two", true, [3]],
      objs: [{ a: 1 }],
    };
    expect(roundTrip(value)).toEqual(value);
  });

  it("skips null and undefined values", () => {
    expect(stringifyToml({ a: 1, b: null, c: undefined } as never)).toBe("a = 1\n");
  });

  it("round-trips deeply nested tables", () => {
    const value = { a: { b: { c: { d: "deep" } } } };
    expect(roundTrip(value)).toEqual(value);
  });

  it("emits an empty document for an empty table", () => {
    expect(stringifyToml({})).toBe("");
  });
});

const fullRequest: SavedRequest = {
  id: "01H8-abc_DEF",
  name: "Create User",
  kind: "http",
  method: "POST",
  url: "{{base}}/users",
  description: "Creates a user and\ncaptures the token",
  body: '{\n  "name": "nova",\n  "note": "he said \\"hi\\"",\n  "tabbed": "\\t"\n}\n',
  headers: [
    ["Accept", "application/json"],
    ["Content-Type", "application/json"],
  ],
  auth: { type: "bearer", token: "{{token}}" },
  graphql: null,
  grpc: {
    protoPaths: ["/protos/echo.proto", "/protos/other.proto"],
    service: "test.v1.Echo",
    method: "Say",
    message: '{\n  "text": "hi"\n}',
    metadata: [
      ["x-trace", "1"],
      ["x-tenant", "acme"],
    ],
  },
  scripts: {
    pre: 'pm.environment.set("a", 1)\nif (a > 1) {\n  console.log("big \\"one\\"")\n}',
    post: "console.log(pm.response.code)\nconsole.log('done')",
  },
  tests: [
    { kind: "status", op: "eq", value: 201 },
    { kind: "json", path: "$.token", op: "exists" },
    { kind: "json", path: "$.count", op: "gt", value: 3 },
    { kind: "header", name: "Content-Type", op: "contains", value: "json" },
    { kind: "header", name: "X-Legacy", op: "absent" },
    { kind: "duration", op: "lt", value: 1000 },
  ],
  captures: [{ from: "body.$.token", into: "token", scope: "session" }],
};

describe("request codec", () => {
  it("round-trips a realistic request through TOML", () => {
    const text = encodeRequest(fullRequest);
    expect(decodeRequest(text)).toEqual(fullRequest);
    expect(encodeRequest(decodeRequest(text))).toBe(text);
  });

  it("writes the multi-line body and scripts as multi-line strings", () => {
    const text = encodeRequest(fullRequest);
    expect(text).toContain('body = """\n');
    expect(text).toContain('pre = """\n');
    expect(text).toContain('post = """\n');
  });

  it("keeps the header ordering the Rust parser expects", () => {
    const text = encodeRequest(fullRequest);
    const scalars = text.slice(0, text.indexOf("["));
    expect(scalars).toContain("id = ");
    expect(text.indexOf("[auth]")).toBeLessThan(text.indexOf("[[tests]]"));
    expect(text.indexOf("[grpc]")).toBeLessThan(text.indexOf("[[tests]]"));
    expect(text.indexOf("[[tests]]")).toBeLessThan(text.indexOf("[[captures]]"));
  });

  it("round-trips a graphql request", () => {
    const gql: SavedRequest = {
      id: "gql-1",
      name: "Gql",
      kind: "graphql",
      method: "POST",
      url: "https://x.dev/graphql",
      description: null,
      body: null,
      headers: [],
      auth: { type: "apikey", key: "X-Api-Key", value: "{{k}}", placement: "header" },
      graphql: {
        query: "query User($id: ID!) {\n  user(id: $id) {\n    name\n  }\n}",
        variables: '{\n  "id": "7"\n}',
      },
      grpc: null,
      scripts: { pre: null, post: null },
      tests: [],
      captures: [],
    };
    const text = encodeRequest(gql);
    expect(decodeRequest(text)).toEqual(gql);
    expect(encodeRequest(decodeRequest(text))).toBe(text);
  });

  it("round-trips every auth variant", () => {
    const auths: SavedRequest["auth"][] = [
      { type: "none" },
      { type: "bearer", token: "t" },
      { type: "basic", username: "u", password: "p" },
      { type: "apikey", key: "k", value: "v", placement: "query" },
    ];
    for (const auth of auths) {
      const req: SavedRequest = { ...fullRequest, auth };
      expect(decodeRequest(encodeRequest(req)).auth).toEqual(auth);
    }
  });

  it("omits absent optionals rather than writing empty strings", () => {
    const minimal: SavedRequest = {
      id: "a",
      name: "Ping",
      kind: "http",
      method: "GET",
      url: "https://x.dev",
      headers: [],
      auth: { type: "none" },
    };
    const text = encodeRequest(minimal);
    expect(text).not.toContain("body =");
    expect(text).not.toContain("description =");
    expect(text).not.toContain("[graphql]");
    expect(text).not.toContain("[grpc]");
    expect(text).not.toContain("[scripts]");
    expect(text).not.toContain("[[tests]]");
    expect(decodeRequest(text)).toEqual({
      ...minimal,
      description: null,
      body: null,
      graphql: null,
      grpc: null,
      scripts: { pre: null, post: null },
      tests: [],
      captures: [],
    });
  });

  it("applies serde defaults for every missing optional section", () => {
    const text = [
      'id = "a"',
      'name = "Ping"',
      'kind = "http"',
      'method = "GET"',
      'url = "https://x.dev"',
    ].join("\n");
    expect(decodeRequest(text)).toEqual({
      id: "a",
      name: "Ping",
      kind: "http",
      method: "GET",
      url: "https://x.dev",
      description: null,
      body: null,
      headers: [],
      auth: { type: "none" },
      graphql: null,
      grpc: null,
      scripts: { pre: null, post: null },
      tests: [],
      captures: [],
    });
  });

  it("reads a file written the way the Rust toml crate writes it", () => {
    const text = [
      'id = "abc"',
      'name = "List Users"',
      'kind = "http"',
      'method = "GET"',
      'url = "https://x.dev/users"',
      'headers = [["Accept", "application/json"]]',
      "",
      "[auth]",
      'type = "bearer"',
      'token = "{{token}}"',
      "",
      "[scripts]",
      'pre = "console.log(1)"',
      "",
      "[[tests]]",
      'kind = "status"',
      'op = "eq"',
      "value = 200",
      "",
      "[[captures]]",
      'from = "body.$.token"',
      'into = "token"',
      'scope = "persist"',
      "",
    ].join("\n");
    const req = decodeRequest(text);
    expect(req.headers).toEqual([["Accept", "application/json"]]);
    expect(req.auth).toEqual({ type: "bearer", token: "{{token}}" });
    expect(req.scripts).toEqual({ pre: "console.log(1)", post: null });
    expect(req.tests).toEqual([{ kind: "status", op: "eq", value: 200 }]);
    expect(req.captures).toEqual([{ from: "body.$.token", into: "token", scope: "persist" }]);
  });

  it("round-trips a json assertion with a string, boolean and array value", () => {
    const req: SavedRequest = {
      ...fullRequest,
      tests: [
        { kind: "json", path: "$.a", op: "eq", value: "text" },
        { kind: "json", path: "$.b", op: "eq", value: true },
        { kind: "json", path: "$.c", op: "contains", value: [1, 2] },
      ],
    };
    expect(decodeRequest(encodeRequest(req)).tests).toEqual(req.tests);
  });
});

describe("request decoder fails loud", () => {
  const base = [
    'id = "a"',
    'name = "Ping"',
    'kind = "http"',
    'method = "GET"',
    'url = "https://x.dev"',
  ];
  const withExtra = (...lines: string[]) => base.concat(lines).join("\n");

  it("rejects a missing or non-string required field", () => {
    expect(() => decodeRequest('name = "x"\nkind = "http"\nmethod = "GET"\nurl = "u"')).toThrow(
      /missing required key "id"/,
    );
    expect(() => decodeRequest('id = 7\nname = "x"\nkind = "http"\nmethod = "G"\nurl = "u"')).toThrow(
      /"id" must be a string/,
    );
    expect(() => decodeRequest('id = "a"\nname = "x"\nkind = "http"\nmethod = "G"')).toThrow(
      /missing required key "url"/,
    );
  });

  it("rejects an unknown kind naming the value", () => {
    expect(() =>
      decodeRequest('id = "a"\nname = "x"\nkind = "soap"\nmethod = "G"\nurl = "u"'),
    ).toThrow(/unknown value "soap" \(expected http, graphql, grpc\)/);
  });

  it("rejects malformed headers", () => {
    expect(() => decodeRequest(withExtra('headers = "Accept: json"'))).toThrow(
      /headers must be an array/,
    );
    expect(() => decodeRequest(withExtra('headers = [["a", "b", "c"]]'))).toThrow(
      /headers\[0\] must be a two-element array of strings/,
    );
    expect(() => decodeRequest(withExtra('headers = [["a", 1]]'))).toThrow(/headers\[0\]/);
    expect(() => decodeRequest(withExtra('headers = ["a"]'))).toThrow(/headers\[0\]/);
    expect(() =>
      decodeRequest(withExtra("[grpc]", 'service = "s"', 'method = "m"', 'message = "{}"', "metadata = [[1, 2]]")),
    ).toThrow(/grpc\.metadata\[0\]/);
  });

  it("rejects a bad auth type and missing auth fields", () => {
    expect(() => decodeRequest(withExtra("[auth]", 'type = "oauth3"'))).toThrow(
      /unknown type "oauth3" \(expected none, bearer, basic, apikey\)/,
    );
    expect(() => decodeRequest(withExtra("[auth]", 'type = "bearer"'))).toThrow(
      /auth: missing required key "token"/,
    );
    expect(() => decodeRequest(withExtra("[auth]", 'type = "basic"', 'username = "u"'))).toThrow(
      /auth: missing required key "password"/,
    );
    expect(() =>
      decodeRequest(
        withExtra("[auth]", 'type = "apikey"', 'key = "k"', 'value = "v"', 'placement = "cookie"'),
      ),
    ).toThrow(/unknown value "cookie" \(expected header, query\)/);
  });

  it("rejects unknown assertion kinds and ops", () => {
    expect(() => decodeRequest(withExtra("[[tests]]", 'kind = "vibes"'))).toThrow(
      /unknown assertion kind "vibes"/,
    );
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "status"', 'op = "approx"', "value = 200")),
    ).toThrow(/tests\[0\]\.op: unknown value "approx" \(expected eq, ne, lt, gt\)/);
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "json"', 'path = "$.a"', 'op = "startswith"')),
    ).toThrow(/unknown value "startswith"/);
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "header"', 'name = "A"', 'op = "len"')),
    ).toThrow(/unknown value "len"/);
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "duration"', 'op = "eq"', "value = 1")),
    ).toThrow(/unknown value "eq" \(expected lt, gt\)/);
  });

  it("requires numeric values for status and duration", () => {
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "status"', 'op = "eq"', 'value = "200"')),
    ).toThrow(/"value" must be an integer/);
    expect(() =>
      decodeRequest(withExtra("[[tests]]", 'kind = "duration"', 'op = "lt"')),
    ).toThrow(/missing required key "value"/);
  });

  it("accepts an optional value on json and header assertions", () => {
    const req = decodeRequest(
      withExtra("[[tests]]", 'kind = "json"', 'path = "$.a"', 'op = "exists"'),
    );
    expect(req.tests).toEqual([{ kind: "json", path: "$.a", op: "exists" }]);
  });

  it("rejects a bad capture scope", () => {
    expect(() =>
      decodeRequest(
        withExtra("[[captures]]", 'from = "status"', 'into = "s"', 'scope = "forever"'),
      ),
    ).toThrow(/captures\[0\]\.scope: unknown value "forever" \(expected run, session, persist\)/);
    expect(() =>
      decodeRequest(withExtra("[[captures]]", 'from = "status"', 'scope = "run"')),
    ).toThrow(/captures\[0\]: missing required key "into"/);
  });

  it("rejects a malformed section type", () => {
    expect(() => decodeRequest(withExtra('auth = "bearer"'))).toThrow(/auth must be a table/);
    expect(() => decodeRequest(withExtra('scripts = "x"'))).toThrow(/scripts must be a table/);
    expect(() => decodeRequest(withExtra('tests = "x"'))).toThrow(/tests must be an array/);
    expect(() => decodeRequest(withExtra('graphql = "x"'))).toThrow(/"graphql" must be a table/);
    expect(() => decodeRequest(withExtra("[graphql]", 'query = "{ ping }"'))).toThrow(
      /graphql: missing required key "variables"/,
    );
  });

  it("propagates parser errors with a line number", () => {
    expect(() => decodeRequest('id = "a"\nid = "b"\n')).toThrow(/TOML line 2: duplicate key: id/);
  });

  it("rejects encoding an unknown kind", () => {
    expect(() => encodeRequest({ ...fullRequest, kind: "soap" as never })).toThrow(
      /unknown value "soap"/,
    );
  });
});

describe("environment codec", () => {
  it("round-trips plain declarations", () => {
    const doc = {
      name: "staging",
      vars: {
        base: { secret: false as const, value: "https://staging.x.dev" },
        "odd key": { secret: false as const, value: "a\nb" },
      },
    };
    const text = encodeEnvDoc(doc);
    expect(text).toContain("schema_version = 1");
    expect(text).toContain('name = "staging"');
    expect(text).toContain("[vars.base]");
    expect(decodeEnvDoc("staging", text)).toEqual(doc);
    expect(encodeEnvDoc(decodeEnvDoc("staging", text))).toBe(text);
  });

  it("writes a secret declaration the way the Rust core does", () => {
    const text = encodeEnvDoc({
      name: "prod",
      vars: {
        access_token: { secret: true, hosts: ["api.acme.com"] },
        base: { secret: false, value: "https://api.acme.com" },
      },
    });
    expect(text).toBe(
      'schema_version = 1\nname = "prod"\n\n[vars.access_token]\nsecret = true\nhosts = ["api.acme.com"]\n\n[vars.base]\nvalue = "https://api.acme.com"\n',
    );
  });

  it("never lets a secret carry a value", () => {
    expect(() =>
      decodeEnvDoc("prod", '[vars.token]\nsecret = true\nvalue = "leaked"\n'),
    ).toThrow(/declared secret = true and also carries a value/);
  });

  it("rejects a host-bound variable that is not a secret", () => {
    expect(() =>
      decodeEnvDoc("prod", '[vars.token]\nvalue = "v"\nhosts = ["a.dev"]\n'),
    ).toThrow(/binds hosts but is not declared secret/);
  });

  it("rejects a declaration with neither a value nor secret = true", () => {
    expect(() => decodeEnvDoc("prod", "[vars.token]\n")).toThrow(
      /neither a value nor secret = true/,
    );
  });

  it("reads the legacy flat shape as a plain declaration", () => {
    expect(
      decodeEnvDoc("prod", 'name = "prod"\n\n[vars]\nbase = "https://x.dev"\n'),
    ).toEqual({
      name: "prod",
      vars: { base: { secret: false, value: "https://x.dev" } },
    });
  });

  it("lowercases bound hosts", () => {
    const doc = decodeEnvDoc(
      "prod",
      '[vars.t]\nsecret = true\nhosts = ["API.Acme.COM"]\n',
    );
    expect(doc.vars.t).toEqual({ secret: true, hosts: ["api.acme.com"] });
  });

  it("falls back to the file stem when the file carries no name", () => {
    expect(decodeEnvDoc("staging", '[vars]\na = "1"\n')).toEqual({
      name: "staging",
      vars: { a: { secret: false, value: "1" } },
    });
  });

  it("defaults vars to an empty map", () => {
    expect(decodeEnvDoc("empty", 'name = "empty"\n')).toEqual({
      name: "empty",
      vars: {},
    });
  });

  it("rejects an unsupported schema version", () => {
    expect(() => decodeEnvDoc("x", 'schema_version = 9\nname = "x"\n')).toThrow(
      /unsupported environment schema_version 9/,
    );
  });

  it("rejects vars that are neither a string nor a table", () => {
    expect(() => decodeEnvDoc("x", "[vars]\nport = 8080\n")).toThrow(
      /vars\.port must be a string or a table/,
    );
    expect(() => decodeEnvDoc("x", 'vars = "nope"\n')).toThrow(
      /"vars" must be a table/,
    );
  });
});

describe("manifest codec", () => {
  it("uses snake_case on the wire", () => {
    const text = encodeCollectionManifest({ schemaVersion: 1, id: "abc", name: "Acme API" });
    expect(text).toBe('schema_version = 1\nid = "abc"\nname = "Acme API"\n');
    expect(decodeManifest(text)).toEqual({ schemaVersion: 1, id: "abc", name: "Acme API" });
  });

  it("writes a workspace manifest", () => {
    expect(encodeWorkspaceManifest({ schemaVersion: 1, name: "Personal" })).toBe(
      'schema_version = 1\nname = "Personal"\n',
    );
    expect(encodeWorkspaceManifest({ schemaVersion: 1, id: "w1", name: "Personal" })).toBe(
      'schema_version = 1\nid = "w1"\nname = "Personal"\n',
    );
  });

  it("reads a manifest without an id", () => {
    expect(decodeManifest('schema_version = 2\nname = "X"\n')).toEqual({
      schemaVersion: 2,
      name: "X",
    });
  });

  it("fails loud on a missing or malformed schema_version", () => {
    expect(() => decodeManifest('name = "X"\n')).toThrow(
      /manifest: missing required key "schema_version"/,
    );
    expect(() => decodeManifest('schema_version = "1"\nname = "X"\n')).toThrow(
      /"schema_version" must be an integer/,
    );
    expect(() => decodeManifest("schema_version = 1\n")).toThrow(
      /manifest: missing required key "name"/,
    );
  });
});
