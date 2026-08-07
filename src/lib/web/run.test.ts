import { describe, expect, it, vi } from "vitest";
import type { SavedRequest } from "../api";
import { webRunRequest } from "./run";
import { MemoryVfs } from "./vfs.testkit";

vi.mock("./send", () => ({
  webSend: vi.fn(() =>
    Promise.resolve({
      status: 200,
      statusText: "OK",
      headers: [],
      body: "{}",
      binary: false,
      durationMs: 1,
      sizeBytes: 2,
    }),
  ),
  hostOf: (url: string) => url,
}));

vi.mock("./script", () => ({ webExecuteScript: vi.fn() }));

const PROD =
  'schema_version = 1\nname = "prod"\n\n' +
  '[vars.base]\nvalue = "https://api.acme.com"\n\n' +
  '[vars.token]\nsecret = true\nhosts = ["api.acme.com"]\n';

async function workspace(): Promise<MemoryVfs> {
  const vfs = new MemoryVfs();
  await vfs.write("environments/prod.toml", PROD);
  return vfs;
}

function request(patch: Partial<SavedRequest> = {}): SavedRequest {
  return {
    id: "r1",
    name: "Whoami",
    kind: "http",
    method: "GET",
    url: "{{base}}/me",
    headers: [],
    auth: { type: "none" },
    ...patch,
  } as SavedRequest;
}

describe("the browser run pipeline", () => {
  it("names the secrets file when a request needs a secret", async () => {
    const req = request({ auth: { type: "bearer", token: "{{token}}" } });

    await expect(
      webRunRequest(await workspace(), req, "prod"),
    ).rejects.toThrow(/secrets\.toml/);
  });

  it("sends a request that only uses plain variables", async () => {
    const step = await webRunRequest(await workspace(), request(), "prod");

    expect(step.response?.status).toBe(200);
    expect(step.error).toBeNull();
  });

  // The browser has no assertion engine. Reporting `passed: true` with an empty
  // `tests` array is the one failure mode a run cannot see from its own output.
  it("refuses a request carrying declarative tests instead of reporting a pass", async () => {
    const req = request({
      tests: [
        { kind: "status", op: "eq", value: 999 },
        { kind: "json", path: "$.nope", op: "exists" },
      ],
    });

    await expect(webRunRequest(await workspace(), req, "prod")).rejects.toThrow(
      /2 declarative test\(s\).*report a pass nobody checked/s,
    );
  });

  it("refuses a request carrying captures", async () => {
    const req = request({
      captures: [{ from: "body.$.token", into: "token", scope: "run" }],
    });

    await expect(webRunRequest(await workspace(), req, "prod")).rejects.toThrow(
      /1 capture\(s\)/,
    );
  });

  it("refuses a `< file` body it cannot read", async () => {
    const req = request({ bodyFile: "files/payload.json" });

    await expect(webRunRequest(await workspace(), req, "prod")).rejects.toThrow(
      /cannot read a file off the workspace/,
    );
  });
});
