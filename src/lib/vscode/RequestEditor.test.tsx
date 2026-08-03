import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SavedRequest } from "../api";
import { PROTOCOL_VERSION, type CallMessage } from "./protocol";
import { RequestEditor } from "./RequestEditor";

const PING: SavedRequest = {
  id: "abc",
  name: "Ping",
  kind: "http",
  method: "GET",
  url: "{{base}}/ping",
  description: null,
  body: null,
  headers: [["Accept", "application/json"]],
  auth: { type: "none" },
  graphql: null,
  grpc: null,
  scripts: { pre: null, post: null },
  tests: [],
  captures: [],
};

const CONTEXT = {
  workspace: "/w",
  collection: "Acme API",
  requestPath: "ping.toml",
  fileName: "ping.toml",
};

const ENVIRONMENT = {
  items: [{ name: "local", vars: { base: "https://api.test" } }],
  selected: "local",
};

/** The extension host, reduced to the half of it the WebView can observe. */
class FakeHost {
  readonly calls: CallMessage[] = [];
  request: SavedRequest = structuredClone(PING);

  postMessage = (message: unknown): void => {
    const call = message as CallMessage;
    this.calls.push(call);
    if (call.command === "load_document") {
      this.reply(call.id, { request: this.request, error: null, context: CONTEXT });
      return;
    }
    if (call.command === "list_environments") {
      this.reply(call.id, ENVIRONMENT);
      return;
    }
    if (call.command === "patch_document" || call.command === "save_document") {
      this.request = call.args["request"] as SavedRequest;
      this.reply(call.id, null);
      return;
    }
    if (call.command === "run_request_full") {
      this.reply(call.id, {
        path: "ping.toml",
        name: "Ping",
        method: "GET",
        url: "https://api.test/ping",
        response: {
          status: 201,
          statusText: "Created",
          headers: [["x-trace", "9"]],
          body: '{"ok":true}',
          binary: false,
          durationMs: 7,
          sizeBytes: 11,
        },
        grpc: null,
        tests: [],
        captures: [],
        logs: [],
        passed: true,
        durationMs: 7,
        error: null,
        errorCode: null,
      });
      return;
    }
    this.reply(call.id, null);
  };

  of(command: string): CallMessage[] {
    return this.calls.filter((call) => call.command === command);
  }

  patched(): SavedRequest | undefined {
    const calls = this.of("patch_document");
    return calls[calls.length - 1]?.args["request"] as SavedRequest | undefined;
  }

  pushDocument(request: SavedRequest | null, error: string | null = null): void {
    this.deliver({
      v: PROTOCOL_VERSION,
      type: "event",
      event: "document",
      payload: { request, error, context: CONTEXT },
    });
  }

  private reply(id: string, value: unknown): void {
    queueMicrotask(() => this.deliver({ v: PROTOCOL_VERSION, type: "reply", id, ok: true, value }));
  }

  private deliver(data: unknown): void {
    window.dispatchEvent(new MessageEvent("message", { data }));
  }
}

let host: FakeHost;

// The WebView acquires its channel once per page, so the tests share one seam and
// swap the host behind it rather than trying to re-acquire it per test.
vi.stubGlobal("acquireVsCodeApi", () => ({ postMessage: (m: unknown) => host.postMessage(m) }));

beforeEach(() => {
  host = new FakeHost();
});

afterEach(cleanup);

async function mount(): Promise<void> {
  render(<RequestEditor />);
  await screen.findByLabelText("URL");
}

async function settle(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
}

describe("RequestEditor", () => {
  it("asks the host for the document and renders it as the workbench", async () => {
    await mount();
    expect(host.of("load_document")).toHaveLength(1);
    expect((screen.getByLabelText("URL") as HTMLInputElement).value).toBe("{{base}}/ping");
    expect((screen.getByLabelText("Method") as HTMLSelectElement).value).toBe("GET");
    expect(screen.getByText("Acme API · ping.toml")).toBeTruthy();
  });

  it("shows the environment the extension selected", async () => {
    await mount();
    await waitFor(() => expect(screen.getByText("local")).toBeTruthy());
  });

  it("sends an edit to the host as a patch, debounced into one call", async () => {
    await mount();
    const url = screen.getByLabelText("URL");
    fireEvent.change(url, { target: { value: "{{base}}/pin" } });
    fireEvent.change(url, { target: { value: "{{base}}/pingu" } });
    await settle();
    expect(host.of("patch_document")).toHaveLength(1);
    expect(host.patched()?.url).toBe("{{base}}/pingu");
  });

  it("leaves every field the user did not touch exactly as it was", async () => {
    await mount();
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "{{base}}/health" } });
    await settle();
    const patched = host.patched() as SavedRequest;
    expect(patched).toEqual({ ...PING, url: "{{base}}/health" });
  });

  it("adopts an edit that arrived from the text editor", async () => {
    await mount();
    act(() => host.pushDocument({ ...PING, url: "{{base}}/from-disk", method: "POST" }));
    await waitFor(() =>
      expect((screen.getByLabelText("URL") as HTMLInputElement).value).toBe("{{base}}/from-disk"),
    );
    expect((screen.getByLabelText("Method") as HTMLSelectElement).value).toBe("POST");
  });

  it("does not answer a document push that only echoes what it already has", async () => {
    await mount();
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "{{base}}/echo" } });
    await settle();
    const before = host.calls.length;
    act(() => host.pushDocument(host.patched() as SavedRequest));
    await settle();
    expect(host.calls.length).toBe(before);
    expect((screen.getByLabelText("URL") as HTMLInputElement).value).toBe("{{base}}/echo");
  });

  it("never loops: a hundred pushes of its own state produce no traffic", async () => {
    await mount();
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "{{base}}/loop" } });
    await settle();
    const before = host.calls.length;
    for (let i = 0; i < 100; i += 1) act(() => host.pushDocument(host.patched() as SavedRequest));
    await settle();
    expect(host.calls.length).toBe(before);
  });

  it("saves through the host, flushing the pending edit first", async () => {
    await mount();
    fireEvent.change(screen.getByLabelText("URL"), { target: { value: "{{base}}/saved" } });
    fireEvent.click(screen.getByTitle("Save (⌘S)"));
    await settle();
    expect(host.of("save_document")).toHaveLength(1);
    expect((host.of("save_document")[0]?.args["request"] as SavedRequest).url).toBe("{{base}}/saved");
    expect(host.of("patch_document")).toHaveLength(0);
  });

  it("runs the request through the host and renders the response", async () => {
    await mount();
    fireEvent.click(screen.getByTitle("Send (⌘⏎)"));
    await waitFor(() => expect(screen.getByText(/201\s+Created/)).toBeTruthy());
    expect(host.of("run_request_full")).toHaveLength(1);
    expect(host.of("run_request_full")[0]?.args["env"]).toBe("local");
  });

  it("explains a request it cannot read instead of rendering an empty form", async () => {
    host.request = { ...PING, kind: "telepathy" as SavedRequest["kind"] };
    render(<RequestEditor />);
    await waitFor(() => expect(screen.getByText("This request could not be read")).toBeTruthy());
    expect(screen.getByText(/unknown kind "telepathy"/)).toBeTruthy();
  });

  it("asks VS Code to pick the environment rather than picking one itself", async () => {
    await mount();
    await waitFor(() => expect(screen.getByText("local")).toBeTruthy());
    fireEvent.click(screen.getByText("local"));
    await settle();
    expect(host.of("select_environment")).toHaveLength(1);
  });
});
