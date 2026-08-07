import { parse } from "acorn";

export type ScriptKind = "pre" | "post";

export interface ScriptFinding {
  from: number;
  to: number;
  message: string;
  severity: "error" | "warning";
  code: string;
}

export const PM_MEMBERS = new Set([
  "info",
  "environment",
  "globals",
  "variables",
  "collectionVariables",
  "request",
  "response",
  "test",
  "expect",
  "sendRequest",
  "setNextRequest",
]);

// Mirrors the engine prelude's blockedMembers/unavailable() — same names, same
// reasons, surfaced at write time instead of run time.
export const PM_UNAVAILABLE = new Map<string, string>([
  [
    "execution",
    "run-flow control (setNextRequest, skipRequest, location) is not implemented — requests run in collection order",
  ],
  ["cookies", "there is no cookie jar; send cookies as an explicit Cookie header"],
  ["visualizer", "response visualizations are not implemented"],
  [
    "iterationData",
    "there is no data-file runner (newman -d); pass the value as an environment variable instead",
  ],
  ["vault", "the Postman vault is not implemented; use a Mándalo secret"],
  ["require", "there is no module loader; scripts cannot import packages"],
  [
    "sendRequest",
    "scripts cannot make network requests — chain a real request in the collection and capture what you need from its response",
  ],
  ["setNextRequest", "run order is the collection order; branching is not implemented"],
]);

interface PmChild {
  label: string;
  detail: string;
}

export interface PmEntry {
  label: string;
  detail: string;
  postOnly?: boolean;
  children?: PmChild[];
}

const VAR_SCOPE_CHILDREN: PmChild[] = [
  { label: "get", detail: "(name) → value" },
  { label: "set", detail: "(name, value)" },
  { label: "unset", detail: "(name)" },
  { label: "has", detail: "(name) → boolean" },
  { label: "toObject", detail: "() → object" },
];

export const PM_SURFACE: PmEntry[] = [
  { label: "test", detail: '(name, fn) — register a test' },
  { label: "expect", detail: "(value) — chai-style assertion" },
  { label: "environment", detail: "selected environment variables", children: VAR_SCOPE_CHILDREN },
  { label: "variables", detail: "merged variable scopes", children: VAR_SCOPE_CHILDREN },
  { label: "globals", detail: "global variables", children: VAR_SCOPE_CHILDREN },
  { label: "collectionVariables", detail: "collection variables", children: VAR_SCOPE_CHILDREN },
  {
    label: "request",
    detail: "the outgoing request",
    children: [
      { label: "method", detail: "HTTP method" },
      { label: "url", detail: "request URL object" },
      { label: "headers", detail: "header list (get/add/upsert/remove/each)" },
      { label: "body", detail: "request body" },
    ],
  },
  {
    label: "response",
    detail: "the received response",
    postOnly: true,
    children: [
      { label: "code", detail: "status code (number)" },
      { label: "status", detail: "status text" },
      { label: "responseTime", detail: "elapsed ms" },
      { label: "text", detail: "() → body as string" },
      { label: "json", detail: "() → body parsed as JSON" },
      { label: "headers", detail: "header list (get)" },
      { label: "to", detail: "assertion chain: to.have.status(200)" },
    ],
  },
  {
    label: "info",
    detail: "run metadata",
    children: [
      { label: "eventName", detail: '"prerequest" | "test"' },
      { label: "requestName", detail: "the request's name" },
    ],
  },
];

function stripPosition(message: string): string {
  return message.replace(/ \(\d+:\d+\)$/, "");
}

interface AstNode {
  type: string;
  start: number;
  end: number;
  [key: string]: unknown;
}

function walk(node: AstNode, visit: (node: AstNode) => void): void {
  visit(node);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) {
      for (const item of value) {
        if (item && typeof item === "object" && "type" in item) walk(item as AstNode, visit);
      }
    } else if (value && typeof value === "object" && "type" in value) {
      walk(value as AstNode, visit);
    }
  }
}

export function lintScriptSource(source: string, kind: ScriptKind): ScriptFinding[] {
  let program: AstNode;
  try {
    program = parse(source, {
      ecmaVersion: 2023,
      allowReturnOutsideFunction: true,
    }) as unknown as AstNode;
  } catch (error) {
    const raised = error as Error & { pos?: number };
    const from = typeof raised.pos === "number" ? Math.min(raised.pos, source.length) : 0;
    const lineEnd = source.indexOf("\n", from);
    const to = Math.max(from + 1, lineEnd === -1 ? source.length : lineEnd);
    return [
      {
        from,
        to,
        message: stripPosition(raised.message),
        severity: "error",
        code: "mandalo.scriptSyntax",
      },
    ];
  }

  const findings: ScriptFinding[] = [];
  walk(program, (node) => {
    if (node.type !== "MemberExpression" || node.computed === true) return;
    const object = node.object as AstNode;
    const property = node.property as AstNode;
    if (object.type !== "Identifier" || object.name !== "pm") return;
    if (property.type !== "Identifier") return;
    const name = property.name as string;
    const at = { from: property.start, to: property.end };
    const reason = PM_UNAVAILABLE.get(name);
    if (reason !== undefined) {
      findings.push({
        ...at,
        message: `pm.${name} fails when run: ${reason}`,
        severity: "warning",
        code: "mandalo.pmUnavailable",
      });
    } else if (!PM_MEMBERS.has(name)) {
      findings.push({
        ...at,
        message: `pm.${name} is not part of the pm API`,
        severity: "error",
        code: "mandalo.pmUnknown",
      });
    } else if (name === "response" && kind === "pre") {
      findings.push({
        ...at,
        message:
          "pm.response is not available in a pre-request script — the request has not been sent yet",
        severity: "warning",
        code: "mandalo.pmUnavailable",
      });
    }
  });
  return findings;
}
