import { parse as parseToml } from "smol-toml";
import type {
  Auth,
  Capture,
  CollectionManifest,
  EnvironmentModel,
  GraphqlBody,
  GrpcRequest,
  RequestModel,
  Scripts,
  TestAssertion,
  WorkspaceManifest,
} from "./model";

export class ParseError extends Error {
  constructor(
    message: string,
    readonly source?: string,
  ) {
    super(message);
    this.name = "ParseError";
  }
}

type Table = Record<string, unknown>;

function asTable(value: unknown, what: string): Table {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ParseError(`${what} must be a table`);
  }
  return value as Table;
}

function optString(table: Table, key: string, what: string): string | undefined {
  const value = table[key];
  if (value === undefined) return undefined;
  if (typeof value !== "string") throw new ParseError(`${what}.${key} must be a string`);
  return value;
}

function reqString(table: Table, key: string, what: string): string {
  const value = optString(table, key, what);
  if (value === undefined) throw new ParseError(`${what} is missing required key "${key}"`);
  return value;
}

function optNumber(table: Table, key: string, what: string): number | undefined {
  const value = table[key];
  if (value === undefined) return undefined;
  if (typeof value !== "number") throw new ParseError(`${what}.${key} must be a number`);
  return value;
}

function pairs(value: unknown, what: string): [string, string][] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new ParseError(`${what} must be an array of [key, value] pairs`);
  return value.map((entry, index) => {
    if (!Array.isArray(entry) || entry.length !== 2) {
      throw new ParseError(`${what}[${index}] must be a two-element [key, value] array`);
    }
    const [k, v] = entry;
    if (typeof k !== "string" || typeof v !== "string") {
      throw new ParseError(`${what}[${index}] must hold strings`);
    }
    return [k, v] as [string, string];
  });
}

function parseAuth(value: unknown): Auth {
  if (value === undefined) return { type: "none" };
  const table = asTable(value, "[auth]");
  const type = optString(table, "type", "[auth]") ?? "none";
  switch (type) {
    case "none":
      return { type: "none" };
    case "bearer":
      return { type: "bearer", token: reqString(table, "token", "[auth]") };
    case "basic":
      return {
        type: "basic",
        username: reqString(table, "username", "[auth]"),
        password: reqString(table, "password", "[auth]"),
      };
    case "apikey":
      return {
        type: "apikey",
        key: reqString(table, "key", "[auth]"),
        value: reqString(table, "value", "[auth]"),
        placement: optString(table, "placement", "[auth]") ?? "header",
      };
    default:
      throw new ParseError(`[auth].type must be none, bearer, basic or apikey (got "${type}")`);
  }
}

function parseScripts(value: unknown): Scripts {
  if (value === undefined) return {};
  const table = asTable(value, "[scripts]");
  const scripts: Scripts = {};
  const pre = optString(table, "pre", "[scripts]");
  const post = optString(table, "post", "[scripts]");
  if (pre !== undefined) scripts.pre = pre;
  if (post !== undefined) scripts.post = post;
  return scripts;
}

function parseGraphql(value: unknown): GraphqlBody | undefined {
  if (value === undefined) return undefined;
  const table = asTable(value, "[graphql]");
  return {
    query: reqString(table, "query", "[graphql]"),
    variables: optString(table, "variables", "[graphql]") ?? "",
  };
}

// The desktop app serialises `GrpcRequest` with serde camelCase, so `protoPaths` is the
// canonical spelling and the one we write; `proto_paths` is still read for older files.
function parseGrpc(value: unknown): GrpcRequest | undefined {
  if (value === undefined) return undefined;
  const table = asTable(value, "[grpc]");
  const raw = table["protoPaths"] ?? table["proto_paths"];
  let protoPaths: string[] = [];
  if (raw !== undefined) {
    if (!Array.isArray(raw) || raw.some((p) => typeof p !== "string")) {
      throw new ParseError("[grpc].protoPaths must be an array of strings");
    }
    protoPaths = raw as string[];
  }
  return {
    protoPaths,
    service: reqString(table, "service", "[grpc]"),
    method: reqString(table, "method", "[grpc]"),
    message: optString(table, "message", "[grpc]") ?? "{}",
    metadata: pairs(table["metadata"], "[grpc].metadata"),
  };
}

function parseTests(value: unknown): TestAssertion[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new ParseError("[[tests]] must be an array of tables");
  return value.map((entry, index) => {
    const table = asTable(entry, `[[tests]][${index}]`);
    const kind = reqString(table, "kind", `[[tests]][${index}]`);
    const op = reqString(table, "op", `[[tests]][${index}]`);
    return { ...table, kind, op } as TestAssertion;
  });
}

function parseCaptures(value: unknown): Capture[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new ParseError("[[captures]] must be an array of tables");
  return value.map((entry, index) => {
    const what = `[[captures]][${index}]`;
    const table = asTable(entry, what);
    return {
      from: reqString(table, "from", what),
      into: reqString(table, "into", what),
      scope: optString(table, "scope", what) ?? "run",
    };
  });
}

