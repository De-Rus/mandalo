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
