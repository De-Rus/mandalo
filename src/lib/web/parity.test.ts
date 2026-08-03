import { beforeAll, describe, expect, it } from "vitest";
import type { RequestSpec, ScriptOutcome, ScriptTest } from "../api";
import fixtures from "./parity.fixtures.json";
import scriptRs from "../../../crates/core/src/script.rs?raw";
import { prepare, prepareRequest } from "./send";
import { webExecuteScript } from "./script";

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
  pre: (fixture as { pre?: string }).pre,
  scriptTests: (fixture as { scriptTests?: [string, boolean][] }).scriptTests,
}));

const PRELUDE_OPEN = 'const PRELUDE: &str = r#"';

function prelude(): string {
  const start = scriptRs.indexOf(PRELUDE_OPEN);
  const end = start === -1 ? -1 : scriptRs.indexOf('"#;', start + PRELUDE_OPEN.length);
  if (end === -1) throw new Error("the pm.* prelude moved inside crates/core/src/script.rs");
  return scriptRs.slice(start + PRELUDE_OPEN.length, end);
}

/**
 * jsdom has no Worker, so the suite runs the Rust prelude itself — the same
 * bytes the browser build ships — inside a `with` scope that stands in for the
 * worker's global. Faking the generated values instead would leave the one
 * thing this fixture is here to prove untested.
 */
class PreludeWorker {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;

  postMessage(payload: { source: string; context: unknown }): void {
    const scope: Record<string, unknown> = { __ctx_json: JSON.stringify(payload.context) };
    scope["globalThis"] = scope;
    let reply: unknown;
    try {
      const run = new Function(
        "__scope",
        `with (__scope) { ${prelude()}\n${payload.source}\n; return __mandalo.outcome(); }`,
      ) as (scope: Record<string, unknown>) => string;
      reply = { ok: true, outcome: JSON.parse(run(scope)) };
    } catch (e) {
      reply = { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
    this.onmessage?.({ data: reply });
  }

  terminate(): void {}
}

beforeAll(() => {
  globalThis.Worker = PreludeWorker as unknown as typeof Worker;
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
});
