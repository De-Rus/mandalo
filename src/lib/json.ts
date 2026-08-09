export type TokenType =
  | "key"
  | "string"
  | "number"
  | "bool"
  | "null"
  | "punct"
  | "text";

export interface Token {
  type: TokenType;
  text: string;
}

const PUNCT = new Set(["{", "}", "[", "]", ":", ","]);

function readString(src: string, start: number): number {
  let i = start + 1;
  while (i < src.length) {
    const c = src[i];
    if (c === "\\") {
      i += 2;
      continue;
    }
    if (c === '"') return i + 1;
    i++;
  }
  return src.length;
}

function readNumber(src: string, start: number): number {
  let i = start;
  if (src[i] === "-") i++;
  while (i < src.length && /[0-9eE+.\-]/.test(src[i])) i++;
  return i;
}

function nextMeaningful(src: string, from: number): string {
  let i = from;
  while (i < src.length && /\s/.test(src[i])) i++;
  return src[i] ?? "";
}

export function tokenizeJson(src: string): Token[] {
  const tokens: Token[] = [];
  const push = (type: TokenType, text: string) => {
    if (text === "") return;
    const last = tokens[tokens.length - 1];
    if (last && last.type === type) last.text += text;
    else tokens.push({ type, text });
  };

  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === '"') {
      const end = readString(src, i);
      const text = src.slice(i, end);
      push(nextMeaningful(src, end) === ":" ? "key" : "string", text);
      i = end;
      continue;
    }
    if (PUNCT.has(c)) {
      push("punct", c);
      i++;
      continue;
    }
    if (c === "-" || (c >= "0" && c <= "9")) {
      const end = readNumber(src, i);
      push("number", src.slice(i, end));
      i = end;
      continue;
    }
    if (src.startsWith("true", i) || src.startsWith("false", i)) {
      const word = src.startsWith("true", i) ? "true" : "false";
      push("bool", word);
      i += word.length;
      continue;
    }
    if (src.startsWith("null", i)) {
      push("null", "null");
      i += 4;
      continue;
    }
    push("text", c);
    i++;
  }
  return tokens;
}

export interface JsonLeaf {
  /** RFC 9535 path — `body.<path>` is exactly what `parse_capture_source` takes. */
  path: string;
  raw: string;
  type: "string" | "number" | "bool" | "null";
}

interface Frame {
  path: string;
  array: boolean;
  index: number;
}

function decodeKey(text: string): string {
  try {
    return JSON.parse(text) as string;
  } catch {
    return text.replace(/^"|"$/g, "");
  }
}

function segment(key: string): string {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key)
    ? `.${key}`
    : `['${key.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}']`;
}

const LEAF_TYPES = new Set<TokenType>(["string", "number", "bool", "null"]);

/**
 * The path of the one scalar on each line, walked over the tokenizer's own
 * output — a line holding two scalars gets none, because no single path
 * describes it.
 */
export function leafPaths(lines: Token[][]): (JsonLeaf | null)[] {
  const found: (JsonLeaf | null)[] = lines.map(() => null);
  const crowded = new Set<number>();
  const stack: Frame[] = [];
  let key: string | null = null;

  const childPath = (): string | null => {
    const frame = stack[stack.length - 1];
    if (!frame) return "$";
    if (frame.array) return `${frame.path}[${frame.index}]`;
    return key === null ? null : `${frame.path}${segment(key)}`;
  };

  lines.forEach((tokens, line) => {
    for (const token of tokens) {
      if (token.type === "key") {
        key = decodeKey(token.text);
        continue;
      }
      if (LEAF_TYPES.has(token.type)) {
        const path = childPath();
        if (path !== null) {
          if (found[line] !== null) crowded.add(line);
          found[line] = { path, raw: token.text, type: token.type as JsonLeaf["type"] };
        }
        key = null;
        continue;
      }
      if (token.type !== "punct") continue;
      for (const char of token.text) {
        if (char === "{" || char === "[") {
          stack.push({ path: childPath() ?? "$", array: char === "[", index: 0 });
          key = null;
        } else if (char === "}" || char === "]") {
          stack.pop();
          key = null;
        } else if (char === ",") {
          const frame = stack[stack.length - 1];
          if (frame?.array) frame.index++;
          key = null;
        }
      }
    }
  });

  return found.map((leaf, line) => (crowded.has(line) ? null : leaf));
}

export function tokenizeLines(src: string): Token[][] {
  const lines: Token[][] = [[]];
  for (const token of tokenizeJson(src)) {
    const parts = token.text.split("\n");
    parts.forEach((part, index) => {
      if (index > 0) lines.push([]);
      if (part !== "") lines[lines.length - 1].push({ type: token.type, text: part });
    });
  }
  return lines;
}
