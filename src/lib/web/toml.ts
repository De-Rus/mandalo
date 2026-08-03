import type {
  Auth,
  Capture,
  CaptureScope,
  DurationOp,
  Environment,
  HeaderOp,
  JsonOp,
  Kind,
  SavedGrpc,
  SavedRequest,
  Scripts,
  StatusOp,
  TestAssertion,
} from "../api";

export type TomlValue = string | number | boolean | TomlValue[] | TomlTable;

export interface TomlTable {
  [key: string]: TomlValue;
}

const BARE_KEY = /^[A-Za-z0-9_-]+$/;
const DATETIME =
  /^\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:\d{2})?)?/;
const LOCAL_TIME = /^\d{2}:\d{2}:\d{2}(?:\.\d+)?/;
const DEC_INT = /^[+-]?(?:0|[0-9](?:_?[0-9])*)/;
const FLOAT_TAIL = /^(?:\.[0-9](?:_?[0-9])*)?(?:[eE][+-]?[0-9](?:_?[0-9])*)?/;
const RADIX = /^0([xob])([0-9A-Fa-f_]+)/;

const SEP = "\u0001";

class TomlParser {
  private pos = 0;
  private line = 1;
  private readonly root: TomlTable = {};
  private current: TomlTable = this.root;
  private prefix = "";

  private readonly explicit = new Set<string>();
  private readonly arrays = new Set<string>();
  private readonly dotted = new Set<string>();
  private readonly keys = new Set<string>();

  constructor(private readonly src: string) {}

  parse(): TomlTable {
    for (;;) {
      this.skipIgnorable(true);
      if (this.pos >= this.src.length) return this.root;
      if (this.peek() === "[") {
        this.parseHeader();
      } else {
        this.parseKeyValue(this.current, this.prefix);
      }
      this.expectLineEnd();
    }
  }

  private fail(message: string): never {
    throw new Error(`TOML line ${this.line}: ${message}`);
  }

  private peek(offset = 0): string {
    return this.src[this.pos + offset] ?? "";
  }

  private take(): string {
    const c = this.src[this.pos];
    if (c === undefined) this.fail("unexpected end of input");
    this.pos += 1;
    if (c === "\n") this.line += 1;
    return c;
  }

  private skipRun(count: number): string {
    const text = this.src.slice(this.pos, this.pos + count);
    for (let i = 0; i < text.length; i += 1) if (text[i] === "\n") this.line += 1;
    this.pos += text.length;
    return text;
  }

  private skipSpaces(): void {
    while (this.peek() === " " || this.peek() === "\t") this.pos += 1;
  }

  private skipComment(): void {
    if (this.peek() !== "#") return;
    while (this.pos < this.src.length && this.peek() !== "\n") this.pos += 1;
  }

  private skipIgnorable(newlines: boolean): void {
    for (;;) {
      this.skipSpaces();
      if (this.peek() === "#") {
        this.skipComment();
        continue;
      }
      if (newlines && (this.peek() === "\n" || this.peek() === "\r")) {
        this.take();
        continue;
      }
      return;
    }
  }

  private expectLineEnd(): void {
    this.skipSpaces();
    this.skipComment();
    if (this.pos >= this.src.length) return;
    const c = this.peek();
    if (c === "\n") {
      this.take();
      return;
    }
    if (c === "\r" && this.peek(1) === "\n") {
      this.pos += 1;
      this.take();
      return;
    }
    this.fail(`unexpected trailing text: ${JSON.stringify(c)}`);
  }

