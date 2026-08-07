import type {
  EnvironmentView,
  GrpcSpec,
  RequestSpec,
  SavedRequest,
  ScriptRequest,
  ScriptOutcome,
  StepResult,
} from "../api";
import { webSendGrpc } from "./grpc";
import { webExecuteScript } from "./script";
import { webSend } from "./send";
import type { Vfs } from "./vfs";
import { listEnvironments } from "./workspace";

/**
 * A page cannot read the machine-local secrets file, so a declared secret has no
 * value here. Saying
 * "unresolved variable" would send the user hunting for a typo that does not
 * exist — name the real reason and where the value lives.
 */
function noKeychain(env: string, name: string): Error {
  return new Error(
    `${env}.${name} is a secret kept on the machine that stores it (~/.config/mandalo/secrets.toml), which a web page cannot read. Open this workspace in the Mándalo desktop app to send a request that uses it.`,
  );
}

/**
 * The assertion engine lives in `crates/core`. Reporting a step green when nobody
 * evaluated its `tests` would be a lie the run itself cannot see, so a request that
 * carries any refuses to run here rather than passing silently.
 */
function noAssertionEngine(req: SavedRequest): Error {
  const carried = [
    (req.tests?.length ?? 0) > 0 ? `${req.tests!.length} declarative test(s)` : null,
    (req.captures?.length ?? 0) > 0 ? `${req.captures!.length} capture(s)` : null,
  ].filter((part): part is string => part !== null);
  return new Error(
    `"${req.name}" carries ${carried.join(" and ")}, which a web page has no engine for — it would report a pass nobody checked. Open this workspace in the Mándalo desktop app or run it with the CLI, or assert from a \`> {% … %}\` response script.`,
  );
}

function noBodyFile(req: SavedRequest): Error {
  return new Error(
    `"${req.name}" reads its body from ${req.bodyFile}, and a web page cannot read a file off the workspace. Open this workspace in the Mándalo desktop app or run it with the CLI.`,
  );
}

const VAR = /\{\{\s*([^}\s]+)\s*\}\}/g;

function namesIn(template: string | null | undefined, out: Set<string>): void {
  if (!template) return;
  for (const match of template.matchAll(VAR)) out.add(match[1]);
}

function referenced(req: SavedRequest, wire: ScriptRequest): Set<string> {
  const names = new Set<string>();
  namesIn(wire.method, names);
  namesIn(wire.url, names);
  for (const [key, value] of wire.headers) {
    namesIn(key, names);
    namesIn(value, names);
  }
  namesIn(wire.body, names);
  namesIn(req.graphql?.query, names);
  namesIn(req.graphql?.variables, names);
  namesIn(req.grpc?.message, names);
  for (const [key, value] of req.grpc?.metadata ?? []) {
    namesIn(key, names);
    namesIn(value, names);
  }
  const auth = req.auth;
  if (auth.type === "bearer") namesIn(auth.token, names);
  if (auth.type === "basic") {
    namesIn(auth.username, names);
    namesIn(auth.password, names);
  }
  if (auth.type === "apikey") {
    namesIn(auth.key, names);
    namesIn(auth.value, names);
  }
  return names;
}

function plainVars(view: EnvironmentView | undefined): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [key, info] of Object.entries(view?.vars ?? {}))
    if (!info.secret && info.value !== null) vars[key] = info.value;
  return vars;
}

function secretNames(view: EnvironmentView | undefined): Set<string> {
  return new Set(
    Object.entries(view?.vars ?? {})
      .filter(([, info]) => info.secret)
      .map(([key]) => key),
  );
}

function bodyText(req: SavedRequest): string | null {
  const body = req.body ?? null;
  if (body === null || typeof body === "string") return body;
  if (body.mode === "raw") return body.text;
  throw new Error(
    `"${req.name}" has a ${body.mode} body, and a web page cannot send one. Open this workspace in the Mándalo desktop app or run it with the CLI.`,
  );
}

function wireOf(req: SavedRequest): ScriptRequest {
  return {
    method: req.method,
    url: req.url,
    headers: req.headers ?? [],
    body: bodyText(req),
  };
}

