import { beforeEach, describe, expect, it, vi } from "vitest";
import { detectImportKind } from "./importKind";
import { importAs } from "./transfer";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("importAs", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({
      imported: 1,
      collections: 1,
      environments: 0,
      skipped: [],
      warnings: [],
      summary: "ok",
    });
  });

  it("routes a bundle to import_bundle", async () => {
    const json = '{"mandaloBundle":1,"requests":[]}';
    await importAs("/ws", detectImportKind(json).kind, json);
    expect(invoke).toHaveBeenCalledWith("import_bundle", {
      workspace: "/ws",
      json,
    });
  });

  it("routes an OpenAPI document to import_openapi as source text", async () => {
    const source = "openapi: 3.1.0\npaths: {}\n";
    await importAs("/ws", detectImportKind(source).kind, source);
    expect(invoke).toHaveBeenCalledWith("import_openapi", {
      workspace: "/ws",
      source,
    });
  });

  it("routes anything else to import_postman", async () => {
    const json = '{"info":{"name":"Coll"},"item":[]}';
    await importAs("/ws", detectImportKind(json).kind, json);
    expect(invoke).toHaveBeenCalledWith("import_postman", {
      workspace: "/ws",
      json,
    });
  });

  it("honours an override over what the document declares", async () => {
    const source = '{"openapi":"3.0.0"}';
    await importAs("/ws", "postman", source);
    expect(invoke).toHaveBeenCalledWith("import_postman", {
      workspace: "/ws",
      json: source,
    });
  });
});
