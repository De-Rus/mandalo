export class TemplateError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TemplateError";
  }
}

export class UnresolvedVarError extends Error {
  constructor(readonly key: string) {
    super(`unresolved variable: ${key}`);
    this.name = "UnresolvedVarError";
  }
}

export function apply(template: string, vars: Record<string, string>): string {
  let out = "";
  let rest = template;
  for (;;) {
    const start = rest.indexOf("{{");
    if (start === -1) break;
    out += rest.slice(0, start);
    const after = rest.slice(start + 2);
    const end = after.indexOf("}}");
    if (end === -1) throw new TemplateError(`unclosed {{ in: ${template}`);
    const key = after.slice(0, end).trim();
    const value = Object.prototype.hasOwnProperty.call(vars, key) ? vars[key] : undefined;
    if (value === undefined) throw new UnresolvedVarError(key);
    out += value;
    rest = after.slice(end + 2);
  }
  return out + rest;
}

export function applyJson(value: unknown, vars: Record<string, string>): unknown {
  if (typeof value === "string") return apply(value, vars);
  if (Array.isArray(value)) return value.map((item) => applyJson(item, vars));
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) out[apply(key, vars)] = applyJson(item, vars);
    return out;
  }
  return value;
}

// serde_json's Map is a BTreeMap, so every object Rust serialises comes out with sorted
// keys. GraphQL bodies are compared byte for byte in the parity suite, so mirror it.
export function canonicalJson(value: unknown): string {
  return JSON.stringify(sortKeys(value));
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value as Record<string, unknown>).sort()) {
      out[key] = sortKeys((value as Record<string, unknown>)[key]);
    }
    return out;
  }
  return value;
}
