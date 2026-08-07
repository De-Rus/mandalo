import { describe, expect, it } from "vitest";
import { evaluateTests, resolveCapture, type ResponseFacts } from "../../src/engine/assert";
import { apply, canonicalJson, UnresolvedVarError } from "../../src/engine/interpolate";
import { query, UnsupportedJsonPathError } from "../../src/engine/jsonpath";
import { prepare } from "../../src/engine/prepare";
import { runOne } from "../../src/engine/run";
import { dynamicNames, executeScript } from "../../src/engine/sandbox";
import type { RequestModel } from "../../../src/lib/format/model";

const BODY = '{"id": 7, "name": "nova", "tags": ["a", "b"], "nested": {"ok": true}, "empty": null}';

function facts(overrides: Partial<ResponseFacts> = {}): ResponseFacts {
  return {
    status: 200,
    headers: [
      ["content-type", "application/json"],
      ["x-dup", "one"],
      ["x-dup", "two"],
    ],
    body: BODY,
    durationMs: 12,
    ...overrides,
  };
}

function model(overrides: Partial<RequestModel> = {}): RequestModel {
  return {
    schemaVersion: 1,
    id: "id",
    name: "Name",
    kind: "http",
    method: "GET",
    url: "http://host/p",
    headers: [],
    auth: { type: "none" },
    scripts: {},
    tests: [],
    captures: [],
    ...overrides,
  };
}

function wire(model: RequestModel) {
  return {
    method: model.method,
    url: model.url,
    headers: model.headers,
    body: model.body ?? null,
  };
}

