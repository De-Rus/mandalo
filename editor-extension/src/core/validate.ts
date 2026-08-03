import { CAPTURE_SCOPES, REQUEST_KINDS } from "./model";
import type { Capture, TestAssertion } from "./model";

export const TEST_OPS: Record<string, readonly string[]> = {
  status: ["eq", "ne", "lt", "gt"],
  json: ["eq", "ne", "exists", "absent", "contains", "matches", "lt", "gt", "len"],
  header: ["exists", "absent", "eq", "contains", "matches"],
  duration: ["lt", "gt"],
};

const VALUELESS_OPS = new Set(["exists", "absent"]);

export type CaptureSource =
  | { source: "status" }
  | { source: "header"; name: string }
  | { source: "body"; path: string };

export function parseCaptureSource(from: string): CaptureSource {
  if (from === "status") return { source: "status" };
  if (from.startsWith("header.")) {
    const name = from.slice("header.".length);
    if (name === "") throw new Error(`capture source has an empty header name: "${from}"`);
    return { source: "header", name };
  }
  if (from.startsWith("body.")) {
    const path = from.slice("body.".length);
    if (!path.startsWith("$")) {
      throw new Error(`capture source body path must start with $: "${from}"`);
    }
    validateJsonPath(path);
    return { source: "body", path };
  }
  throw new Error(
    `invalid capture source: "${from}" (expected status, header.<Name> or body.$.<jsonpath>)`,
  );
}

// Structural check only — the CLI owns the real JSONPath grammar. This catches the
// typos a hand-editor actually makes (unbalanced brackets, trailing dots, empty segments).
function validateJsonPath(path: string): void {
  if (path === "$") return;
  let depth = 0;
  let quote: string | null = null;
  for (let i = 1; i < path.length; i += 1) {
    const char = path[i]!;
    if (quote) {
      if (char === quote) quote = null;
      continue;
    }
    if (char === "'" || char === '"') quote = char;
    else if (char === "[") depth += 1;
    else if (char === "]") {
      depth -= 1;
      if (depth < 0) throw new Error(`invalid JSONPath in "${path}": unbalanced "]"`);
    } else if (char === "." && depth === 0) {
      const next = path[i + 1];
      if (next === undefined || next === "." || next === "[") {
        throw new Error(`invalid JSONPath in "${path}": empty path segment`);
      }
    }
  }
  if (quote) throw new Error(`invalid JSONPath in "${path}": unterminated quote`);
  if (depth !== 0) throw new Error(`invalid JSONPath in "${path}": unbalanced "["`);
}

export function validateCapture(capture: Capture): string[] {
  const problems: string[] = [];
  try {
    parseCaptureSource(capture.from);
  } catch (error) {
    problems.push((error as Error).message);
  }
  if (capture.into === "" || !/^[A-Za-z0-9_-]+$/.test(capture.into)) {
    problems.push(
      `invalid capture target: "${capture.into}" (letters, digits, "-" and "_" only)`,
    );
  }
  if (!(CAPTURE_SCOPES as readonly string[]).includes(capture.scope)) {
    problems.push(
      `unknown capture scope "${capture.scope}" (expected ${CAPTURE_SCOPES.join(", ")})`,
    );
  }
  return problems;
}

export function validateTest(test: TestAssertion): string[] {
  const problems: string[] = [];
  const kind = String(test.kind);
  const ops = TEST_OPS[kind];
  if (!ops) {
    problems.push(
      `unknown test kind "${kind}" (expected ${Object.keys(TEST_OPS).join(", ")})`,
    );
    return problems;
  }
  const op = String((test as Record<string, unknown>)["op"] ?? "");
  if (!ops.includes(op)) {
    problems.push(`test kind "${kind}" does not support op "${op}" (expected ${ops.join(", ")})`);
  }
  const record = test as Record<string, unknown>;
  const hasValue = record["value"] !== undefined;
  if (kind === "json" && typeof record["path"] !== "string") {
    problems.push('json tests require a "path" key');
  }
  if (kind === "header" && typeof record["name"] !== "string") {
    problems.push('header tests require a "name" key');
  }
  if (!VALUELESS_OPS.has(op) && !hasValue) {
    problems.push(`test kind "${kind}" with op "${op}" requires a "value"`);
  }
  if (VALUELESS_OPS.has(op) && hasValue) {
    problems.push(`test op "${op}" takes no "value"`);
  }
  if ((kind === "status" || kind === "duration") && hasValue && typeof record["value"] !== "number") {
    problems.push(`${kind} test "value" must be a number`);
  }
  return problems;
}

export function validateKind(kind: string): string | undefined {
  if ((REQUEST_KINDS as readonly string[]).includes(kind)) return undefined;
  return `unknown request kind "${kind}" (expected ${REQUEST_KINDS.join(", ")})`;
}

const VAR_PATTERN = /\{\{\s*([^{}\s][^{}]*?)\s*\}\}/g;

export interface VarReference {
  name: string;
  offset: number;
  length: number;
}

export function collectVarReferences(raw: string): VarReference[] {
  const found: VarReference[] = [];
  for (const match of raw.matchAll(VAR_PATTERN)) {
    found.push({
      name: match[1]!.trim(),
      offset: match.index,
      length: match[0].length,
    });
  }
  return found;
}
