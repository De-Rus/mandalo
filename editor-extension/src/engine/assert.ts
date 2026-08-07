import type { Capture, TestAssertion } from "../../../src/lib/format/model";
import { JsonPathError, query } from "./jsonpath";

export interface AssertionResult {
  name: string;
  passed: boolean;
  detail: string | null;
}

export class CaptureError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CaptureError";
  }
}

export interface CaptureResult {
  from: string;
  into: string;
  value: string;
  scope: string;
}

function debug(value: string): string {
  return JSON.stringify(value);
}

function display(node: unknown): string {
  return JSON.stringify(node) ?? "null";
}

function nodeToString(node: unknown): string {
  return typeof node === "string" ? node : display(node);
}

function numeric(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function jsonEq(node: unknown, expected: unknown): boolean {
  const a = numeric(node);
  const b = numeric(expected);
  if (a !== null && b !== null) return a === b;
  return display(node) === display(expected);
}

function field(assertion: TestAssertion, key: string): unknown {
  return (assertion as Record<string, unknown>)[key];
}

export function assertionName(assertion: TestAssertion): string {
  const op = String(field(assertion, "op") ?? "");
  switch (assertion.kind) {
    case "status":
      return `status ${op} ${field(assertion, "value")}`;
    case "duration":
      return `duration ${op} ${field(assertion, "value")}ms`;
    case "header": {
      const name = String(field(assertion, "name") ?? "");
      const value = field(assertion, "value");
      return value === undefined || value === null
        ? `header ${name} ${op}`
        : `header ${name} ${op} ${value}`;
    }
    case "json": {
      const path = String(field(assertion, "path") ?? "");
      const value = field(assertion, "value");
      return value === undefined ? `json ${path} ${op}` : `json ${path} ${op} ${nodeToString(value)}`;
    }
    default:
      return `${assertion.kind} ${op}`;
  }
}

export interface ResponseFacts {
  status: number;
  headers: [string, string][];
  body: string;
  durationMs: number;
}

export function evaluateTests(
  tests: readonly TestAssertion[],
  facts: ResponseFacts,
): AssertionResult[] {
  let parsed: { ok: true; value: unknown } | { ok: false; error: string } | undefined;
  return tests.map((assertion) => {
    let detail: string | null = null;
    try {
      switch (assertion.kind) {
        case "status":
          detail = checkStatus(assertion, facts.status);
          break;
        case "duration":
          detail = checkDuration(assertion, facts.durationMs);
          break;
        case "header":
          detail = checkHeader(assertion, facts.headers);
          break;
        case "json": {
          parsed ??= parseBody(facts.body);
          detail = parsed.ok ? checkJson(assertion, parsed.value) : parsed.error;
          break;
        }
        default:
          detail = `unknown test kind: ${assertion.kind}`;
      }
    } catch (error) {
      detail = (error as Error).message;
    }
    return { name: assertionName(assertion), passed: detail === null, detail };
  });
}

function parseBody(body: string): { ok: true; value: unknown } | { ok: false; error: string } {
  try {
    return { ok: true, value: JSON.parse(body) };
  } catch (error) {
    return { ok: false, error: `response body is not JSON: ${(error as Error).message}` };
  }
}

function checkStatus(assertion: TestAssertion, actual: number): string | null {
  const expected = Number(field(assertion, "value"));
  const op = String(field(assertion, "op"));
  const ok =
    op === "eq"
      ? actual === expected
      : op === "ne"
        ? actual !== expected
        : op === "lt"
          ? actual < expected
          : op === "gt"
            ? actual > expected
            : null;
  if (ok === null) throw new Error(`unknown status op: ${op}`);
  return ok ? null : `status was ${actual}`;
}

function checkDuration(assertion: TestAssertion, actual: number): string | null {
  const expected = Number(field(assertion, "value"));
  const op = String(field(assertion, "op"));
  const ok = op === "lt" ? actual < expected : op === "gt" ? actual > expected : null;
  if (ok === null) throw new Error(`unknown duration op: ${op}`);
  return ok ? null : `took ${actual}ms`;
}

function checkHeader(assertion: TestAssertion, headers: [string, string][]): string | null {
  const name = String(field(assertion, "name") ?? "");
  const op = String(field(assertion, "op"));
  const raw = field(assertion, "value");
  const expected = raw === undefined || raw === null ? null : String(raw);
  const values = headers.filter(([k]) => k.toLowerCase() === name.toLowerCase()).map(([, v]) => v);

  if (op === "exists") return values.length === 0 ? `header ${name} is absent` : null;
  if (op === "absent") return values.length === 0 ? null : `header ${name} is ${values.join(", ")}`;
  if (op !== "eq" && op !== "contains" && op !== "matches") throw new Error(`unknown header op: ${op}`);
  if (expected === null) return `header ${op} needs a value`;
  if (values.length === 0) return `header ${name} is absent`;

  const ok =
    op === "eq"
      ? values.includes(expected)
      : op === "contains"
        ? values.some((v) => v.includes(expected))
        : values.some((v) => regex(expected).test(v));
  return ok ? null : `header ${name} is ${values.join(", ")}`;
}

function regex(pattern: string): RegExp {
  try {
    return new RegExp(pattern);
  } catch (error) {
    throw new Error(`invalid regex ${debug(pattern)}: ${(error as Error).message}`);
  }
}

function checkJson(assertion: TestAssertion, json: unknown): string | null {
  const path = String(field(assertion, "path") ?? "");
  const op = String(field(assertion, "op"));
  const expected = field(assertion, "value");
  let nodes: unknown[];
  try {
    nodes = query(path, json);
  } catch (error) {
    if (error instanceof JsonPathError) throw error;
    throw new Error(`invalid JSONPath ${debug(path)}: ${(error as Error).message}`);
  }
  const found = nodes.length > 0;
  const node = nodes[0];

  if (op === "exists") return found ? null : `${path} matched nothing`;
  if (op === "absent") return found ? `${path} is ${display(node)}` : null;
  if (!found) return `${path} matched nothing`;
  if (expected === undefined) return `json ${op} needs a value`;

  const fail = `${path} is ${display(node)}`;
  switch (op) {
    case "eq":
      return jsonEq(node, expected) ? null : fail;
    case "ne":
      return jsonEq(node, expected) ? fail : null;
    case "lt":
    case "gt": {
      const a = numeric(node);
      if (a === null) return `${path} is not a number: ${display(node)}`;
      const b = numeric(expected);
      if (b === null) return `json ${op} needs a numeric value, got ${display(expected)}`;
      return (op === "lt" ? a < b : a > b) ? null : fail;
    }
    case "len": {
      const want = numeric(expected);
      if (want === null) return `json len needs a numeric value, got ${display(expected)}`;
      const got = length(node);
      if (got === null) return `${path} has no length: ${display(node)}`;
      return got === want ? null : fail;
    }
    case "contains":
      return contains(path, node, expected) ? null : fail;
    case "matches": {
      if (typeof expected !== "string") {
        return `json matches needs a string pattern, got ${display(expected)}`;
      }
      return regex(expected).test(nodeToString(node)) ? null : fail;
    }
    default:
      throw new Error(`unknown json op: ${op}`);
  }
}

function length(node: unknown): number | null {
  if (Array.isArray(node)) return node.length;
  if (typeof node === "string") return [...node].length;
  if (node !== null && typeof node === "object") return Object.keys(node).length;
  return null;
}

function contains(path: string, node: unknown, needle: unknown): boolean {
  if (typeof node === "string") {
    return node.includes(typeof needle === "string" ? needle : display(needle));
  }
  if (Array.isArray(node)) return node.some((item) => jsonEq(item, needle));
  if (node !== null && typeof node === "object") {
    if (typeof needle !== "string") {
      throw new Error(`${path} is an object, ${display(needle)} is not a key`);
    }
    return Object.prototype.hasOwnProperty.call(node, needle);
  }
  throw new Error(`${path} cannot contain anything: ${display(node)}`);
}

export function resolveCapture(from: string, facts: ResponseFacts): string | null {
  if (from === "status") return String(facts.status);
  if (from.startsWith("header.")) {
    const name = from.slice("header.".length);
    if (name === "") throw new CaptureError(`capture source has an empty header name: ${debug(from)}`);
    const hit = facts.headers.find(([k]) => k.toLowerCase() === name.toLowerCase());
    return hit ? hit[1] : null;
  }
  if (from.startsWith("body.")) {
    const path = from.slice("body.".length);
    if (!path.startsWith("$")) {
      throw new CaptureError(`capture source body path must start with $: ${debug(from)}`);
    }
    let json: unknown;
    try {
      json = JSON.parse(facts.body);
    } catch (error) {
      throw new CaptureError(
        `response body is not JSON, cannot capture ${debug(from)}: ${(error as Error).message}`,
      );
    }
    const nodes = query(path, json);
    return nodes.length === 0 ? null : nodeToString(nodes[0]);
  }
  throw new CaptureError(
    `invalid capture source: ${debug(from)} (expected status, header.<Name> or body.$.<jsonpath>)`,
  );
}

export function applyCaptures(
  captures: readonly Capture[],
  facts: ResponseFacts,
  vars: Record<string, string>,
): CaptureResult[] {
  return captures.map((capture) => {
    const value = resolveCapture(capture.from, facts);
    if (value === null) throw new CaptureError(`capture ${debug(capture.from)} matched nothing`);
    vars[capture.into] = value;
    return { from: capture.from, into: capture.into, value, scope: capture.scope };
  });
}
