import type { ScriptContext, ScriptOutcome } from "../api";

const TIMEOUT_MS = 2000;

interface WorkerReply {
  ok: boolean;
  outcome?: ScriptOutcome;
  error?: string;
}

function explain(error: string): string {
  if (/eval|unsafe-eval|Content Security Policy/i.test(error))
    return `${error} — this browser's content security policy blocked the script engine. Scripts run normally in the desktop app.`;
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
      reject(new Error(`script exceeded ${TIMEOUT_MS}ms`));
    }, TIMEOUT_MS);

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
