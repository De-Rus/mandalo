import { afterEach, describe, expect, it } from "vitest";
import { currentHost, hasNativeGrpc, isLoopback } from "./host";

const global = window as unknown as Record<string, unknown>;

afterEach(() => {
  delete global.__TAURI_INTERNALS__;
  delete global.acquireVsCodeApi;
});

describe("host detection", () => {
  it("is the browser when neither Tauri nor VS Code is around", () => {
    expect(currentHost()).toBe("browser");
    expect(hasNativeGrpc()).toBe(false);
  });

  it("is the desktop app when Tauri injected its bridge", () => {
    global.__TAURI_INTERNALS__ = {};
    expect(currentHost()).toBe("desktop");
    expect(hasNativeGrpc()).toBe(true);
  });

  it("is the editor when the VS Code webview API is around", () => {
    global.acquireVsCodeApi = () => ({});
    expect(currentHost()).toBe("editor");
    expect(hasNativeGrpc()).toBe(true);
  });
});

describe("loopback targets", () => {
  it("recognises the addresses a local mock listens on", () => {
    expect(isLoopback("http://localhost:50051")).toBe(true);
    expect(isLoopback("http://127.0.0.1:50051")).toBe(true);
    expect(isLoopback("http://[::1]:50051")).toBe(true);
    expect(isLoopback("localhost:50051")).toBe(true);
  });

  it("leaves real hosts and unresolved templates alone", () => {
    expect(isLoopback("https://api.mandalo.dev")).toBe(false);
    expect(isLoopback("{{grpcUrl}}")).toBe(false);
    expect(isLoopback("")).toBe(false);
  });
});
