import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { RequestSpec, ScriptOutcome, ScriptTest } from "../api";
import fixtures from "./parity.fixtures.json";
import { parseTextDocument, withFileVars } from "../format/httpFormat";
import { prepare, prepareRequest } from "./send";
import { webExecuteScript } from "./script";
import { useGeneratedWorker } from "./worker.testkit";

/**
 * The browser half of the two-engine contract. Every expectation lives in
 * parity.fixtures.json and was captured off the socket while the Rust CLI sent
 * the same request — `node src/lib/web/parity-derive.mjs` re-runs that capture
 * and is what keeps these numbers honest.
 */

const ORIGIN = "http://127.0.0.1:7788";
const VARS: Record<string, string> = fixtures.vars;

interface Wire {
  method: string;
  path: string;
  headers: [string, string][];
  body: string | null;
}

interface Fixture {
  name: string;
  spec: RequestSpec;
  wire: Wire;
  http?: string;
  pre?: string;
  scriptTests?: [string, boolean][];
}

function withBase<T>(value: T): T {
  return JSON.parse(JSON.stringify(value).split("{{BASE}}").join(ORIGIN)) as T;
}

const FIXTURES: Fixture[] = fixtures.fixtures.map((fixture) => ({
  name: fixture.name,
  spec: { ...withBase(fixture.spec), vars: VARS } as unknown as RequestSpec,
  wire: withBase(fixture.wire) as Wire,
  http: (fixture as { http?: string }).http,
  pre: (fixture as { pre?: string }).pre,
  scriptTests: (fixture as { scriptTests?: [string, boolean][] }).scriptTests,
}));

let restoreWorker: () => void;

beforeAll(() => {
  restoreWorker = useGeneratedWorker();
});

afterAll(() => {
  restoreWorker();
});

const PLUMBING = new Set(["accept", "accept-encoding", "connection", "content-length", "host"]);

function comparable(headers: [string, string][]): [string, string][] {
  return headers
    .map(([key, value]): [string, string] => [key.toLowerCase(), value])
    .filter(([key]) => !PLUMBING.has(key))
    .sort((a, b) => a[0].localeCompare(b[0]));
}

function matches(actual: string | null, expected: string | null, what: string): void {
  if (expected === null) {
    expect(actual, what).toBeNull();
    return;
  }
  if (expected.startsWith("~")) expect(actual ?? "", what).toMatch(new RegExp(expected.slice(1)));
  else expect(actual, what).toBe(expected);
}

function assertWire(actual: Wire, wire: Wire, who: string): void {
  expect(actual.method, `${who}: method`).toBe(wire.method);
  matches(actual.path, wire.path, `${who}: path`);
  const headers = comparable(actual.headers);
  expect(
    headers.map(([key]) => key),
    `${who}: header names`,
  ).toEqual(wire.headers.map(([key]) => key));
  headers.forEach(([key, value], index) => {
    matches(value, wire.headers[index][1], `${who}: header ${key}`);
  });
  matches(actual.body, wire.body, `${who}: body`);
}

async function browserWire(
  fixture: Fixture,
): Promise<{ wire: Wire; tests: ScriptTest[]; outcome: ScriptOutcome | null }> {
  let spec = fixture.spec;
  let outcome: ScriptOutcome | null = null;
  if (fixture.pre !== undefined) {
    outcome = await webExecuteScript(fixture.pre, {
      vars: { ...VARS },
      requestName: fixture.name,
      request: { method: spec.method, url: spec.url, headers: spec.headers, body: spec.body },
      response: null,
    });
    const patch = outcome.requestPatch;
    if (patch)
      spec = {
        ...spec,
        method: patch.method,
        url: patch.url,
        headers: patch.headers,
        body: patch.body,
      };
  }
  const prepared = await prepareRequest(spec);
  const url = new URL(prepared.url);
  return {
    wire: {
      method: prepared.method,
      path: `${url.pathname}${url.search}`,
      headers: prepared.headers,
      body: prepared.body,
    },
    tests: outcome?.tests ?? [],
    outcome,
  };
}

describe("the browser engine sends what the Rust engine sends", () => {
  for (const fixture of FIXTURES) {
    it(`${fixture.name} goes on the wire the way Rust puts it there`, async () => {
      const { wire } = await browserWire(fixture);
      assertWire(wire, fixture.wire, `browser/${fixture.name}`);
    });
  }

  it("a graphql variable value can never restructure the variables document", () => {
    const prepared = prepare({
      ...FIXTURES[0].spec,
      kind: "graphql",
      headers: [],
      auth: { type: "none" },
      graphql: { query: "{ ping }", variables: '{"q":"{{term}}"}' },
    });

    const variables = (JSON.parse(prepared.body as string) as { variables: Record<string, unknown> })
      .variables;
    expect(Object.keys(variables)).toEqual(["q"]);
    expect(variables["q"]).toBe(VARS["term"]);
  });

  it("a pre-script sees the same variables and reports the same tests as Rust", async () => {
    const scripts = FIXTURES.find((fixture) => fixture.name === "scripts") as Fixture;
    const { tests, outcome } = await browserWire(scripts);

    expect(outcome?.varSets).toEqual({ injected: "yes" });
    expect(tests.map((test) => [test.name, test.passed])).toEqual(
      (scripts.scriptTests as [string, boolean][]).slice(0, 1),
    );
  });

  it("every dynamic variable comes from the Rust prelude, not from a second table", async () => {
    const spec: RequestSpec = {
      ...FIXTURES[0].spec,
      url: `${ORIGIN}/x`,
      headers: [["X-Value", "{{$randomBankAccount}}"]],
      auth: { type: "none" },
    };

    await expect(prepareRequest(spec)).rejects.toThrow(/\$randomBankAccount/);
  });

  // The fixture's `.http` source is what the CLI was given when the wire pin below was
  // captured. If the browser's reader saw a different request in it, the two halves of
  // this file would be testing two different things.
  it("reads the same request out of the .http source the CLI was given", () => {
    for (const fixture of FIXTURES) {
      if (fixture.http === undefined) continue;
      const source = fixture.http.split("{{BASE}}").join(ORIGIN);
      const blocks = withFileVars(parseTextDocument(`${fixture.name}.http`, source, "http"));
      expect(blocks.length, fixture.name).toBe(1);
      const model = blocks[0].model;
      expect(model.method === "GET" ? "GET" : model.method, fixture.name).toBe(
        fixture.spec.kind === "graphql" ? "POST" : fixture.spec.method,
      );
      expect(model.url, fixture.name).toBe(fixture.spec.url);
    }
  });

  it("has a .http source for every fixture the CLI can be asked to send", () => {
    const derivable = fixtures.fixtures.filter(
      (fixture) => (fixture as { derivable?: boolean }).derivable !== false,
    );
    expect(derivable.length).toBeGreaterThan(8);
    for (const fixture of derivable)
      expect((fixture as { http?: string }).http, fixture.name).toBeTypeOf("string");
  });
});
