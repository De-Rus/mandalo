import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  documentFromFile,
  documentFromText,
  documentFromUrl,
  importUrl,
} from "./importSource";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const SPEC = '{"openapi":"3.1.0","paths":{}}';

function asDesktop(): void {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

describe("importUrl", () => {
  it("refuses something that is not a URL", () => {
    expect(() => importUrl("petstore.json")).toThrow(/not a URL/);
  });

  it("refuses a scheme no import can be fetched over", () => {
    expect(() => importUrl("file:///etc/passwd")).toThrow(/http or https/);
  });

  it("accepts http and https", () => {
    expect(importUrl(" https://example.com/openapi.json ").href).toBe(
      "https://example.com/openapi.json",
    );
    expect(importUrl("http://localhost:8080/spec.yaml").protocol).toBe("http:");
  });
});

describe("documentFromText", () => {
  it("measures the document in bytes, not characters", () => {
    const doc = documentFromText("é");
    expect(doc.bytes).toBe(2);
    expect(doc.origin).toBe("text");
  });
});

describe("documentFromFile", () => {
  it("keeps the file name and reads the text", async () => {
    const file = new File([SPEC], "petstore.json", {
      type: "application/json",
    });
    const doc = await documentFromFile(file);
    expect(doc).toMatchObject({
      origin: "file",
      name: "petstore.json",
      text: SPEC,
    });
  });
});

describe("documentFromUrl on the desktop", () => {
  beforeEach(() => {
    invoke.mockReset();
    asDesktop();
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("fetches through the Rust side, never the webview", async () => {
    const spy = vi.spyOn(globalThis, "fetch");
    invoke.mockResolvedValue({
      url: "https://example.com/openapi.json",
      contentType: "application/json",
      bytes: SPEC.length,
      text: SPEC,
    });

    const doc = await documentFromUrl("https://example.com/openapi.json");

    expect(invoke).toHaveBeenCalledWith("fetch_text_for_import", {
      url: "https://example.com/openapi.json",
    });
    expect(spy).not.toHaveBeenCalled();
    expect(doc.text).toBe(SPEC);
    spy.mockRestore();
  });

  it("reports the rejection instead of falling back to a page fetch", async () => {
    invoke.mockRejectedValue("https://169.254.169.254 is a cloud metadata endpoint");
    await expect(
      documentFromUrl("https://169.254.169.254/latest/meta-data"),
    ).rejects.toBeTruthy();
  });
});

describe("documentFromUrl in the browser", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("fetches with the page and keeps the URL as the name", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(SPEC, { status: 200 }),
    );
    const doc = await documentFromUrl("https://example.com/openapi.json");
    expect(doc).toMatchObject({
      origin: "url",
      name: "https://example.com/openapi.json",
      text: SPEC,
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("explains CORS honestly when the browser drops the request", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("Failed to fetch"));
    await expect(
      documentFromUrl("https://example.com/openapi.json"),
    ).rejects.toThrow(/CORS headers.*drop it on this dialog/s);
  });

  it("reports a non-2xx answer as itself", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("nope", { status: 404, statusText: "Not Found" }),
    );
    await expect(
      documentFromUrl("https://example.com/openapi.json"),
    ).rejects.toThrow(/404/);
  });
});
