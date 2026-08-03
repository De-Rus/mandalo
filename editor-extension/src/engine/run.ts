import type { RequestOutcome, RunResult, SendResult, TestOutcome } from "../core/cli";
import type { RequestModel } from "../core/model";
import { applyCaptures, evaluateTests, type ResponseFacts } from "./assert";
import { UnsupportedJsonPathError } from "./jsonpath";
import { UnresolvedVarError } from "./interpolate";
import { prepare, UnsupportedError } from "./prepare";
import { dynamicNames, executeScript, resolveDynamic, type ScriptRequest } from "./sandbox";
import { NetworkError, send } from "./transport";

/** Raised when a request needs something only the Rust CLI can do. */
export class EscalateError extends Error {
  constructor(readonly reason: string) {
    super(reason);
    this.name = "EscalateError";
  }
}

export interface EngineRequest {
  model: RequestModel;
  relPath: string;
}

export interface EngineOptions {
  log?: (line: string) => void;
}

function templatesOf(model: RequestModel): string[] {
  const out = [model.url, ...model.headers.flat()];
  if (model.body !== undefined) out.push(model.body);
  if (model.graphql) out.push(model.graphql.query, model.graphql.variables);
  const auth = model.auth;
  if (auth.type === "bearer") out.push(auth.token);
  if (auth.type === "basic") out.push(auth.username, auth.password);
  if (auth.type === "apikey") out.push(auth.key, auth.value);
  return out;
}

function guard(error: unknown): never {
  if (error instanceof UnresolvedVarError) {
    throw new EscalateError(
      `the variable {{${error.key}}} is not in the selected environment — if it is a secret, only the CLI can read it from the OS keychain`,
    );
  }
  if (error instanceof UnsupportedJsonPathError) throw new EscalateError(error.message);
  if (error instanceof UnsupportedError) throw new EscalateError(error.message);
  throw error;
}

export async function runRequest(
  request: EngineRequest,
  vars: Record<string, string>,
  options: EngineOptions = {},
): Promise<RequestOutcome> {
  const { model, relPath } = request;
  if (model.kind === "grpc") {
    throw new EscalateError(
      "gRPC needs HTTP/2 trailers, which the editor's Node runtime cannot read — it runs through the Mándalo CLI",
    );
  }

  const started = performance.now();
  const outcome: RequestOutcome = {
    path: relPath,
    name: model.name,
    method: model.method,
    url: model.url,
    response: null,
    grpc: null,
    tests: [],
    captures: [],
    logs: [],
    passed: false,
    durationMs: 0,
    error: null,
    errorCode: null,
  };
  const scriptTests: TestOutcome[] = [];

  const dynamic = dynamicNames(templatesOf(model));
  const scope: Record<string, string> = { ...vars, ...(await resolveDynamic(dynamic)) };

  let wire: ScriptRequest = {
    method: model.method,
    url: model.url,
    headers: model.headers,
    body: model.body ?? null,
  };

  const pre = model.scripts.pre?.trim();
  if (pre !== undefined && pre !== "") {
    options.log?.(`  script: pre (${model.name})`);
    const result = await executeScript(model.scripts.pre as string, {
      vars: scope,
      requestName: model.name,
      request: wire,
      response: null,
    });
    applyWrites(scope, result.varSets, result.varUnsets);
    pushScriptTests(scriptTests, result.tests);
    outcome.logs.push(...result.logs);
    if (result.requestPatch) wire = result.requestPatch;
    outcome.method = wire.method;
    outcome.url = wire.url;
  }

  let prepared;
  try {
    prepared = prepare(model, wire, scope);
  } catch (error) {
    guard(error);
  }
  outcome.method = prepared.method;
  outcome.url = prepared.url;

  const response = await send(prepared);
  outcome.response = response;

  const facts: ResponseFacts = {
    status: response.status,
    headers: response.headers,
    body: response.body,
    durationMs: response.durationMs,
  };

  const post = model.scripts.post?.trim();
  if (post !== undefined && post !== "") {
    options.log?.(`  script: post (${model.name})`);
    const result = await executeScript(model.scripts.post as string, {
      vars: scope,
      requestName: model.name,
      request: wire,
      response: {
        status: response.status,
        statusText: response.statusText,
        headers: response.headers,
        body: response.body,
        durationMs: response.durationMs,
      },
    });
    applyWrites(scope, result.varSets, result.varUnsets);
    pushScriptTests(scriptTests, result.tests);
    outcome.logs.push(...result.logs);
  }

  try {
    outcome.tests = evaluateTests(model.tests, facts).map((result, index) => ({
      id: `test:${index}`,
      name: result.name,
      kind: "assertion" as const,
      passed: result.passed,
      detail: result.detail,
    }));
    outcome.captures = applyCaptures(model.captures, facts, scope);
  } catch (error) {
    guard(error);
  }

  outcome.tests = [...outcome.tests, ...scriptTests];
  for (const [key, value] of Object.entries(scope)) vars[key] = value;

  outcome.passed = outcome.tests.every((test) => test.passed);
  outcome.durationMs = Math.round(performance.now() - started);
  return outcome;
}

function pushScriptTests(
  into: TestOutcome[],
  tests: readonly { name: string; passed: boolean; error: string | null }[],
): void {
  for (const test of tests) {
    into.push({
      id: `script:${into.length}`,
      name: test.name,
      kind: "script",
      passed: test.passed,
      detail: test.error,
    });
  }
}

function applyWrites(
  scope: Record<string, string>,
  sets: Record<string, string>,
  unsets: readonly string[],
): void {
  for (const [key, value] of Object.entries(sets)) scope[key] = value;
  for (const key of unsets) delete scope[key];
}

export async function runOne(
  request: EngineRequest,
  collection: string,
  env: string | null,
  vars: Record<string, string>,
  options: EngineOptions = {},
): Promise<SendResult> {
  const outcome = await runRequest(request, vars, options);
  return { ...outcome, collection, env };
}

export async function runMany(
  requests: readonly EngineRequest[],
  collection: string,
  env: string | null,
  vars: Record<string, string>,
  options: EngineOptions = {},
): Promise<RunResult> {
  const started = performance.now();
  const scope = { ...vars };
  const outcomes: RequestOutcome[] = [];
  for (const request of requests) {
    try {
      outcomes.push(await runRequest(request, scope, options));
    } catch (error) {
      if (error instanceof EscalateError) throw error;
      outcomes.push(failed(request, error));
    }
  }
  const failedCount = outcomes.filter((outcome) => !outcome.passed).length;
  return {
    collection,
    env,
    total: outcomes.length,
    passed: outcomes.length - failedCount,
    failed: failedCount,
    durationMs: Math.round(performance.now() - started),
    requests: outcomes,
  };
}

export function failed(request: EngineRequest, error: unknown): RequestOutcome {
  return {
    path: request.relPath,
    name: request.model.name,
    method: request.model.method,
    url: request.model.url,
    response: null,
    grpc: null,
    tests: [],
    captures: [],
    logs: [],
    passed: false,
    durationMs: 0,
    error: error instanceof Error ? error.message : String(error),
    errorCode: error instanceof NetworkError ? "E_NETWORK" : null,
  };
}