function specOf(
  req: SavedRequest,
  wire: ScriptRequest,
  vars: Record<string, string>,
): RequestSpec {
  return {
    kind: req.kind === "graphql" ? "graphql" : "http",
    method: wire.method,
    url: wire.url,
    headers: wire.headers,
    body: wire.body,
    auth: req.auth,
    graphql: req.graphql ?? null,
    vars,
  };
}

function grpcSpecOf(
  req: SavedRequest,
  wire: ScriptRequest,
  vars: Record<string, string>,
): GrpcSpec {
  if (!req.grpc) throw new Error("grpc request is missing the grpc body");
  return {
    url: wire.url,
    protoPaths: req.grpc.protoPaths,
    service: req.grpc.service,
    method: req.grpc.method,
    message: req.grpc.message,
    metadata: req.grpc.metadata,
    vars,
  };
}

function empty(req: SavedRequest): StepResult {
  return {
    requestName: req.name,
    path: "",
    method: req.method,
    url: req.url,
    response: null,
    grpc: null,
    tests: [],
    scriptTests: [],
    logs: [],
    captured: {},
    captures: [],
    unboundSecrets: [],
    varSets: {},
    varUnsets: [],
    secretVarSets: [],
    passed: false,
    durationMs: 0,
    error: null,
    errorCode: null,
  };
}

function record(
  step: StepResult,
  outcome: ScriptOutcome,
  vars: Record<string, string>,
  secrets: Set<string>,
): void {
  for (const [key, value] of Object.entries(outcome.varSets)) {
    vars[key] = value;
    step.varUnsets = step.varUnsets.filter((u) => u !== key);
    if (secrets.has(key)) {
      if (!step.secretVarSets.includes(key)) step.secretVarSets.push(key);
    } else {
      step.varSets[key] = value;
    }
  }
  for (const key of outcome.varUnsets) {
    delete vars[key];
    delete step.varSets[key];
    if (!step.varUnsets.includes(key)) step.varUnsets.push(key);
  }
  step.scriptTests.push(...outcome.tests);
  step.logs.push(...outcome.logs);
}

/**
 * The browser twin of `Runner::run_one`: same order — pre-script, send,
 * post-script — over the request the editor holds. Declarative tests and
 * captures stay with the desktop app and the CLI, which own the assertion
 * engine; a request that carries any is refused, never reported green.
 */
export async function webRunRequest(
  vfs: Vfs,
  req: SavedRequest,
  env: string | null,
): Promise<StepResult> {
  if ((req.tests?.length ?? 0) > 0 || (req.captures?.length ?? 0) > 0)
    throw noAssertionEngine(req);
  if (req.bodyFile !== undefined && req.bodyFile !== null) throw noBodyFile(req);
  const started = performance.now();
  const step = empty(req);
  const views = env ? (await listEnvironments(vfs)).items : [];
  const view = views.find((e) => e.name === env);
  const vars = plainVars(view);
  const secrets = secretNames(view);
  let wire = wireOf(req);

  if (req.scripts?.pre && req.scripts.pre.trim() !== "") {
    const outcome = await webExecuteScript(req.scripts.pre, {
      vars: { ...vars },
      requestName: req.name,
      request: wire,
      response: null,
    });
    record(step, outcome, vars, secrets);
    if (outcome.requestPatch) wire = outcome.requestPatch;
  }
  step.method = wire.method;
  step.url = wire.url;

  for (const name of referenced(req, wire))
    if (secrets.has(name)) throw noKeychain(env ?? "", name);

  if (req.kind === "grpc") {
    step.grpc = await webSendGrpc(vfs, grpcSpecOf(req, wire, vars));
  } else {
    step.response = await webSend(specOf(req, wire, vars));
  }

  if (req.scripts?.post && req.scripts.post.trim() !== "") {
    const response = step.response;
    const outcome = await webExecuteScript(req.scripts.post, {
      vars: { ...vars },
      requestName: req.name,
      request: wire,
      response: {
        status: response?.status ?? 0,
        statusText: response?.statusText ?? "",
        headers: response?.headers ?? [],
        body: response?.body ?? step.grpc?.body ?? "",
        durationMs: response?.durationMs ?? step.grpc?.durationMs ?? 0,
      },
    });
    record(step, outcome, vars, secrets);
  }

  step.passed = step.scriptTests.every((t) => t.passed);
  step.durationMs = Math.round(performance.now() - started);
  return step;
}