  private parseHeader(): void {
    const isArray = this.peek(1) === "[";
    this.skipRun(isArray ? 2 : 1);
    this.skipSpaces();
    const parts = this.parseKeyPath();
    this.skipSpaces();
    if (this.peek() !== "]") this.fail("expected ']' to close the table header");
    this.pos += 1;
    if (isArray) {
      if (this.peek() !== "]") this.fail("expected ']]' to close the array-of-tables header");
      this.pos += 1;
    }
    const display = parts.join(".");
    const landing = this.walk(parts.slice(0, -1), display);
    const last = parts[parts.length - 1];
    const path = landing.prefix + SEP + last;
    const existing = landing.table[last];

    if (this.keys.has(path)) this.fail(`${display} is already defined as a value`);

    if (isArray) {
      if (existing === undefined) {
        this.arrays.add(path);
        const created: TomlTable = {};
        landing.table[last] = [created];
        this.current = created;
        this.prefix = `${path}${SEP}#0`;
        return;
      }
      if (!this.arrays.has(path) || !Array.isArray(existing)) {
        this.fail(`${display} is already defined as a table, not an array of tables`);
      }
      const created: TomlTable = {};
      (existing as TomlTable[]).push(created);
      this.current = created;
      this.prefix = `${path}${SEP}#${existing.length - 1}`;
      return;
    }

    if (this.arrays.has(path)) this.fail(`${display} is already defined as an array of tables`);
    if (this.explicit.has(path)) this.fail(`table ${display} is defined more than once`);
    if (this.dotted.has(path)) {
      this.fail(`table ${display} was already defined by a dotted key`);
    }
    if (existing === undefined) landing.table[last] = {};
    else if (typeof existing !== "object" || Array.isArray(existing)) {
      this.fail(`${display} is already defined as a value`);
    }
    this.explicit.add(path);
    this.current = landing.table[last] as TomlTable;
    this.prefix = path;
  }

  private walk(parts: string[], display: string): { table: TomlTable; prefix: string } {
    let table = this.root;
    let prefix = "";
    for (const part of parts) {
      prefix += SEP + part;
      let child = table[part];
      if (child === undefined) {
        child = {};
        table[part] = child;
      }
      if (Array.isArray(child)) {
        if (!this.arrays.has(prefix)) this.fail(`${display} walks through a non-table value`);
        const items = child as TomlTable[];
        const last = items[items.length - 1];
        if (last === undefined) this.fail(`${display} walks through an empty array of tables`);
        table = last;
        prefix += `${SEP}#${items.length - 1}`;
        continue;
      }
      if (typeof child !== "object") this.fail(`${display} walks through a non-table value`);
      table = child as TomlTable;
    }
    return { table, prefix };
  }

  private parseKeyValue(target: TomlTable, basePrefix: string): void {
    const parts = this.parseKeyPath();
    this.skipSpaces();
    if (this.peek() !== "=") this.fail(`expected '=' after key ${parts.join(".")}`);
    this.pos += 1;
    this.skipSpaces();
    const value = this.parseValue();

    let table = target;
    let prefix = basePrefix;
    for (let i = 0; i < parts.length - 1; i += 1) {
      const part = parts[i];
      prefix += SEP + part;
      if (this.keys.has(prefix)) this.fail(`${parts.join(".")} redefines an existing value`);
      if (this.explicit.has(prefix) || this.arrays.has(prefix)) {
        this.fail(`${parts.join(".")} redefines an existing table`);
      }
      let child = table[part];
      if (child === undefined) {
        child = {};
        table[part] = child;
        this.dotted.add(prefix);
      }
      if (typeof child !== "object" || Array.isArray(child)) {
        this.fail(`${parts.join(".")} redefines an existing value`);
      }
      table = child as TomlTable;
    }
    const last = parts[parts.length - 1];
    const path = prefix + SEP + last;
    if (
      this.keys.has(path) ||
      this.explicit.has(path) ||
      this.arrays.has(path) ||
      this.dotted.has(path) ||
      last in table
    ) {
      this.fail(`duplicate key: ${parts.join(".")}`);
    }
    this.keys.add(path);
    table[last] = value;
  }

  private parseKeyPath(): string[] {
    const parts = [this.parseKey()];
    for (;;) {
      this.skipSpaces();
      if (this.peek() !== ".") return parts;
      this.pos += 1;
      this.skipSpaces();
      parts.push(this.parseKey());
    }
  }

  private parseKey(): string {
    const c = this.peek();
    if (c === '"') return this.parseBasicString();
    if (c === "'") return this.parseLiteralString();
    const start = this.pos;
    while (BARE_KEY.test(this.peek())) this.pos += 1;
    if (this.pos === start) this.fail(`expected a key, found ${JSON.stringify(c || "<eof>")}`);
    return this.src.slice(start, this.pos);
  }

