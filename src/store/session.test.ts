import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StepResult } from "../lib/api";
import { newDraft } from "../lib/draft";
import { useEnv } from "./env";
import { useSession } from "./session";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const RESPONSE = {
  status: 200,
  statusText: "OK",
  headers: [["Content-Type", "application/json"]] as [string, string][],
  body: '{"id":7}',
  binary: false,
  durationMs: 12,
  sizeBytes: 8,
};

function step(patch: Partial<StepResult> = {}): StepResult {
  return {
    requestName: "New Request",
    path: "",
    method: "GET",
    url: "https://x.dev",
    response: RESPONSE,
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
    passed: true,
    durationMs: 12,
    error: null,
    errorCode: null,
    ...patch,
  };
}

function draftAt(url = "https://x.dev") {
  const draft = newDraft();
  draft.url = url;
  return draft;
}

describe("session store send", () => {
  beforeEach(() => {
    invoke.mockReset();
    localStorage.clear();
    useSession.setState({ responses: {} });
    useEnv.setState({
      workspace: "/ws",
      envs: [],
      selected: null,
      error: null,
    });
  });

  it("runs the whole request through one call to the runner", async () => {
    invoke.mockResolvedValueOnce(step());
    const draft = draftAt();
    draft.preScript = "console.log('hi');";
    draft.testScript = "pm.test('ok', () => {});";

    await useSession.getState().send(draft);

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("run_request_draft", {
      workspace: "/ws",
      env: null,
      request: expect.objectContaining({
        url: "https://x.dev",
        scripts: { pre: "console.log('hi');", post: "pm.test('ok', () => {});" },
      }),
    });
    expect(useSession.getState().responses[draft.id].phase).toBe("http");
  });

  it("passes the selected environment to the runner", async () => {
    invoke.mockResolvedValue(step());
    useEnv.setState({
      workspace: "/ws",
      envs: [{ name: "staging", vars: {} }],
      selected: "staging",
      error: null,
    });

    await useSession.getState().send(draftAt());

    expect(invoke.mock.calls[0][1]).toMatchObject({ env: "staging" });
  });

  it("applies every part of the result to the response state", async () => {
    invoke.mockResolvedValueOnce(
      step({
        tests: [{ name: "status is 200", passed: true, detail: null }],
        scriptTests: [{ name: "has a token", passed: false, error: "missing" }],
        logs: ["checked"],
        captured: { token: "t0k" },
        unboundSecrets: [{ name: "token", env: "prod", host: "api.dev" }],
        secretVarSets: ["token"],
      }),
    );
    const draft = draftAt();

    await useSession.getState().send(draft);

    const state = useSession.getState().responses[draft.id];
    if (state.phase !== "http") throw new Error("expected an http response");
    expect(state.data.body).toBe('{"id":7}');
    expect(state.run.tests).toEqual([
      { name: "status is 200", passed: true, detail: null },
    ]);
    expect(state.run.scriptTests).toEqual([
      { name: "has a token", passed: false, detail: "missing" },
    ]);
    expect(state.run.logs).toEqual(["checked"]);
    expect(state.run.captured).toEqual({ token: "t0k" });
    expect(state.run.unboundSecrets).toEqual([
      { name: "token", env: "prod", host: "api.dev" },
    ]);
    expect(state.run.secretVarSets).toEqual(["token"]);
  });

  it("keeps a grpc reply in the grpc phase", async () => {
    invoke.mockResolvedValueOnce(
      step({ response: null, grpc: { body: "{}", durationMs: 3 } }),
    );
    const draft = newDraft("Echo", "grpc");
    draft.url = "http://localhost:50051";
    draft.grpc.protoPaths = "/a.proto";

    await useSession.getState().send(draft);

    expect(invoke.mock.calls[0][1].request).toMatchObject({
      grpc: expect.objectContaining({ protoPaths: ["/a.proto"] }),
    });
    expect(useSession.getState().responses[draft.id].phase).toBe("grpc");
  });

  it("captures invoke string errors as a readable error state", async () => {
    invoke.mockRejectedValueOnce("connection refused");
    const draft = draftAt("https://down.dev");

    await useSession.getState().send(draft);

    expect(useSession.getState().responses[draft.id]).toMatchObject({
      phase: "error",
      message: "connection refused",
    });
  });

  it("shows a failed run as an error and keeps its logs", async () => {
    invoke.mockResolvedValueOnce(
      step({
        response: null,
        error: "prod.token is bound to api.acme.com",
        errorCode: "E_SECRET_HOST_DENIED",
        logs: ["prepared"],
        passed: false,
      }),
    );
    const draft = draftAt();

    await useSession.getState().send(draft);

    expect(useSession.getState().responses[draft.id]).toMatchObject({
      phase: "error",
      message: "prod.token is bound to api.acme.com",
      logs: ["prepared"],
    });
  });

  it("writes script variables into the active environment", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "save_environment") return Promise.resolve("staging");
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [
            {
              name: "staging",
              vars: { userId: { shared: true, secret: false, value: "7", set: true } },
            },
          ],
          skipped: [],
        });
      return Promise.resolve(step({ varSets: { userId: "7" } }));
    });
    useEnv.setState({
      workspace: "/ws",
      envs: [{ name: "staging", vars: {} }],
      selected: "staging",
      error: null,
    });

    await useSession.getState().send(draftAt());

    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: { name: "staging", vars: { userId: "7" } },
    });
  });

  it("drops a variable the run unset from the environment file", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "save_environment") return Promise.resolve("staging");
      if (cmd === "list_environments")
        return Promise.resolve({ items: [], skipped: [] });
      return Promise.resolve(step({ varUnsets: ["stale"] }));
    });
    useEnv.setState({
      workspace: "/ws",
      envs: [
        {
          name: "staging",
          vars: { stale: { shared: true, secret: false, value: "old", set: true } },
        },
      ],
      selected: "staging",
      error: null,
    });

    await useSession.getState().send(draftAt());

    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: { name: "staging", vars: {} },
    });
  });

  it("refuses to write a variable the environment declares secret", async () => {
    invoke.mockResolvedValue(step({ varSets: { token: "leaked" } }));
    useEnv.setState({
      workspace: "/ws",
      envs: [
        {
          name: "staging",
          vars: { token: { shared: false, secret: true, value: null, hosts: [], set: true } },
        },
      ],
      selected: "staging",
      error: null,
    });

    await useSession.getState().send(draftAt());

    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("save_environment");
    expect(useEnv.getState().error).toMatch(/declared secret/);
  });

  it("ignores a second send while one is in flight", async () => {
    let resolveRun!: (value: unknown) => void;
    invoke.mockImplementation(
      () =>
        new Promise((r) => {
          resolveRun = r;
        }),
    );
    const draft = draftAt();

    const first = useSession.getState().send(draft);
    await useSession.getState().send(draft);
    expect(
      invoke.mock.calls.filter((c) => c[0] === "run_request_draft"),
    ).toHaveLength(1);

    resolveRun(step());
    await first;
  });
});
