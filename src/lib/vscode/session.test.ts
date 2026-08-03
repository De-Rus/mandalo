import { describe, expect, it } from "vitest";
import { toResponseState, type CliOutcome } from "./session";

function outcome(overrides: Partial<CliOutcome> = {}): CliOutcome {
  return {
    path: "ping.toml",
    name: "Ping",
    method: "GET",
    url: "https://api.test/ping",
    response: {
      status: 200,
      statusText: "OK",
      headers: [["content-type", "application/json"]],
      body: '{"ok":true}',
      binary: false,
      durationMs: 12,
      sizeBytes: 11,
    },
    grpc: null,
    tests: [],
    captures: [],
    logs: [],
    passed: true,
    durationMs: 12,
    error: null,
    errorCode: null,
    ...overrides,
  };
}

describe("toResponseState", () => {
  it("maps an HTTP outcome onto the pane's http phase", () => {
    const state = toResponseState(outcome());
    expect(state.phase).toBe("http");
    if (state.phase !== "http") throw new Error("unreachable");
    expect(state.data.status).toBe(200);
  });

  it("splits declarative assertions from script tests", () => {
    const state = toResponseState(
      outcome({
        tests: [
          { id: "1", name: "status eq 200", kind: "assertion", passed: true, detail: null },
          { id: "2", name: "pm: body has token", kind: "script", passed: false, detail: "missing" },
        ],
      }),
    );
    if (state.phase !== "http") throw new Error("unreachable");
    expect(state.run.tests).toEqual([{ name: "status eq 200", passed: true, detail: null }]);
    expect(state.run.scriptTests).toEqual([{ name: "pm: body has token", passed: false, detail: "missing" }]);
  });

  it("turns captures into the variables the pane lists", () => {
    const state = toResponseState(
      outcome({ captures: [{ from: "body.$.token", into: "token", value: "abc", scope: "run" }] }),
    );
    if (state.phase !== "http") throw new Error("unreachable");
    expect(state.run.captured).toEqual({ token: "abc" });
  });

  it("surfaces a transport failure as the error phase, logs and all", () => {
    const state = toResponseState(
      outcome({ error: "connection refused", response: null, logs: ["pre-script ran"] }),
    );
    expect(state).toEqual({ phase: "error", message: "connection refused", logs: ["pre-script ran"] });
  });

  it("maps a gRPC outcome onto the grpc phase", () => {
    const state = toResponseState(
      outcome({ response: null, grpc: { body: '{"pong":true}', durationMs: 4 } }),
    );
    expect(state.phase).toBe("grpc");
    if (state.phase !== "grpc") throw new Error("unreachable");
    expect(state.data.body).toBe('{"pong":true}');
  });

  it("refuses to invent a response when the CLI reported none", () => {
    const state = toResponseState(outcome({ response: null }));
    expect(state).toMatchObject({ phase: "error" });
    if (state.phase !== "error") throw new Error("unreachable");
    expect(state.message).toContain("no response");
  });
});