  private parseValue(): TomlValue {
    const c = this.peek();
    if (c === "") this.fail("expected a value, found end of input");
    if (c === '"') {
      if (this.peek(1) === '"' && this.peek(2) === '"') return this.parseMultilineBasic();
      return this.parseBasicString();
    }
    if (c === "'") {
      if (this.peek(1) === "'" && this.peek(2) === "'") return this.parseMultilineLiteral();
      return this.parseLiteralString();
    }
    if (c === "[") return this.parseArray();
    if (c === "{") return this.parseInlineTable();
    if (this.src.startsWith("true", this.pos)) {
      this.pos += 4;
      return true;
    }
    if (this.src.startsWith("false", this.pos)) {
      this.pos += 5;
      return false;
    }
    const rest = this.src.slice(this.pos);
    const datetime = DATETIME.exec(rest) ?? LOCAL_TIME.exec(rest);
    if (datetime) {
      this.pos += datetime[0].length;
      return datetime[0];
    }
    return this.parseNumber();
  }

  private parseNumber(): number {
    const rest = this.src.slice(this.pos);
    const radix = RADIX.exec(rest);
    if (radix) {
      const base = radix[1] === "x" ? 16 : radix[1] === "o" ? 8 : 2;
      const digits = radix[2].replace(/_/g, "");
      if (!/^[0-9A-Fa-f]+$/.test(digits) || radix[2].startsWith("_") || radix[2].endsWith("_")) {
        this.fail(`malformed number: ${radix[0]}`);
      }
      const parsed = parseInt(digits, base);
      if (!Number.isFinite(parsed) || Number.isNaN(parsed)) {
        this.fail(`malformed number: ${radix[0]}`);
      }
      this.pos += radix[0].length;
      return parsed;
    }
    const special = /^([+-]?)(inf|nan)/.exec(rest);
    if (special) {
      this.pos += special[0].length;
      if (special[2] === "nan") return NaN;
      return special[1] === "-" ? -Infinity : Infinity;
    }
    const int = DEC_INT.exec(rest);
    if (!int || int[0] === "" || int[0] === "+" || int[0] === "-") {
      const bad = /^\S+/.exec(rest);
      this.fail(`expected a value, found ${JSON.stringify(bad ? bad[0] : "<eof>")}`);
    }
    const tail = FLOAT_TAIL.exec(rest.slice(int[0].length));
    const text = int[0] + (tail ? tail[0] : "");
    this.pos += text.length;
    const cleaned = text.replace(/_/g, "");
    const value = Number(cleaned);
    if (Number.isNaN(value)) this.fail(`malformed number: ${text}`);
    return value;
  }

  private parseArray(): TomlValue[] {
    this.pos += 1;
    const items: TomlValue[] = [];
    for (;;) {
      this.skipIgnorable(true);
      if (this.peek() === "") this.fail("unterminated array");
      if (this.peek() === "]") {
        this.pos += 1;
        return items;
      }
      items.push(this.parseValue());
      this.skipIgnorable(true);
      if (this.peek() === ",") {
        this.pos += 1;
        continue;
      }
      if (this.peek() === "]") {
        this.pos += 1;
        return items;
      }
      if (this.peek() === "") this.fail("unterminated array");
      this.fail(`expected ',' or ']' in array, found ${JSON.stringify(this.peek())}`);
    }
  }

  private parseInlineTable(): TomlTable {
    this.pos += 1;
    const table: TomlTable = {};
    const seen = new Set<string>();
    this.skipIgnorable(true);
    if (this.peek() === "}") {
      this.pos += 1;
      return table;
    }
    for (;;) {
      this.skipIgnorable(true);
      if (this.peek() === "") this.fail("unterminated inline table");
      if (this.peek() === "}") this.fail("trailing comma is not allowed in an inline table");
      const parts = this.parseKeyPath();
      this.skipSpaces();
      if (this.peek() !== "=") this.fail(`expected '=' after key ${parts.join(".")}`);
      this.pos += 1;
      this.skipIgnorable(false);
      const value = this.parseValue();
      let target = table;
      for (let i = 0; i < parts.length - 1; i += 1) {
        let child = target[parts[i]];
        if (child === undefined) {
          child = {};
          target[parts[i]] = child;
        }
        if (typeof child !== "object" || Array.isArray(child)) {
          this.fail(`${parts.join(".")} redefines an existing value`);
        }
        target = child as TomlTable;
      }
      const key = parts.join(".");
      if (seen.has(key)) this.fail(`duplicate key in inline table: ${key}`);
      seen.add(key);
      const last = parts[parts.length - 1];
      if (last in target) this.fail(`duplicate key in inline table: ${key}`);
      target[last] = value;
      this.skipIgnorable(true);
      if (this.peek() === ",") {
        this.pos += 1;
        continue;
      }
      if (this.peek() === "}") {
        this.pos += 1;
        return table;
      }
      if (this.peek() === "") this.fail("unterminated inline table");
      this.fail(`expected ',' or '}' in inline table, found ${JSON.stringify(this.peek())}`);
    }
  }

