import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ResponseData } from "../lib/api";
import { newDraft, type RequestDraft } from "../lib/draft";
import { useCollection } from "../store/collection";
import { EMPTY_RUN, type ResponseState, type RunResult } from "../store/session";
import { useEnv } from "../store/env";
import { useToasts } from "../store/toast";
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

  it("renders the preview mode in a sandboxed frame instead of the code view", () => {
    render(<ResponsePane response={http()} findSignal={0} />);
    fireEvent.click(screen.getByText("Preview"));

    const frame = screen.getByTitle("Response preview") as HTMLIFrameElement;
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("srcdoc")).toBe('{"a":1}');
    expect(document.querySelector(".code-view")).toBeNull();
  });

  it("collapses a JSON block from the gutter and restores it", () => {
    render(<ResponsePane response={http()} findSignal={0} />);

    const collapse = screen.getByLabelText("Collapse lines 1 to 3");
    fireEvent.click(collapse);
    expect(body()).not.toContain('"a"');

    fireEvent.click(screen.getByLabelText("Expand lines 1 to 3"));
    expect(body()).toContain('"a"');
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

  const testsBadge = () =>
    screen.getByText("Test Results").closest("button")?.querySelector(".count");

  it("badges the Test Results tab green when every test passed", () => {
    render(
      <ResponsePane
        response={http(
          {},
          {
            tests: [{ name: "status", passed: true, detail: null }],
            scriptTests: [{ name: "a", passed: true, detail: null }],
          },
        )}
        findSignal={0}
      />,
    );

    expect(testsBadge()?.textContent).toBe("2/2");
    expect(testsBadge()?.classList.contains("count-success")).toBe(true);
  });

  it("badges the Test Results tab red when anything failed", () => {
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

    expect(testsBadge()?.textContent).toBe("1/2");
    expect(testsBadge()?.classList.contains("count-danger")).toBe(true);
  });

  it("shows no badge at all when the request carried no tests", () => {
    render(<ResponsePane response={http()} findSignal={0} />);
    expect(testsBadge()).toBeFalsy();
  });

  it("reads a run that died before its tests as a failure, not a pass", () => {
    render(
      <ResponsePane
        response={http({}, { runError: "ReferenceError: pm is not defined" })}
        findSignal={0}
      />,
    );

    expect(testsBadge()?.textContent).toBe("0/0");
    expect(testsBadge()?.classList.contains("count-danger")).toBe(true);

    fireEvent.click(screen.getByText("Test Results"));
    expect(screen.getByText(/ReferenceError: pm is not defined/)).toBeTruthy();
  });

  it("points at the Post-response Script tab when no pm.test ran", () => {
    render(<ResponsePane response={http()} findSignal={0} />);
    fireEvent.click(screen.getByText("Test Results"));
    // The tab it names has to be the tab that exists: it was called Tests once.
    expect(
      screen.getByText("No tests ran. Write pm.test(…) in the Post-response Script tab of the request."),
    ).toBeTruthy();
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

  it("offers no authoring buttons on a line without an active request", () => {
    useCollection.setState({ activeId: null, drafts: {} });
    render(<ResponsePane response={http()} findSignal={0} />);

    expect(screen.getByLabelText("Copy path body.$.a")).toBeTruthy();
    expect(screen.queryByLabelText(/^Capture/)).toBeNull();
    expect(screen.queryByLabelText(/^Assert/)).toBeNull();
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

function activate(patch: Partial<RequestDraft> = {}): void {
  useCollection.setState({
    workspace: null,
    activeId: "r1",
    drafts: { r1: { ...newDraft("R", "http"), id: "r1", ...patch } },
  });
}

const active = (): RequestDraft => useCollection.getState().drafts.r1;

const toastText = (): string =>
  useToasts
    .getState()
    .items.map((t) => t.text)
    .join(" | ");

describe("ResponsePane leaf actions", () => {
  afterEach(() => {
    cleanup();
    useToasts.setState({ items: [] });
  });

  it("copies the path in the syntax a capture source accepts", () => {
    activate();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(
      <ResponsePane response={http({ body: '{"data":{"id":7}}' })} findSignal={0} />,
    );

    fireEvent.click(screen.getByLabelText("Copy path body.$.data.id"));
    expect(writeText).toHaveBeenCalledWith("body.$.data.id");
    expect(toastText()).toContain("Copied body.$.data.id");
  });

  it("captures a plain value into a variable named after its key", () => {
    activate();
    render(<ResponsePane response={http({ body: '{"userId":7}' })} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Capture body.$.userId into a variable"));
    expect(active().testScript).toBe(
      'pm.environment.set("userId", pm.response.json().userId);',
    );
  });

  it("warns when the captured value looks like a credential", () => {
    activate();
    render(
      <ResponsePane response={http({ body: '{"accessToken":"abc"}' })} findSignal={0} />,
    );

    fireEvent.click(
      screen.getByLabelText("Capture body.$.accessToken into a variable"),
    );
    expect(active().testScript).toContain('pm.environment.set("accessToken"');
    expect(toastText()).toContain("credential");
  });

  it("keeps an existing script and appends under it", () => {
    activate({ testScript: 'pm.test("ok", function () {});' });
    render(<ResponsePane response={http({ body: '{"id":7}' })} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Capture body.$.id into a variable"));
    const script = active().testScript;
    expect(script.startsWith('pm.test("ok", function () {});')).toBe(true);
    expect(script).toContain('pm.environment.set("id", pm.response.json().id);');
  });

  it("does not capture the same value twice", () => {
    activate({ testScript: 'pm.environment.set("a", pm.response.json().a);' });
    render(<ResponsePane response={http()} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Capture body.$.a into a variable"));
    expect(active().testScript.match(/pm\.environment\.set/g)).toHaveLength(1);
    expect(toastText()).toContain("already captured");
  });

  it("gives a second capture of the same key a name of its own", () => {
    activate({ testScript: 'pm.environment.set("id", pm.response.json().data.id);' });
    render(<ResponsePane response={http({ body: '{"id":7}' })} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Capture body.$.id into a variable"));
    expect(active().testScript).toContain('pm.environment.set("id_2"');
  });

  it("asserts a plain value by equality, in the script the file stores", () => {
    activate();
    render(<ResponsePane response={http({ body: '{"id":7}' })} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Assert on body.$.id"));
    expect(active().testScript).toContain("pm.expect(pm.response.json().id).to.eql(7)");
  });

  it("asserts a credential exists rather than writing its value to the file", () => {
    activate();
    render(<ResponsePane response={http({ body: '{"token":"s3cret"}' })} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Assert on body.$.token"));
    const script = active().testScript;
    expect(script).toContain("to.not.be.undefined");
    expect(script).not.toContain("s3cret");
    expect(toastText()).toContain("credential");
  });

  it("does not add the same assertion twice", () => {
    activate();
    render(<ResponsePane response={http()} findSignal={0} />);

    fireEvent.click(screen.getByLabelText("Assert on body.$.a"));
    fireEvent.click(screen.getByLabelText("Assert on body.$.a"));
    expect(active().testScript.match(/pm\.test\(/g)).toHaveLength(1);
    expect(toastText()).toContain("already asserted on");
  });

  it("offers nothing on a raw body, where no line is a single JSON leaf", () => {
    activate();
    render(<ResponsePane response={http()} findSignal={0} />);
    fireEvent.click(screen.getByText("Raw"));

    expect(screen.queryByLabelText(/^Copy path/)).toBeNull();
  });
});
