import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResponseData } from "../lib/api";
import { EMPTY_RUN, type ResponseState, type RunResult } from "../store/session";
import { useEnv } from "../store/env";
import { ResponsePane } from "./ResponsePane";

function data(patch: Partial<ResponseData> = {}): ResponseData {
  return {
    status: 200,
    statusText: "OK",
    headers: [["Content-Type", "application/json"]],
    body: '{"a":1}',
    binary: false,
    durationMs: 12,
    sizeBytes: 7,
    ...patch,
  };
}

function run(patch: Partial<RunResult> = {}): RunResult {
  return { ...EMPTY_RUN, ...patch };
}

function http(
  patch: Partial<ResponseData> = {},
  runPatch: Partial<RunResult> = {},
): ResponseState {
  return { phase: "http", data: data(patch), run: run(runPatch) };
}

const body = () => document.querySelector(".code-view-body")?.textContent ?? "";

describe("ResponsePane", () => {
  afterEach(cleanup);

  it("pretty prints a textual JSON body and shows no binary chip", () => {
    render(<ResponsePane response={http()} findSignal={0} />);

    expect(body()).toContain('"a"');
    expect(
      document.querySelectorAll(".code-view-body > div").length,
    ).toBeGreaterThan(1);
    expect(screen.queryByText("binary")).toBeNull();
    expect(screen.queryByText(/not valid UTF-8/)).toBeNull();
  });

  it("marks a binary body with a chip, a note, and leaves it unformatted", () => {
    render(
      <ResponsePane
        response={http({ binary: true, sizeBytes: 5120 })}
        findSignal={0}
      />,
    );

    expect(screen.getByText("binary")).toBeTruthy();
    expect(screen.getByText(/not valid UTF-8/)).toBeTruthy();
    expect(body()).toBe('{"a":1}');
    expect(screen.getByText("5.0 KB")).toBeTruthy();
  });

  it("switches to the raw body view", () => {
    render(<ResponsePane response={http()} findSignal={0} />);
    fireEvent.click(screen.getByText("Raw"));
    expect(body()).toBe('{"a":1}');
  });

  it("counts matches when searching in the body", () => {
    render(
      <ResponsePane response={http({ body: '{"a":1,"b":"a"}' })} findSignal={0} />,
    );
    fireEvent.change(screen.getByLabelText("Find in body"), {
      target: { value: "a" },
    });
    expect(screen.getByText("1/2")).toBeTruthy();
  });

  it("summarises pm.test results without labelling their source", () => {
    render(
      <ResponsePane
        response={http(
          {},
          {
            scriptTests: [
              { name: "Status code is 200", passed: true, detail: null },
              { name: "Body has an id", passed: false, detail: "expected 1" },
            ],
          },
        )}
        findSignal={0}
      />,
    );

    fireEvent.click(screen.getByText("Test Results"));
    expect(screen.getByText("1 passed")).toBeTruthy();
    expect(screen.getByText("1 failed")).toBeTruthy();
    expect(screen.getByText("expected 1")).toBeTruthy();
    expect(screen.queryByText("assertion")).toBeNull();
    expect(screen.queryByText("script")).toBeNull();
  });

  it("badges the Test Results tab with the failure count", () => {
    render(
      <ResponsePane
        response={http(
          {},
          {
            scriptTests: [
              { name: "a", passed: true, detail: null },
              { name: "b", passed: false, detail: "no" },
            ],
          },
        )}
        findSignal={0}
      />,
    );

    const tab = screen.getByText("Test Results").closest("button");
    expect(tab?.querySelector(".count")?.textContent).toBe("1");
  });

  it("points at the Tests tab when no pm.test ran", () => {
    render(<ResponsePane response={http()} findSignal={0} />);
    fireEvent.click(screen.getByText("Test Results"));
    expect(screen.getByText(/Write pm\.test/)).toBeTruthy();
  });

  it("shows script console output", () => {
    render(
      <ResponsePane
        response={http({}, { logs: ["hello from the script"] })}
        findSignal={0}
      />,
    );
    fireEvent.click(screen.getByText("Console"));
    expect(screen.getByText("hello from the script")).toBeTruthy();
  });

  it("teaches what to do when there is no response yet", () => {
    render(<ResponsePane response={{ phase: "idle" }} findSignal={0} />);
    expect(screen.getByText("No response yet")).toBeTruthy();
  });

  it("offers to bind a secret to the host it just travelled to", async () => {
    const bindHost = vi.fn(() => Promise.resolve());
    useEnv.setState({ selected: "prod", bindHost } as never);
    render(
      <ResponsePane
        response={http(
          {},
          {
            unboundSecrets: [{ name: "token", env: "prod", host: "api.acme.com" }],
          },
        )}
        findSignal={0}
      />,
    );

    expect(screen.getByText(/prod\.token was sent to api\.acme\.com/)).toBeTruthy();
    fireEvent.click(screen.getByText("Bind to api.acme.com"));
    expect(bindHost).toHaveBeenCalledWith("prod", "token", "api.acme.com");
  });

  it("reports a script write to a secret by name, never by value", () => {
    useEnv.setState({ selected: "prod" } as never);
    render(
      <ResponsePane
        response={http({}, { secretVarSets: ["token"] })}
        findSignal={0}
      />,
    );
    expect(screen.getByText(/prod\.token was set by a script/)).toBeTruthy();
  });
});