  private parseBasicString(): string {
    this.pos += 1;
    let out = "";
    for (;;) {
      const c = this.peek();
      if (c === "" || c === "\n") this.fail("unterminated string");
      if (c === '"') {
        this.pos += 1;
        return out;
      }
      if (c === "\\") {
        this.pos += 1;
        out += this.parseEscape(false);
        continue;
      }
      out += this.take();
    }
  }

  private parseLiteralString(): string {
    this.pos += 1;
    const end = this.src.indexOf("'", this.pos);
    const newline = this.src.indexOf("\n", this.pos);
    if (end === -1 || (newline !== -1 && newline < end)) this.fail("unterminated literal string");
    const out = this.src.slice(this.pos, end);
    this.pos = end + 1;
    return out;
  }

  private parseMultilineBasic(): string {
    this.skipRun(3);
    this.trimLeadingNewline();
    let out = "";
    for (;;) {
      const c = this.peek();
      if (c === "") this.fail("unterminated multi-line string");
      if (c === '"') {
        let quotes = 0;
        while (this.peek(quotes) === '"') quotes += 1;
        if (quotes >= 3) {
          if (quotes > 5) this.fail("too many quotes before the end of a multi-line string");
          out += '"'.repeat(quotes - 3);
          this.pos += quotes;
          return out;
        }
        out += this.skipRun(quotes);
        continue;
      }
      if (c === "\\") {
        this.pos += 1;
        const next = this.peek();
        if (next === "\n" || next === "\r" || next === " " || next === "\t") {
          const save = this.pos;
          const saveLine = this.line;
          this.skipSpaces();
          if (this.peek() === "\n" || (this.peek() === "\r" && this.peek(1) === "\n")) {
            this.skipIgnorable(true);
            continue;
          }
          this.pos = save;
          this.line = saveLine;
        }
        out += this.parseEscape(true);
        continue;
      }
      out += this.take();
    }
  }

  private parseMultilineLiteral(): string {
    this.skipRun(3);
    this.trimLeadingNewline();
    let out = "";
    for (;;) {
      const c = this.peek();
      if (c === "") this.fail("unterminated multi-line literal string");
      if (c === "'") {
        let quotes = 0;
        while (this.peek(quotes) === "'") quotes += 1;
        if (quotes >= 3) {
          if (quotes > 5) this.fail("too many quotes before the end of a multi-line literal string");
          out += "'".repeat(quotes - 3);
          this.pos += quotes;
          return out;
        }
        out += this.skipRun(quotes);
        continue;
      }
      out += this.take();
    }
  }

  private trimLeadingNewline(): void {
    if (this.peek() === "\n") this.take();
    else if (this.peek() === "\r" && this.peek(1) === "\n") {
      this.pos += 1;
      this.take();
    }
  }

  private parseEscape(multiline: boolean): string {
    const c = this.take();
    switch (c) {
      case "b":
        return "\b";
      case "t":
        return "\t";
      case "n":
        return "\n";
      case "f":
        return "\f";
      case "r":
        return "\r";
      case '"':
        return '"';
      case "\\":
        return "\\";
      case "u":
        return this.parseCodepoint(4);
      case "U":
        return this.parseCodepoint(8);
      default:
        this.fail(
          `invalid escape \\${c} in ${multiline ? "multi-line " : ""}string (expected b t n f r " \\ u U)`,
        );
    }
  }

  private parseCodepoint(width: number): string {
    const digits = this.src.slice(this.pos, this.pos + width);
    if (digits.length < width || !/^[0-9A-Fa-f]+$/.test(digits)) {
      this.fail(`invalid unicode escape: \\${width === 4 ? "u" : "U"}${digits}`);
    }
    this.pos += width;
    const code = parseInt(digits, 16);
    if (code > 0x10ffff) this.fail(`unicode escape out of range: ${digits}`);
    return String.fromCodePoint(code);
  }
}

export function parseToml(text: string): TomlTable {
  return new TomlParser(text).parse();
}