export function parseRequest(raw: string): RequestModel {
  let root: Table;
  try {
    root = asTable(parseToml(raw), "request");
  } catch (error) {
    throw new ParseError(`invalid TOML: ${(error as Error).message}`, raw);
  }
  const method = optString(root, "method", "request");
  const kind = optString(root, "kind", "request") ?? "http";
  const model: RequestModel = {
    schemaVersion: optNumber(root, "schema_version", "request") ?? 1,
    id: optString(root, "id", "request") ?? "",
    name: reqString(root, "name", "request"),
    kind,
    method: method ?? (kind === "http" ? "GET" : "POST"),
    url: reqString(root, "url", "request"),
    headers: pairs(root["headers"], "headers"),
    auth: parseAuth(root["auth"]),
    scripts: parseScripts(root["scripts"]),
    tests: parseTests(root["tests"]),
    captures: parseCaptures(root["captures"]),
  };
  const description = optString(root, "description", "request");
  const body = optString(root, "body", "request");
  const graphql = parseGraphql(root["graphql"]);
  const grpc = parseGrpc(root["grpc"]);
  if (description !== undefined) model.description = description;
  if (body !== undefined) model.body = body;
  if (graphql !== undefined) model.graphql = graphql;
  if (grpc !== undefined) model.grpc = grpc;
  return model;
}

export function parseCollectionManifest(raw: string): CollectionManifest {
  const root = asTable(parseToml(raw), "collection.toml");
  return {
    schemaVersion: optNumber(root, "schema_version", "collection.toml") ?? 1,
    id: optString(root, "id", "collection.toml") ?? "",
    name: reqString(root, "name", "collection.toml"),
  };
}

export function parseWorkspaceManifest(raw: string): WorkspaceManifest {
  const root = asTable(parseToml(raw), "mandalo.toml");
  return {
    schemaVersion: optNumber(root, "schema_version", "mandalo.toml") ?? 1,
    id: optString(root, "id", "mandalo.toml") ?? "",
    name: optString(root, "name", "mandalo.toml") ?? "Workspace",
  };
}

export function parseEnvironment(raw: string, fallbackName: string): EnvironmentModel {
  const root = asTable(parseToml(raw), "environment");
  const vars: Record<string, string> = {};
  const table = root["vars"];
  if (table !== undefined) {
    for (const [key, value] of Object.entries(asTable(table, "[vars]"))) {
      if (typeof value !== "string") throw new ParseError(`[vars].${key} must be a string`);
      vars[key] = value;
    }
  }
  return { name: optString(root, "name", "environment") ?? fallbackName, vars };
}

function quote(value: string): string {
  return JSON.stringify(value);
}

export function renderRequest(model: RequestModel): string {
  const lines = [
    `schema_version = ${model.schemaVersion}`,
    `id = ${quote(model.id)}`,
    `name = ${quote(model.name)}`,
    `kind = ${quote(model.kind)}`,
    `method = ${quote(model.method)}`,
    `url = ${quote(model.url)}`,
  ];
  const headers = model.headers.map(([k, v]) => `[${quote(k)}, ${quote(v)}]`).join(", ");
  lines.push(`headers = [${headers}]`);
  if (model.body !== undefined) lines.push(`body = ${quote(model.body)}`);
  lines.push("", "[auth]", `type = ${quote(model.auth.type)}`);
  for (const [key, value] of Object.entries(model.auth)) {
    if (key === "type") continue;
    lines.push(`${key} = ${quote(String(value))}`);
  }
  if (model.graphql) {
    lines.push(
      "",
      "[graphql]",
      `query = ${quote(model.graphql.query)}`,
      `variables = ${quote(model.graphql.variables)}`,
    );
  }
  if (model.grpc) {
    const protos = model.grpc.protoPaths.map(quote).join(", ");
    const metadata = model.grpc.metadata.map(([k, v]) => `[${quote(k)}, ${quote(v)}]`).join(", ");
    lines.push(
      "",
      "[grpc]",
      `protoPaths = [${protos}]`,
      `service = ${quote(model.grpc.service)}`,
      `method = ${quote(model.grpc.method)}`,
      `message = ${quote(model.grpc.message)}`,
      `metadata = [${metadata}]`,
    );
  }
  if (model.scripts.pre !== undefined || model.scripts.post !== undefined) {
    lines.push("", "[scripts]");
    if (model.scripts.pre !== undefined) lines.push(`pre = ${quote(model.scripts.pre)}`);
    if (model.scripts.post !== undefined) lines.push(`post = ${quote(model.scripts.post)}`);
  }
  for (const test of model.tests) {
    lines.push("", "[[tests]]");
    for (const [key, value] of Object.entries(test)) {
      lines.push(`${key} = ${typeof value === "string" ? quote(value) : JSON.stringify(value)}`);
    }
  }
  for (const capture of model.captures) {
    lines.push(
      "",
      "[[captures]]",
      `from = ${quote(capture.from)}`,
      `into = ${quote(capture.into)}`,
      `scope = ${quote(capture.scope)}`,
    );
  }
  return `${lines.join("\n")}\n`;
}
