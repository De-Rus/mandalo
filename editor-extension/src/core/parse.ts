import { parse as parseToml } from "smol-toml";
import type { CollectionManifest, EnvironmentModel, WorkspaceManifest } from "./model";

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