function isTable(value: unknown): value is TomlTable {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isArrayOfTables(value: unknown): value is TomlTable[] {
  return Array.isArray(value) && value.length > 0 && value.every(isTable);
}

function formatKey(key: string): string {
  return BARE_KEY.test(key) ? key : formatBasicString(key);
}

function escapeControl(c: string): string {
  const code = c.codePointAt(0) ?? 0;
  return `\\u${code.toString(16).toUpperCase().padStart(4, "0")}`;
}

function formatBasicString(value: string): string {
  let out = '"';
  for (const c of value) {
    if (c === "\\") out += "\\\\";
    else if (c === '"') out += '\\"';
    else if (c === "\b") out += "\\b";
    else if (c === "\t") out += "\\t";
    else if (c === "\n") out += "\\n";
    else if (c === "\f") out += "\\f";
    else if (c === "\r") out += "\\r";
    else if (c < " " || c === "\u007f") out += escapeControl(c);
    else out += c;
  }
  return `${out}"`;
}

function formatMultilineString(value: string): string {
  let out = '"""\n';
  for (const c of value) {
    if (c === "\\") out += "\\\\";
    else if (c === '"') out += '\\"';
    else if (c === "\n") out += "\n";
    else if (c === "\t") out += "\t";
    else if (c < " " || c === "\u007f") out += escapeControl(c);
    else out += c;
  }
  return `${out}"""`;
}

function formatString(value: string): string {
  return value.includes("\n") ? formatMultilineString(value) : formatBasicString(value);
}

function formatNumber(value: number): string {
  if (Number.isNaN(value)) return "nan";
  if (value === Infinity) return "inf";
  if (value === -Infinity) return "-inf";
  if (Number.isInteger(value)) return Object.is(value, -0) ? "0" : String(value);
  return String(value);
}

function formatInline(value: TomlValue): string {
  if (typeof value === "string") return formatString(value);
  if (typeof value === "number") return formatNumber(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (Array.isArray(value)) return `[${value.map(formatInline).join(", ")}]`;
  const entries = Object.entries(value).filter(([, v]) => v !== undefined && v !== null);
  if (entries.length === 0) return "{}";
  return `{ ${entries.map(([k, v]) => `${formatKey(k)} = ${formatInline(v)}`).join(", ")} }`;
}

function emitTable(table: TomlTable, path: string[], out: string[]): void {
  const tables: [string, TomlTable][] = [];
  const arrays: [string, TomlTable[]][] = [];
  for (const [key, value] of Object.entries(table)) {
    if (value === undefined || value === null) continue;
    if (isArrayOfTables(value)) arrays.push([key, value]);
    else if (isTable(value)) tables.push([key, value]);
    else out.push(`${formatKey(key)} = ${formatInline(value)}`);
  }
  for (const [key, value] of tables) {
    const next = path.concat(key);
    const entries = Object.entries(value);
    const onlyTables =
      entries.length > 0 && entries.every(([, v]) => isTable(v) && !isArrayOfTables(v));
    if (onlyTables) {
      emitTable(value, next, out);
      continue;
    }
    if (out.length > 0) out.push("");
    out.push(`[${next.map(formatKey).join(".")}]`);
    emitTable(value, next, out);
  }
  for (const [key, items] of arrays) {
    const next = path.concat(key);
    for (const item of items) {
      if (out.length > 0) out.push("");
      out.push(`[[${next.map(formatKey).join(".")}]]`);
      emitTable(item, next, out);
    }
  }
}

export function stringifyToml(value: TomlTable): string {
  const out: string[] = [];
  emitTable(value, [], out);
  return out.length === 0 ? "" : `${out.join("\n")}\n`;
}

const KINDS: Kind[] = ["http", "graphql", "grpc"];
const STATUS_OPS: StatusOp[] = ["eq", "ne", "lt", "gt"];
const JSON_OPS: JsonOp[] = [
  "eq",
  "ne",
  "exists",
  "absent",
  "contains",
  "matches",
  "lt",
  "gt",
  "len",
];
const HEADER_OPS: HeaderOp[] = ["exists", "absent", "eq", "contains", "matches"];
const DURATION_OPS: DurationOp[] = ["lt", "gt"];
const SCOPES: CaptureScope[] = ["run", "session", "persist"];

function fail(message: string): never {
  throw new Error(message);
}

function requiredString(table: TomlTable, key: string, where: string): string {
  const value = table[key];
  if (value === undefined) fail(`${where}: missing required key "${key}"`);
  if (typeof value !== "string") fail(`${where}: "${key}" must be a string`);
  return value;
}

function optionalString(table: TomlTable, key: string, where: string): string | null {
  const value = table[key];
  if (value === undefined) return null;
  if (typeof value !== "string") fail(`${where}: "${key}" must be a string`);
  return value;
}

function requiredInteger(table: TomlTable, key: string, where: string): number {
  const value = table[key];
  if (value === undefined) fail(`${where}: missing required key "${key}"`);
  if (typeof value !== "number" || !Number.isInteger(value)) {
    fail(`${where}: "${key}" must be an integer`);
  }
  return value;
}

function optionalTable(table: TomlTable, key: string, where: string): TomlTable | null {
  const value = table[key];
  if (value === undefined) return null;
  if (!isTable(value)) fail(`${where}: "${key}" must be a table`);
  return value;
}

function decodePairs(value: TomlValue | undefined, where: string): [string, string][] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) fail(`${where} must be an array of [name, value] pairs`);
  return value.map((entry, i) => {
    if (!Array.isArray(entry) || entry.length !== 2) {
      fail(`${where}[${i}] must be a two-element array of strings`);
    }
    const [name, item] = entry;
    if (typeof name !== "string" || typeof item !== "string") {
      fail(`${where}[${i}] must be a two-element array of strings`);
    }
    return [name, item] as [string, string];
  });
}

