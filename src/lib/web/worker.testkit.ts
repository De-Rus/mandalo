/// <reference types="node" />
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Worker as NodeWorker } from "node:worker_threads";

const HERE = dirname(fileURLToPath(new URL(import.meta.url)));
const GENERATOR = resolve(HERE, "gen-worker.mjs");
const GENERATED = resolve(HERE, "public/script-worker.js");

/**
 * The real `script-worker.js` the browser build ships, running under Node.
 *
 * jsdom has no Worker, and a `new Function` stand-in would leave the generated worker,
 * its seal and `script.ts` itself untested — which is exactly how the seal list drifted
 * from the extension's. This installs a `Worker` that boots the generated file byte for
 * byte inside a worker thread, with `self` mapped onto the thread's global.
 */
const SHIM = (source: string): string => `
const { parentPort } = require("node:worker_threads");
globalThis.self = globalThis;
globalThis.postMessage = (message) => parentPort.postMessage(message);
const onmessage = (handler) => parentPort.on("message", (data) => handler({ data }));
${source}
onmessage(globalThis.onmessage);
`;

export function generatedWorkerSource(): string {
  if (!existsSync(GENERATED)) execFileSync(process.execPath, [GENERATOR], { stdio: "ignore" });
  return readFileSync(GENERATED, "utf8");
}

class ThreadWorker {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;

  private readonly worker: NodeWorker;

  constructor() {
    this.worker = new NodeWorker(SHIM(generatedWorkerSource()), { eval: true });
    this.worker.on("message", (data) => this.onmessage?.({ data }));
    this.worker.on("error", (error: Error) => this.onerror?.({ message: error.message }));
    this.worker.unref();
  }

  postMessage(payload: unknown): void {
    this.worker.postMessage(payload);
  }

  terminate(): void {
    void this.worker.terminate();
  }
}

/** Installs the real worker as `globalThis.Worker`, and returns the undo. */
export function useGeneratedWorker(): () => void {
  const previous = globalThis.Worker;
  globalThis.Worker = ThreadWorker as unknown as typeof Worker;
  return () => {
    globalThis.Worker = previous;
  };
}
