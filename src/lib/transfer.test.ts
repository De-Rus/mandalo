import { beforeEach, describe, expect, it, vi } from "vitest";
import { importFromText } from "./transfer";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("importFromText", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({
      imported: 1,
      environments: 0,
      skipped: [],
      warnings: [],
      summary: "ok",
    });
  });

  it("routes a mandalo bundle to import_bundle", async () => {
    const json = '{"mandaloBundle":1,"requests":[]}';
    await importFromText("/ws", json);
    expect(invoke).toHaveBeenCalledWith("import_bundle", {
      workspace: "/ws",
      json,
    });
  });

  it("routes anything else to import_postman", async () => {
    const json = '{"info":{"name":"Coll"},"item":[]}';
    await importFromText("/ws", json);
    expect(invoke).toHaveBeenCalledWith("import_postman", {
      workspace: "/ws",
      json,
    });
  });

  it("fails loud on invalid JSON", () => {
    expect(() => importFromText("/ws", "not json")).toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });
});