function decodeStringArray(value: TomlValue | undefined, where: string): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) fail(`${where} must be an array of strings`);
  return value.map((entry, i) => {
    if (typeof entry !== "string") fail(`${where}[${i}] must be a string`);
    return entry;
  });
}

function oneOf<T extends string>(value: string, allowed: readonly T[], where: string): T {
  if ((allowed as readonly string[]).includes(value)) return value as T;
  fail(`${where}: unknown value "${value}" (expected ${allowed.join(", ")})`);
}

function decodeAuth(value: TomlValue | undefined): Auth {
  if (value === undefined) return { type: "none" };
  if (!isTable(value)) fail(`auth must be a table`);
  const type = requiredString(value, "type", "auth");
  switch (type) {
    case "none":
      return { type: "none" };
    case "bearer":
      return { type: "bearer", token: requiredString(value, "token", "auth") };
    case "basic":
      return {
        type: "basic",
        username: requiredString(value, "username", "auth"),
        password: requiredString(value, "password", "auth"),
      };
    case "apikey":
      return {
        type: "apikey",
        key: requiredString(value, "key", "auth"),
        value: requiredString(value, "value", "auth"),
        placement: oneOf(
          requiredString(value, "placement", "auth"),
          ["header", "query"] as const,
          "auth.placement",
        ),
      };
    default:
      fail(`auth: unknown type "${type}" (expected none, bearer, basic, apikey)`);
  }
}

function decodeAssertion(entry: TomlValue, index: number): TestAssertion {
  const where = `tests[${index}]`;
  if (!isTable(entry)) fail(`${where} must be a table`);
  const kind = requiredString(entry, "kind", where);
  switch (kind) {
    case "status":
      return {
        kind: "status",
        op: oneOf(requiredString(entry, "op", where), STATUS_OPS, `${where}.op`),
        value: requiredInteger(entry, "value", where),
      };
    case "duration":
      return {
        kind: "duration",
        op: oneOf(requiredString(entry, "op", where), DURATION_OPS, `${where}.op`),
        value: requiredInteger(entry, "value", where),
      };
    case "json": {
      const assertion: Extract<TestAssertion, { kind: "json" }> = {
        kind: "json",
        path: requiredString(entry, "path", where),
        op: oneOf(requiredString(entry, "op", where), JSON_OPS, `${where}.op`),
      };
      if (entry.value !== undefined) assertion.value = entry.value;
      return assertion;
    }
    case "header": {
      const assertion: Extract<TestAssertion, { kind: "header" }> = {
        kind: "header",
        name: requiredString(entry, "name", where),
        op: oneOf(requiredString(entry, "op", where), HEADER_OPS, `${where}.op`),
      };
      const value = optionalString(entry, "value", where);
      if (value !== null) assertion.value = value;
      return assertion;
    }
    default:
      fail(`${where}: unknown assertion kind "${kind}" (expected status, json, header, duration)`);
  }
}

function decodeTests(value: TomlValue | undefined): TestAssertion[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) fail("tests must be an array of tables");
  return value.map(decodeAssertion);
}

