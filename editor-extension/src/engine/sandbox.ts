import { Worker } from "node:worker_threads";
import { LIMITS, WORKER_SOURCE } from "./prelude.generated";

export interface ScriptRequest {
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
}

export interface ScriptResponse {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: string;
  durationMs: number;
}

export interface ScriptContext {
  vars: Record<string, string>;
  requestName: string;
  request: ScriptRequest;
  response: ScriptResponse | null;
}

export interface ScriptTest {
  name: string;
  passed: boolean;
  error: string | null;
}

export interface ScriptOutcome {
  varSets: Record<string, string>;
  varUnsets: string[];
  tests: ScriptTest[];
  logs: string[];
  requestPatch: ScriptRequest | null;
}

export class ScriptError extends Error {
  constructor(
    message: string,
    readonly timedOut: boolean,
  ) {
    super(message);
    this.name = "ScriptError";
  }
}

interface WorkerReply {
  ok: boolean;
  outcome?: ScriptOutcome;
  error?: string;
}

const MEMORY_MB = Math.round(LIMITS.memoryBytes / (1024 * 1024));

export function executeScript(source: string, context: ScriptContext): Promise<ScriptOutcome> {
  const eventName = context.response ? "test" : "prerequest";
  return new Promise((resolve, reject) => {
    const worker = new Worker(WORKER_SOURCE, {
      eval: true,
      resourceLimits: { maxOldGenerationSizeMb: MEMORY_MB },
      stdout: true,
      stderr: true,
    });

    let settled = false;
    const finish = (fn: () => void): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      void worker.terminate();
      fn();
    };

    const timer = setTimeout(() => {
      finish(() => reject(new ScriptError(`script exceeded ${LIMITS.timeoutMs}ms`, true)));
    }, LIMITS.timeoutMs);

    worker.on("message", (reply: WorkerReply) => {
      finish(() => {
        if (reply.ok && reply.outcome) resolve(reply.outcome);
        else reject(new ScriptError(reply.error ?? "script failed", false));
      });
    });

    worker.on("error", (error: NodeJS.ErrnoException) => {
      finish(() => {
        if (error.code === "ERR_WORKER_OUT_OF_MEMORY") {
          reject(
            new ScriptError(`script exceeded the memory limit of ${LIMITS.memoryBytes} bytes`, false),
          );
          return;
        }
        reject(new ScriptError(error.message, false));
      });
    });

    worker.on("exit", (code) => {
      finish(() => reject(new ScriptError(`script engine exited with code ${code}`, false)));
    });

    worker.postMessage({ source, context: { ...context, eventName } });
  });
}

const DYNAMIC_PREFIX = "$";

export function dynamicNames(templates: readonly string[]): string[] {
  const found = new Set<string>();
  for (const template of templates) {
    for (const match of template.matchAll(/\{\{\s*([^}]*?)\s*\}\}/g)) {
      const name = match[1];
      if (name !== undefined && name.startsWith(DYNAMIC_PREFIX)) found.add(name);
    }
  }
  return [...found].sort();
}

// Postman's {{$guid}}-style values have exactly one generator in the codebase: the
// DYNAMIC map inside the Rust prelude. Reaching it through pm.variables.replaceIn keeps
// the extension from growing a second, drifting copy.
export async function resolveDynamic(names: readonly string[]): Promise<Record<string, string>> {
  if (names.length === 0) return {};
  const source = `
    var names = ${JSON.stringify(names)};
    for (var i = 0; i < names.length; i++) {
      pm.variables.set(names[i], pm.variables.replaceIn("{{" + names[i] + "}}"));
    }
  `;
  const outcome = await executeScript(source, {
    vars: {},
    requestName: "",
    request: { method: "GET", url: "", headers: [], body: null },
    response: null,
  });
  return outcome.varSets;
}
