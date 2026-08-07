import { afterEach, describe, expect, it, vi } from "vitest";
import { formatBytes, persisted, requestPersistence, storageState } from "./storage";

function withStorage(value: unknown): void {
  Object.defineProperty(navigator, "storage", { value, configurable: true });
}

afterEach(() => withStorage(undefined));

const QUOTA = { usage: 2 * 1024 * 1024, quota: 100 * 1024 * 1024 };

describe("what the user is told about durability", () => {
  it("reports persistent storage when the browser granted it", async () => {
    withStorage({
      persisted: () => Promise.resolve(true),
      persist: () => Promise.resolve(true),
      estimate: () => Promise.resolve(QUOTA),
    });

    const state = await storageState("browser");

    expect(state.durability).toBe("persisted");
    expect(state.nearQuota).toBe(false);
  });

  it("reports best-effort when persistence was never granted", async () => {
    withStorage({
      persisted: () => Promise.resolve(false),
      persist: () => Promise.resolve(false),
      estimate: () => Promise.resolve(QUOTA),
    });

    expect((await storageState("browser")).durability).toBe("best-effort");
  });

  it("says persistence was refused rather than pretending it worked", async () => {
    withStorage({
      persisted: () => Promise.resolve(false),
      persist: () => Promise.resolve(false),
      estimate: () => Promise.resolve(QUOTA),
    });

    expect(await requestPersistence()).toBe(false);
  });

  it("reports durability as unknown where the API is missing", async () => {
    withStorage(undefined);

    const state = await storageState("browser");

    expect(state.durability).toBe("unavailable");
    expect(state.canPersist).toBe(false);
    expect(await persisted()).toBe(false);
  });

  it("treats a folder workspace as the durable mode regardless of the browser", async () => {
    withStorage(undefined);

    expect((await storageState("folder")).durability).toBe("folder");
  });

  it("warns before the quota runs out rather than failing a save", async () => {
    withStorage({
      persisted: () => Promise.resolve(true),
      persist: () => Promise.resolve(true),
      estimate: () => Promise.resolve({ usage: 90, quota: 100 }),
    });

    const state = await storageState("browser");

    expect(state.nearQuota).toBe(true);
    expect(state.ratio).toBeCloseTo(0.9);
  });

  it("survives a browser that throws from estimate()", async () => {
    withStorage({
      persisted: () => Promise.resolve(true),
      persist: () => Promise.resolve(true),
      estimate: () => Promise.reject(new Error("nope")),
    });

    const state = await storageState("browser");

    expect(state.usage).toBeNull();
    expect(state.nearQuota).toBe(false);
  });

  it("formats sizes the way a person reads them", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });
});

describe("asking for persistence", () => {
  it("does not ask on first paint — only when work is saved", async () => {
    const persist = vi.fn(() => Promise.resolve(true));
    withStorage({
      persist,
      persisted: () => Promise.resolve(false),
      estimate: () => Promise.resolve(QUOTA),
    });

    await storageState("browser");

    expect(persist).not.toHaveBeenCalled();
  });
});