function decodeCaptures(value: TomlValue | undefined): Capture[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) fail("captures must be an array of tables");
  return value.map((entry, i) => {
    const where = `captures[${i}]`;
    if (!isTable(entry)) fail(`${where} must be a table`);
    return {
      from: requiredString(entry, "from", where),
      into: requiredString(entry, "into", where),
      scope: oneOf(requiredString(entry, "scope", where), SCOPES, `${where}.scope`),
    };
  });
}

function decodeScripts(value: TomlValue | undefined): Scripts {
  if (value === undefined) return { pre: null, post: null };
  if (!isTable(value)) fail("scripts must be a table");
  return {
    pre: optionalString(value, "pre", "scripts"),
    post: optionalString(value, "post", "scripts"),
  };
}

function decodeGrpc(value: TomlTable | null): SavedGrpc | null {
  if (value === null) return null;
  return {
    protoPaths: decodeStringArray(value.protoPaths, "grpc.protoPaths"),
    service: requiredString(value, "service", "grpc"),
    method: requiredString(value, "method", "grpc"),
    message: requiredString(value, "message", "grpc"),
    metadata: decodePairs(value.metadata, "grpc.metadata"),
  };
}

export function decodeRequest(text: string): SavedRequest {
  const table = parseToml(text);
  const kind = oneOf(requiredString(table, "kind", "request"), KINDS, "request.kind");
  const graphql = optionalTable(table, "graphql", "request");
  return {
    id: requiredString(table, "id", "request"),
    name: requiredString(table, "name", "request"),
    kind,
    method: requiredString(table, "method", "request"),
    url: requiredString(table, "url", "request"),
    description: optionalString(table, "description", "request"),
    body: optionalString(table, "body", "request"),
    headers: decodePairs(table.headers, "headers"),
    auth: decodeAuth(table.auth),
    graphql:
      graphql === null
        ? null
        : {
            query: requiredString(graphql, "query", "graphql"),
            variables: requiredString(graphql, "variables", "graphql"),
          },
    grpc: decodeGrpc(optionalTable(table, "grpc", "request")),
    scripts: decodeScripts(table.scripts),
    tests: decodeTests(table.tests),
    captures: decodeCaptures(table.captures),
  };
}

function authTable(auth: Auth): TomlTable {
  switch (auth.type) {
    case "none":
      return { type: "none" };
    case "bearer":
      return { type: "bearer", token: auth.token };
    case "basic":
      return { type: "basic", username: auth.username, password: auth.password };
    case "apikey":
      return {
        type: "apikey",
        key: auth.key,
        value: auth.value,
        placement: auth.placement,
      };
  }
}

function assertionTable(assertion: TestAssertion): TomlTable {
  switch (assertion.kind) {
    case "status":
    case "duration":
      return { kind: assertion.kind, op: assertion.op, value: assertion.value };
    case "json": {
      const out: TomlTable = { kind: "json", path: assertion.path, op: assertion.op };
      if (assertion.value !== undefined && assertion.value !== null) {
        out.value = assertion.value as TomlValue;
      }
      return out;
    }
    case "header": {
      const out: TomlTable = { kind: "header", name: assertion.name, op: assertion.op };
      if (assertion.value !== undefined && assertion.value !== null) out.value = assertion.value;
      return out;
    }
  }
}

export function encodeRequest(request: SavedRequest): string {
  oneOf(request.kind, KINDS, "request.kind");
  const table: TomlTable = {
    id: request.id,
    name: request.name,
    kind: request.kind,
    method: request.method,
    url: request.url,
  };
  if (request.description !== undefined && request.description !== null) {
    table.description = request.description;
  }
  if (request.body !== undefined && request.body !== null) table.body = request.body;
  table.headers = (request.headers ?? []).map(([k, v]) => [k, v]);
  table.auth = authTable(request.auth ?? { type: "none" });
  if (request.graphql) {
    table.graphql = { query: request.graphql.query, variables: request.graphql.variables };
  }
  if (request.grpc) {
    table.grpc = {
      protoPaths: request.grpc.protoPaths.slice(),
      service: request.grpc.service,
      method: request.grpc.method,
      message: request.grpc.message,
      metadata: request.grpc.metadata.map(([k, v]) => [k, v]),
    };
  }
  const scripts = request.scripts;
  if (scripts && (scripts.pre !== null || scripts.post !== null)) {
    const out: TomlTable = {};
    if (scripts.pre !== null && scripts.pre !== undefined) out.pre = scripts.pre;
    if (scripts.post !== null && scripts.post !== undefined) out.post = scripts.post;
    table.scripts = out;
  }
  if (request.tests && request.tests.length > 0) table.tests = request.tests.map(assertionTable);
  if (request.captures && request.captures.length > 0) {
    table.captures = request.captures.map((c) => ({ from: c.from, into: c.into, scope: c.scope }));
  }
  return stringifyToml(table);
}

