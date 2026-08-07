import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StorageState } from "./storage";

const state = vi.hoisted(() => ({
  value: {
    durability: "best-effort",
    usage: 1024,
    quota: 10240,
    ratio: 0.1,
    nearQuota: false,
    canPersist: true,
  } as StorageState,
  granted: false,
  folders: true,
  exported: null as number | null,
}));

vi.mock("./storage", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./storage")>()),
  storageState: () => Promise.resolve(state.value),
  requestPersistence: () => Promise.resolve(state.granted),
}));

vi.mock("./mounts", () => ({
  activeVfs: () => Promise.resolve({ kind: "browser", id: "browser" }),
  openFolder: () => Promise.resolve({ id: "folder:k", path: "p", name: "n" }),
  supportsFolders: () => state.folders,
}));

vi.mock("./export", () => ({
  exportWorkspace: () => Promise.resolve(3),
  lastExportAt: () => Promise.resolve(state.exported),
}));

const { COPY, NEAR_QUOTA, NEVER_EXPORTED, NO_FOLDERS, StoragePanel } = await import(
  "./StoragePanel"
);

afterEach(cleanup);

beforeEach(() => {
  state.value = {
    durability: "best-effort",
    usage: 1024,
    quota: 10240,
    ratio: 0.1,
    nearQuota: false,
    canPersist: true,
  };
  state.granted = false;
  state.folders = true;
  state.exported = null;
});

async function open(): Promise<void> {
  render(<StoragePanel />);
  await userEvent.click(await screen.findByRole("button"));
}

describe("telling the user where their work lives", () => {
  it("names browser storage as evictable when persistence was never granted", async () => {
    await open();

    expect(screen.getByRole("dialog").textContent).toContain(
      COPY["best-effort"].body,
    );
  });

  it("says plainly that the browser refused, and points at a folder", async () => {
    await open();
    await userEvent.click(
      screen.getByRole("button", { name: "Ask the browser to keep it" }),
    );

    expect(screen.getByRole("dialog").textContent).toContain(COPY.denied.body);
    expect(screen.getByRole("dialog").textContent).toContain(
      "Opening a folder is the durable option",
    );
  });

  it("reports the granted case once the browser agrees", async () => {
    state.value = { ...state.value, durability: "persisted" };

    await open();

    expect(screen.getByRole("dialog").textContent).toContain(COPY.persisted.body);
  });

  it("calls a folder workspace what it is — files the user owns", async () => {
    state.value = { ...state.value, durability: "folder" };

    await open();

    expect(screen.getByRole("dialog").textContent).toContain(COPY.folder.body);
  });

  it("admits when the browser will not say", async () => {
    state.value = { ...state.value, durability: "unavailable", canPersist: false };

    await open();

    expect(screen.getByRole("dialog").textContent).toContain(COPY.unavailable.body);
  });

  it("warns before the quota runs out rather than failing a save", async () => {
    state.value = { ...state.value, nearQuota: true, ratio: 0.92 };

    await open();

    expect(screen.getByRole("dialog").textContent).toContain(NEAR_QUOTA);
  });

  it("shows what is used out of what is available", async () => {
    await open();

    expect(screen.getByRole("dialog").textContent).toContain("Using 1.0 KB of 10 KB");
  });

  it("nudges a workspace that has never been exported", async () => {
    await open();

    expect(screen.getByRole("dialog").textContent).toContain(NEVER_EXPORTED);
  });

  it("stops nudging once a copy has been taken", async () => {
    state.exported = Date.now();

    await open();

    expect(screen.getByRole("dialog").textContent).not.toContain(NEVER_EXPORTED);
  });

  it("is honest in Firefox and Safari instead of showing a dead button", async () => {
    state.folders = false;

    await open();

    expect(screen.queryByRole("button", { name: "Open folder…" })).toBeNull();
    expect(screen.getByRole("dialog").textContent).toContain(NO_FOLDERS);
  });

  it("offers the folder as the recommended path where it works", async () => {
    await open();

    expect(screen.getByRole("button", { name: "Open folder…" })).toBeTruthy();
  });

  it("lets the user take a copy out", async () => {
    await open();
    await userEvent.click(screen.getByRole("button", { name: "Download a copy" }));

    expect(screen.getByRole("dialog").textContent).toContain("Downloaded 3 files");
  });
});
