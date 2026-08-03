export class JsonPathError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "JsonPathError";
  }
}

export class UnsupportedJsonPathError extends JsonPathError {
  constructor(
    readonly path: string,
    readonly feature: string,
  ) {
    super(`JSONPath ${JSON.stringify(path)} uses ${feature}, which the in-process engine cannot evaluate`);
    this.name = "UnsupportedJsonPathError";
  }
}

type Step =
  | { kind: "child"; name: string }
  | { kind: "index"; index: number }
  | { kind: "wildcard" }
  | { kind: "descendant"; name: string | null };

export function parse(path: string): Step[] {
  if (!path.startsWith("$")) throw new JsonPathError(`JSONPath must start with $: ${JSON.stringify(path)}`);
  const steps: Step[] = [];
  let i = 1;
  while (i < path.length) {
    const char = path[i];
    if (char === ".") {
      if (path[i + 1] === ".") {
        i += 2;
        if (path[i] === "*") {
          steps.push({ kind: "descendant", name: null });
          i += 1;
          continue;
        }
        if (path[i] === "[") continue;
        const name = readName(path, i);
        if (name === "") throw new JsonPathError(`empty name after .. in ${JSON.stringify(path)}`);
        steps.push({ kind: "descendant", name });
        i += name.length;
        continue;
      }
      i += 1;
      if (path[i] === "*") {
        steps.push({ kind: "wildcard" });
        i += 1;
        continue;
      }
      const name = readName(path, i);
      if (name === "") throw new JsonPathError(`empty name after . in ${JSON.stringify(path)}`);
      steps.push({ kind: "child", name });
      i += name.length;
      continue;
    }
    if (char === "[") {
      const close = path.indexOf("]", i);
      if (close === -1) throw new JsonPathError(`unclosed [ in ${JSON.stringify(path)}`);
      const inner = path.slice(i + 1, close).trim();
      i = close + 1;
      if (inner === "*") {
        steps.push({ kind: "wildcard" });
        continue;
      }
      if (inner.startsWith("?")) throw new UnsupportedJsonPathError(path, "a filter selector");
      if (inner.includes(":")) throw new UnsupportedJsonPathError(path, "a slice selector");
      if (inner.includes(",")) throw new UnsupportedJsonPathError(path, "a union selector");
      if (/^-?\d+$/.test(inner)) {
        steps.push({ kind: "index", index: Number(inner) });
        continue;
      }
      const quoted = /^'((?:[^'\\]|\\.)*)'$|^"((?:[^"\\]|\\.)*)"$/.exec(inner);
      if (!quoted) throw new JsonPathError(`unsupported selector [${inner}] in ${JSON.stringify(path)}`);
      steps.push({ kind: "child", name: unescape(quoted[1] ?? quoted[2] ?? "") });
      continue;
    }
    throw new JsonPathError(`unexpected ${JSON.stringify(char)} in ${JSON.stringify(path)}`);
  }
  return steps;
}

function readName(path: string, from: number): string {
  let end = from;
  while (end < path.length && /[A-Za-z0-9_\-$]/.test(path[end] as string)) end += 1;
  return path.slice(from, end);
}

function unescape(raw: string): string {
  return raw.replace(/\\(.)/g, "$1");
}

function child(node: unknown, name: string): unknown[] {
  if (node !== null && typeof node === "object" && !Array.isArray(node)) {
    const record = node as Record<string, unknown>;
    return Object.prototype.hasOwnProperty.call(record, name) ? [record[name]] : [];
  }
  return [];
}

function index(node: unknown, at: number): unknown[] {
  if (!Array.isArray(node)) return [];
  const resolved = at < 0 ? node.length + at : at;
  return resolved >= 0 && resolved < node.length ? [node[resolved]] : [];
}

function values(node: unknown): unknown[] {
  if (Array.isArray(node)) return [...node];
  if (node !== null && typeof node === "object") return Object.values(node as Record<string, unknown>);
  return [];
}

function descend(node: unknown, name: string | null): unknown[] {
  const out: unknown[] = [];
  const walk = (current: unknown): void => {
    if (name === null) out.push(...values(current));
    else out.push(...child(current, name));
    for (const next of values(current)) walk(next);
  };
  walk(node);
  return out;
}

export function query(path: string, root: unknown): unknown[] {
  let nodes: unknown[] = [root];
  for (const step of parse(path)) {
    const next: unknown[] = [];
    for (const node of nodes) {
      switch (step.kind) {
        case "child":
          next.push(...child(node, step.name));
          break;
        case "index":
          next.push(...index(node, step.index));
          break;
        case "wildcard":
          next.push(...values(node));
          break;
        case "descendant":
          next.push(...descend(node, step.name));
          break;
      }
    }
    nodes = next;
  }
  return nodes;
}