export const ENV_SCHEMA_VERSION = 1;

export type VarDef =
  | { secret: false; value: string }
  | { secret: true; hosts: string[] };

export interface EnvDoc {
  name: string;
  vars: Record<string, VarDef>;
}

function decodeVarDef(key: string, raw: TomlValue): VarDef {
  if (typeof raw === "string") return { secret: false, value: raw };
  if (!isTable(raw))
    fail(`environment: vars.${key} must be a string or a table`);
  const value = optionalString(raw, "value", `vars.${key}`);
  const secret = raw.secret;
  if (secret !== undefined && typeof secret !== "boolean")
    fail(`environment: vars.${key}.secret must be a boolean`);
  const hosts =
    raw.hosts === undefined
      ? null
      : decodeStringArray(raw.hosts, `vars.${key}.hosts`);
  if (secret === true) {
    if (value !== null)
      fail(
        `environment: variable "${key}" is declared secret = true and also carries a value; secret values never live in a workspace file — delete the value and store it with Set value`,
      );
    return { secret: true, hosts: (hosts ?? []).map((h) => h.toLowerCase()) };
  }
  if (value === null)
    fail(
      `environment: variable "${key}" has neither a value nor secret = true`,
    );
  if (hosts !== null)
    fail(
      `environment: variable "${key}" binds hosts but is not declared secret = true; a host-bound value belongs in the keychain, not in this file`,
    );
  return { secret: false, value };
}

export function decodeEnvDoc(name: string, text: string): EnvDoc {
  const table = parseToml(text);
  const version = table.schema_version;
  if (version !== undefined) {
    if (typeof version !== "number" || !Number.isInteger(version))
      fail(`environment: "schema_version" must be an integer`);
    if (version !== ENV_SCHEMA_VERSION)
      fail(
        `environment: unsupported environment schema_version ${version} (expected ${ENV_SCHEMA_VERSION})`,
      );
  }
  const declared = optionalString(table, "name", "environment");
  const vars: Record<string, VarDef> = {};
  const raw = optionalTable(table, "vars", "environment");
  if (raw) {
    for (const [key, value] of Object.entries(raw))
      vars[key] = decodeVarDef(key, value);
  }
  return { name: declared ?? name, vars };
}

export function encodeEnvDoc(doc: EnvDoc): string {
  const vars: TomlTable = {};
  for (const key of Object.keys(doc.vars).sort()) {
    const def = doc.vars[key];
    vars[key] = def.secret
      ? { secret: true, hosts: def.hosts.slice() }
      : { value: def.value };
  }
  return stringifyToml({
    schema_version: ENV_SCHEMA_VERSION,
    name: doc.name,
    vars,
  });
}

export function plainViewOf(doc: EnvDoc): Environment {
  const vars: Record<string, string> = {};
  for (const [key, def] of Object.entries(doc.vars)) {
    if (!def.secret) vars[key] = def.value;
  }
  return { name: doc.name, vars };
}

export function decodeManifest(text: string): {
  schemaVersion: number;
  id?: string;
  name: string;
} {
  const table = parseToml(text);
  const schemaVersion = requiredInteger(table, "schema_version", "manifest");
  const name = requiredString(table, "name", "manifest");
  const id = optionalString(table, "id", "manifest");
  return id === null ? { schemaVersion, name } : { schemaVersion, id, name };
}

export function encodeCollectionManifest(m: {
  schemaVersion: number;
  id: string;
  name: string;
}): string {
  return stringifyToml({ schema_version: m.schemaVersion, id: m.id, name: m.name });
}

export function encodeWorkspaceManifest(m: {
  schemaVersion: number;
  name: string;
  id?: string;
}): string {
  const table: TomlTable = { schema_version: m.schemaVersion };
  if (m.id !== undefined) table.id = m.id;
  table.name = m.name;
  return stringifyToml(table);
}
