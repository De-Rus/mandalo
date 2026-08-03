import { spawn } from "node:child_process";

export function cliMissingMessage(binary: string): string {
  return `Mándalo CLI could not be started: nothing executable at "${binary}".`;
}

export class CliMissingError extends Error {
  constructor(readonly binary: string) {
    super(cliMissingMessage(binary));
    this.name = "CliMissingError";
  }
}

/** No binary was found anywhere — bundled, on PATH, or via `mandalo.cliPath`. */
export class CliUnavailableError extends Error {
  constructor(
    message: string,
    readonly platform: string,
    readonly bundledPath: string | undefined,
  ) {
    super(message);
    this.name = "CliUnavailableError";
  }
}

export class CliOutputError extends Error {
  constructor(
    message: string,
    readonly raw: string,
  ) {
    super(message);
    this.name = "CliOutputError";
  }
}

export class CliExitError extends Error {
  constructor(
    readonly code: number,
    readonly stderr: string,
  ) {
    super(stderr.trim() === "" ? `mandalo exited with code ${code}` : stderr.trim());
    this.name = "CliExitError";
  }
}

export interface SpawnResult {
  code: number;
  stdout: string;
  stderr: string;
}

export type SpawnFn = (binary: string, args: string[], cwd?: string) => Promise<SpawnResult>;

export interface CliOptions {
  cliPath: () => string;
  cwd?: () => string | undefined;
  timeoutMs?: () => number;
  log?: (line: string) => void;
  spawnFn?: SpawnFn;
}

export interface ResponseData {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: string;
  binary: boolean;
  durationMs: number;
  sizeBytes: number;
}

export interface GrpcData {
  body: string;
  durationMs: number;
}

export type TestKind = "assertion" | "script";

export interface TestOutcome {
  id: string;
  name: string;
  kind: TestKind;
  passed: boolean;
  detail: string | null;
}

export interface CaptureOutcome {
  from: string;
  into: string;
  value: string;
  scope: string;
}

export interface RequestOutcome {
  path: string;
  name: string;
  method: string;
  url: string;
  response: ResponseData | null;
  grpc: GrpcData | null;
  tests: TestOutcome[];
  captures: CaptureOutcome[];
  logs: string[];
  passed: boolean;
  durationMs: number;
  error: string | null;
  errorCode: string | null;
}

export interface SendResult extends RequestOutcome {
  collection: string;
  env: string | null;
}

export interface RunResult {
  collection: string;
  env: string | null;
  total: number;
  passed: number;
  failed: number;
  durationMs: number;
  requests: RequestOutcome[];
}

export interface LsRequest {
  id: string;
  name: string;
  kind: string;
  method: string;
  path: string;
}

export interface LsFolder {
  name: string;
  path: string;
  folders: LsFolder[];
  requests: LsRequest[];
}

export interface LsCollection {
  id: string;
  slug: string;
  name: string;
  folders: LsFolder[];
  requests: LsRequest[];
}

export interface LsResult {
  collections: LsCollection[];
  skipped: string[];
}

export interface EnvListResult {
  items: { name: string; vars: Record<string, string> }[];
  skipped: string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function fail(what: string, raw: string): never {
  throw new CliOutputError(`mandalo --reporter json produced unexpected output: ${what}`, raw);
}

function parseJson(raw: string): unknown {
  const trimmed = raw.trim();
  if (trimmed === "") fail("empty stdout", raw);
  try {
    return JSON.parse(trimmed);
  } catch (error) {
    fail((error as Error).message, raw);
  }
}

function requireArray(value: unknown, what: string, raw: string): unknown[] {
  if (!Array.isArray(value)) fail(`"${what}" must be an array`, raw);
  return value;
}

function requireNumber(value: unknown, what: string, raw: string): number {
  if (typeof value !== "number") fail(`"${what}" must be a number`, raw);
  return value;
}

function requireString(value: unknown, what: string, raw: string): string {
  if (typeof value !== "string") fail(`"${what}" must be a string`, raw);
  return value;
}

function optString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function readResponse(value: unknown, raw: string): ResponseData | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) fail('"response" must be an object', raw);
  const headers = requireArray(value["headers"] ?? [], "response.headers", raw).map((entry) => {
    if (!Array.isArray(entry) || entry.length !== 2) fail("response.headers entry", raw);
    return [String(entry[0]), String(entry[1])] as [string, string];
  });
  return {
    status: requireNumber(value["status"], "response.status", raw),
    statusText: optString(value["statusText"]) ?? "",
    headers,
    body: optString(value["body"]) ?? "",
    binary: value["binary"] === true,
    durationMs: requireNumber(value["durationMs"] ?? 0, "response.durationMs", raw),
    sizeBytes: requireNumber(value["sizeBytes"] ?? 0, "response.sizeBytes", raw),
  };
}

