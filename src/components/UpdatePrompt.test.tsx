import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UpdatePrompt } from "./UpdatePrompt";

const check = vi.fn();
const downloadAndInstall = vi.fn();
const relaunch = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => check(...args),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...args: unknown[]) => relaunch(...args),
}));

vi.mock("../lib/host", () => ({
  currentHost: () => "desktop",
}));

afterEach(() => {
  cleanup();
  check.mockReset();
  downloadAndInstall.mockReset();
  relaunch.mockReset();
});

describe("UpdatePrompt", () => {
  beforeEach(() => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  it("stays quiet when no update is available", async () => {
    check.mockResolvedValueOnce(null);
    const { queryByRole } = render(<UpdatePrompt />);
    await waitFor(() => expect(check).toHaveBeenCalled());
    expect(queryByRole("heading", { name: "Update available" })).toBeNull();
  });

  it("prompts when an update is ready", async () => {
    check.mockResolvedValueOnce({
      version: "0.2.0",
      body: "Fixes",
      downloadAndInstall,
    });
    const { findByRole } = render(<UpdatePrompt />);
    expect(await findByRole("heading", { name: "Update available" })).toBeTruthy();
  });
});