describe("interpolate", () => {
  it("substitutes and trims the key", () => {
    expect(apply("a{{ x }}b", { x: "1" })).toBe("a1b");
  });

  it("names the unresolved variable", () => {
    expect(() => apply("{{who}}", {})).toThrow(UnresolvedVarError);
    expect(() => apply("{{who}}", {})).toThrow("unresolved variable: who");
  });

  it("rejects an unclosed opener", () => {
    expect(() => apply("{{x", { x: "1" })).toThrow(/unclosed \{\{/);
  });

  it("sorts object keys the way serde_json does", () => {
    expect(canonicalJson({ b: 1, a: { d: 2, c: 3 } })).toBe('{"a":{"c":3,"d":2},"b":1}');
  });
});

describe("jsonpath", () => {
  const json = JSON.parse(BODY) as unknown;

  it("reads children, indexes and wildcards", () => {
    expect(query("$.id", json)).toEqual([7]);
    expect(query("$.tags[0]", json)).toEqual(["a"]);
    expect(query("$.tags[-1]", json)).toEqual(["b"]);
    expect(query("$.tags[*]", json)).toEqual(["a", "b"]);
    expect(query("$.nested.ok", json)).toEqual([true]);
    expect(query("$['name']", json)).toEqual(["nova"]);
  });

  it("descends recursively", () => {
    expect(query("$..ok", json)).toEqual([true]);
  });

  it("yields nothing for a missing path rather than throwing", () => {
    expect(query("$.nope.deeper", json)).toEqual([]);
  });

  it("refuses selectors it cannot evaluate faithfully", () => {
    expect(() => query("$.tags[?(@.x)]", json)).toThrow(UnsupportedJsonPathError);
    expect(() => query("$.tags[0:2]", json)).toThrow(UnsupportedJsonPathError);
    expect(() => query("$.tags[0,1]", json)).toThrow(UnsupportedJsonPathError);
  });
});

describe("assertions", () => {
  it("matches the Rust names and details", () => {
    const results = evaluateTests(
      [
        { kind: "status", op: "eq", value: 404 },
        { kind: "duration", op: "gt", value: 9999 },
        { kind: "json", path: "$.name", op: "eq", value: "other" },
        { kind: "header", name: "x-dup", op: "eq", value: "two" },
        { kind: "header", name: "x-none", op: "exists" },
      ],
      facts(),
    );
    expect(results).toEqual([
      { name: "status eq 404", passed: false, detail: "status was 200" },
      { name: "duration gt 9999ms", passed: false, detail: "took 12ms" },
      { name: "json $.name eq other", passed: false, detail: '$.name is "nova"' },
      { name: "header x-dup eq two", passed: true, detail: null },
      { name: "header x-none exists", passed: false, detail: "header x-none is absent" },
    ]);
  });

  it("reports a non-JSON body once per request", () => {
    const results = evaluateTests(
      [
        { kind: "json", path: "$.a", op: "exists" },
        { kind: "json", path: "$.b", op: "exists" },
      ],
      facts({ body: "not json" }),
    );
    expect(results.every((result) => result.detail?.startsWith("response body is not JSON: "))).toBe(
      true,
    );
  });
});

describe("captures", () => {
  it("reads status, headers and body paths", () => {
    expect(resolveCapture("status", facts())).toBe("200");
    expect(resolveCapture("header.X-Dup", facts())).toBe("one");
    expect(resolveCapture("body.$.name", facts())).toBe("nova");
    expect(resolveCapture("body.$.id", facts())).toBe("7");
    expect(resolveCapture("header.absent", facts())).toBeNull();
  });

  it("rejects a malformed source the way the Rust parser does", () => {
    expect(() => resolveCapture("nonsense", facts())).toThrow(/invalid capture source/);
    expect(() => resolveCapture("body.name", facts())).toThrow(/must start with \$/);
  });
});

describe("prepare", () => {
  it("drops a user Authorization header when auth is bearer", () => {
    const request = model({
      headers: [["Authorization", "stale"]],
      auth: { type: "bearer", token: "{{t}}" },
    });
    const prepared = prepare(request, wire(request), { t: "abc" });
    expect(prepared.headers).toEqual([["Authorization", "Bearer abc"]]);
  });

  it("base64-encodes basic auth", () => {
    const request = model({ auth: { type: "basic", username: "u", password: "p" } });
    expect(prepare(request, wire(request), {}).headers).toEqual([
      ["Authorization", `Basic ${Buffer.from("u:p").toString("base64")}`],
    ]);
  });

  it("appends an apikey to the query string", () => {
    const request = model({
      url: "http://host/p?a=1",
      auth: { type: "apikey", key: "k e y", value: "v", placement: "query" },
    });
    expect(prepare(request, wire(request), {}).url).toBe("http://host/p?a=1&k+e+y=v");
  });

  it("labels a raw body with the type the Rust builder sniffs for it", () => {
    const cases: [string, string][] = [
      ["raw", "text/plain"],
      ['{"a": 1}', "application/json"],
      ["[1, 2]", "application/json"],
      ['<?xml version="1.0"?><a/>', "application/xml"],
      ["<user id=\"1\"/>", "application/xml"],
      ["<!doctype html><p>hi", "text/html"],
    ];
    for (const [body, contentType] of cases) {
      const request = model({ method: "POST", body });
      const prepared = prepare(request, wire(request), {});
      expect(prepared.headers, body).toEqual([["Content-Type", contentType]]);
      expect(prepared.body).toBe(body);
    }
  });

  it("replaces an Authorization header rather than sending both", () => {
    const request = model({
      headers: [["Authorization", "stale"]],
      auth: { type: "bearer", token: "fresh" },
    });
    expect(prepare(request, wire(request), {}).headers).toEqual([
      ["Authorization", "Bearer fresh"],
    ]);

    const basic = model({
      headers: [["authorization", "stale"]],
      auth: { type: "basic", username: "u", password: "p" },
    });
    expect(prepare(basic, wire(basic), {}).headers).toEqual([
      ["Authorization", `Basic ${Buffer.from("u:p").toString("base64")}`],
    ]);
  });

  it("lets a declared Content-Type win over the sniffed one", () => {
    const request = model({
      method: "POST",
      body: '{"a": 1}',
      headers: [["Content-Type", "application/vnd.acme+json"]],
    });
    expect(prepare(request, wire(request), {}).headers).toEqual([
      ["Content-Type", "application/vnd.acme+json"],
    ]);
  });

  it("builds a graphql body with sorted keys and adds the JSON content type", () => {
    const request = model({
      kind: "graphql",
      graphql: { query: "query Q { a }", variables: '{"who":"{{n}}"}' },
    });
    const prepared = prepare(request, wire(request), { n: "nova" });
    expect(prepared.method).toBe("POST");
    expect(prepared.body).toBe('{"query":"query Q { a }","variables":{"who":"nova"}}');
    expect(prepared.headers).toEqual([["Content-Type", "application/json"]]);
  });

  it("rejects a method the HTTP layer would not accept", () => {
    const request = model({ method: "FETCH" });
    expect(() => prepare(request, wire(request), {})).toThrow("invalid method: FETCH");
  });
});

describe("script sandbox", () => {
  const context = {
    vars: { seed: "s" },
    requestName: "R",
    request: { method: "GET", url: "http://host", headers: [] as [string, string][], body: null },
    response: null,
  };

  it("runs the real pm prelude", async () => {
    const outcome = await executeScript(
      "pm.test('a', function () { pm.expect(pm.variables.get('seed')).to.equal('s'); }); pm.variables.set('out', '1'); console.log('x');",
      context,
    );
    expect(outcome.tests).toEqual([{ name: "a", passed: true, error: null }]);
    expect(outcome.varSets["out"]).toBe("1");
    expect(outcome.logs).toEqual(["x"]);
  });

  it("blocks the host globals the Rust engine blocks, with the same reason", async () => {
    await expect(executeScript("require('fs')", context)).rejects.toThrow(
      "require is not available in Mándalo scripts: there is no module loader; scripts cannot import code",
    );
    await expect(executeScript("process.exit(0)", context)).rejects.toThrow(
      /process is not available in Mándalo scripts/,
    );
    await expect(executeScript("fetch('http://x')", context)).rejects.toThrow(
      /fetch is not available in Mándalo scripts/,
    );
  });

  it("kills a runaway script on the shared 2s budget", async () => {
    await expect(executeScript("while (true) {}", context)).rejects.toThrow("script exceeded 2000ms");
  }, 20_000);

  it("captures a request patch from a pre-request script", async () => {
    const outcome = await executeScript("pm.request.headers.add({key:'X-A', value:'1'})", context);
    expect(outcome.requestPatch?.headers).toEqual([["X-A", "1"]]);
  });

  it("finds the dynamic variables a template needs", () => {
    expect(dynamicNames(["{{$guid}}/{{plain}}", "{{ $randomInt }}"])).toEqual([
      "$guid",
      "$randomInt",
    ]);
  });
});

describe("what the in-process engine refuses to guess at", () => {
  it("escalates a form-data body instead of building multipart itself", async () => {
    const request = model({
      formdata: [
        { key: "title", value: "Q3 expenses" },
        { key: "attachments", files: ["files/alpha.txt"] },
      ],
    });
    await expect(
      runOne({ model: request, relPath: "form.http#0" }, "api", null, {}),
    ).rejects.toThrow(/multipart form-data body reads files off the workspace/);
  });
});