function readGrpc(value: unknown, raw: string): GrpcData | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) fail('"grpc" must be an object', raw);
  return {
    body: optString(value["body"]) ?? "",
    durationMs: requireNumber(value["durationMs"] ?? 0, "grpc.durationMs", raw),
  };
}

// `id` is the stable identity the Testing panel keys assertions off; matching by name
// silently collapses duplicate assertion names, so a payload without it is a hard error.
function readTests(value: unknown, raw: string): TestOutcome[] {
  return requireArray(value ?? [], "tests", raw).map((entry) => {
    if (!isRecord(entry)) fail("tests entry must be an object", raw);
    const kind = optString(entry["kind"]) ?? "assertion";
    if (kind !== "assertion" && kind !== "script") fail(`tests[].kind "${kind}"`, raw);
    return {
      id: requireString(entry["id"], "tests[].id", raw),
      name: requireString(entry["name"], "tests[].name", raw),
      kind,
      passed: entry["passed"] === true,
      detail: optString(entry["detail"]),
    };
  });
}

function readCaptures(value: unknown, raw: string): CaptureOutcome[] {
  return requireArray(value ?? [], "captures", raw).map((entry) => {
    if (!isRecord(entry)) fail("captures entry must be an object", raw);
    return {
      from: optString(entry["from"]) ?? "",
      into: requireString(entry["into"], "captures[].into", raw),
      value: optString(entry["value"]) ?? "",
      scope: optString(entry["scope"]) ?? "run",
    };
  });
}

function readRequestOutcome(value: unknown, raw: string): RequestOutcome {
  if (!isRecord(value)) fail("request outcome must be an object", raw);
  const tests = readTests(value["tests"], raw);
  const error = optString(value["error"]);
  return {
    path: requireString(value["path"], "path", raw),
    name: optString(value["name"]) ?? "",
    method: optString(value["method"]) ?? "",
    url: optString(value["url"]) ?? "",
    response: readResponse(value["response"], raw),
    grpc: readGrpc(value["grpc"], raw),
    tests,
    captures: readCaptures(value["captures"], raw),
    logs: requireArray(value["logs"] ?? [], "logs", raw).map(String),
    passed:
      typeof value["passed"] === "boolean"
        ? value["passed"]
        : error === null && tests.every((test) => test.passed),
    durationMs: requireNumber(value["durationMs"] ?? 0, "durationMs", raw),
    error,
    errorCode: optString(value["errorCode"]),
  };
}

function readLsFolder(value: unknown, raw: string): LsFolder {
  if (!isRecord(value)) fail("folder must be an object", raw);
  return {
    name: requireString(value["name"], "folders[].name", raw),
    path: requireString(value["path"], "folders[].path", raw),
    folders: requireArray(value["folders"] ?? [], "folders", raw).map((f) => readLsFolder(f, raw)),
    requests: requireArray(value["requests"] ?? [], "requests", raw).map((r) => {
      if (!isRecord(r)) fail("request must be an object", raw);
      return {
        id: optString(r["id"]) ?? "",
        name: requireString(r["name"], "requests[].name", raw),
        kind: optString(r["kind"]) ?? "http",
        method: optString(r["method"]) ?? "",
        path: requireString(r["path"], "requests[].path", raw),
      };
    }),
  };
}

