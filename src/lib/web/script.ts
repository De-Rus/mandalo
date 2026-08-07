import type { ScriptContext, ScriptOutcome } from "../api";
import { LIMITS } from "./limits.generated";

interface WorkerReply {
  ok: boolean;
  outcome?: ScriptOutcome;
  error?: string;
}

function explain(error: string): string {
  if (/eval|unsafe-eval|Content Security Policy/i.test(error))
    return `${error} — this browser's content security policy blocked the script engine. Scripts run normally in the desktop app.`;
  // A worker cannot be given a heap cap the way rquickjs can, so the browser reports
  // the budget it could not enforce instead of pretending it did.
  if (/out of memory|Array buffer allocation failed|Maximum call stack/i.test(error))
    return `${error} — the desktop engine caps a script at ${LIMITS.memoryBytes} bytes; a web page cannot cap it, only report it.`;
  return error;
}

export function webExecuteScript(
  source: string,
  context: ScriptContext,
): Promise<ScriptOutcome> {
  const url = `${import.meta.env.BASE_URL}script-worker.js`;
  return new Promise((resolve, reject) => {
    let worker: Worker;
    try {
      worker = new Worker(url);
    } catch (e) {
      reject(
        new Error(
          `script engine failed to start: ${e instanceof Error ? e.message : String(e)}`,
        ),
      );
      return;
    }

    const stop = () => {
      clearTimeout(timer);
      worker.terminate();
    };

    const timer = setTimeout(() => {
      stop();
      reject(new Error(`script exceeded ${LIMITS.timeoutMs}ms`));
    }, LIMITS.timeoutMs);

    worker.onmessage = (event: MessageEvent<WorkerReply>) => {
      stop();
      const reply = event.data;
      if (reply.ok && reply.outcome) resolve(reply.outcome);
      else reject(new Error(explain(reply.error ?? "script failed")));
    };

    worker.onerror = (event) => {
      stop();
      reject(new Error(explain(event.message || "script worker failed to load")));
    };

    const eventName = context.response ? "test" : "prerequest";
    worker.postMessage({ source, context: { ...context, eventName } });
  });
}