function defaultSpawn(timeoutMs: number): SpawnFn {
  return (binary, args, cwd) =>
    new Promise((resolve, reject) => {
      const child = spawn(binary, args, { cwd, shell: false });
      let stdout = "";
      let stderr = "";
      const timer = setTimeout(() => {
        child.kill();
        reject(new Error(`mandalo timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      child.stdout.on("data", (chunk) => {
        stdout += String(chunk);
      });
      child.stderr.on("data", (chunk) => {
        stderr += String(chunk);
      });
      child.on("error", (error) => {
        clearTimeout(timer);
        if ((error as NodeJS.ErrnoException).code === "ENOENT") {
          reject(new CliMissingError(binary));
          return;
        }
        reject(error);
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        resolve({ code: code ?? 0, stdout, stderr });
      });
    });
}

export class MandaloCli {
  constructor(private readonly options: CliOptions) {}

  async ls(workspace: string): Promise<LsResult> {
    const raw = await this.json(["ls", "--workspace", workspace, "--reporter", "json"]);
    const value = parseJson(raw);
    if (!isRecord(value)) fail("top level must be an object", raw);
    return {
      collections: requireArray(value["collections"], "collections", raw).map((entry) => {
        if (!isRecord(entry)) fail("collection must be an object", raw);
        const folder = readLsFolder({ name: "", path: "", ...entry }, raw);
        return {
          id: optString(entry["id"]) ?? "",
          slug: requireString(entry["slug"], "collections[].slug", raw),
          name: requireString(entry["name"], "collections[].name", raw),
          folders: folder.folders,
          requests: folder.requests,
        };
      }),
      skipped: requireArray(value["skipped"] ?? [], "skipped", raw).map(String),
    };
  }

  async envList(workspace: string): Promise<EnvListResult> {
    const raw = await this.json(["env", "list", "--workspace", workspace, "--reporter", "json"]);
    const value = parseJson(raw);
    if (!isRecord(value)) fail("top level must be an object", raw);
    return {
      items: requireArray(value["items"], "items", raw).map((entry) => {
        if (!isRecord(entry)) fail("environment must be an object", raw);
        const vars: Record<string, string> = {};
        if (isRecord(entry["vars"])) {
          for (const [key, v] of Object.entries(entry["vars"])) vars[key] = String(v);
        }
        return { name: requireString(entry["name"], "items[].name", raw), vars };
      }),
      skipped: requireArray(value["skipped"] ?? [], "skipped", raw).map(String),
    };
  }

  async send(
    workspace: string,
    collection: string,
    requestPath: string,
    env?: string,
  ): Promise<SendResult> {
    const args = ["send", collection, requestPath, "--workspace", workspace, "--reporter", "json"];
    if (env) args.push("--env", env);
    const raw = await this.json(args);
    const value = parseJson(raw);
    if (!isRecord(value)) fail("top level must be an object", raw);
    const outcome = readRequestOutcome({ path: requestPath, ...value }, raw);
    return { ...outcome, collection: optString(value["collection"]) ?? collection, env: optString(value["env"]) };
  }

  async run(
    workspace: string,
    collection: string,
    options: { folder?: string | undefined; env?: string | undefined; failFast?: boolean } = {},
  ): Promise<RunResult> {
    const args = ["run", collection, "--workspace", workspace, "--reporter", "json"];
    if (options.folder) args.push("--folder", options.folder);
    if (options.env) args.push("--env", options.env);
    if (options.failFast) args.push("--fail-fast");
    const raw = await this.json(args);
    const value = parseJson(raw);
    if (!isRecord(value)) fail("top level must be an object", raw);
    const requests = requireArray(value["requests"], "requests", raw).map((entry) =>
      readRequestOutcome(entry, raw),
    );
    const failed = requests.filter((r) => !r.passed).length;
    return {
      collection: optString(value["collection"]) ?? collection,
      env: optString(value["env"]),
      total: typeof value["total"] === "number" ? value["total"] : requests.length,
      passed: typeof value["passed"] === "number" ? value["passed"] : requests.length - failed,
      failed: typeof value["failed"] === "number" ? value["failed"] : failed,
      durationMs: typeof value["durationMs"] === "number" ? value["durationMs"] : 0,
      requests,
    };
  }

  async version(binary: string): Promise<string | null> {
    const spawnFn = this.options.spawnFn ?? defaultSpawn(10_000);
    try {
      const result = await spawnFn(binary, ["--version"], this.options.cwd?.());
      return result.code === 0 ? result.stdout : null;
    } catch {
      return null;
    }
  }

  // A non-zero exit is normal for `run` when assertions fail, so a well-formed JSON
  // payload on stdout always wins over the exit code.
  private async json(args: string[]): Promise<string> {
    const binary = this.options.cliPath();
    const timeoutMs = this.options.timeoutMs?.() ?? 120_000;
    const spawnFn = this.options.spawnFn ?? defaultSpawn(timeoutMs);
    this.options.log?.(`$ ${binary} ${args.join(" ")}`);
    const result = await spawnFn(binary, args, this.options.cwd?.());
    if (result.code !== 0 && result.stdout.trim() === "") {
      this.options.log?.(result.stderr);
      throw new CliExitError(result.code, result.stderr);
    }
    return result.stdout;
  }
}
