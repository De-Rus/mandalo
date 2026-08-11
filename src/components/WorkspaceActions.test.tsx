import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { useTransfer } from "../store/transfer";
import { WorkspaceActions } from "./WorkspaceActions";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { open, save } = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open, save }));

const PLAN = {
  included: {
    collections: [
      {
        slug: "acme",
        name: "Acme API",
        requests: [{ path: "a.toml", name: "a" }],
      },
    ],
    environments: ["staging"],
    requestCount: 3,
  },
  excluded: {
    secretValues: 2,
    localValues: 0,
    withheldNames: ["token"],
    collections: [],
    requests: 0,
    environments: [],
  },
  findings: [],
  warnings: [],
  bytes: 1024,
  blocked: false,
  token: "plan-1",
  format: "native",
};

describe("WorkspaceActions", () => {
  beforeEach(() => {
    invoke.mockReset();
    open.mockReset();
    save.mockReset();
    localStorage.clear();
    useCollection.setState({
      workspace: "/ws",
      activeId: null,
      tree: {
        collections: [
          {
            id: "1",
            slug: "acme",
            name: "Acme API",
            folders: [],
            requests: [
              { id: "a", path: "a.toml", name: "a", method: "GET", kind: "http" },
            ],
          },
        ],
        skipped: [],
      },
    });
    useEnv.setState({
      envs: [{ name: "staging", vars: {} }],
      selected: "staging",
      error: null,
    } as never);
    useTransfer.setState({ importOpen: false, dropped: null });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share") return Promise.resolve(null);
      if (cmd === "plan_export") return Promise.resolve(PLAN);
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      return Promise.resolve(undefined);
    });
  });

  afterEach(cleanup);

  it("opens the import dialog rather than a bare file picker", () => {
    render(<WorkspaceActions />);

    fireEvent.click(screen.getByLabelText("Workspace actions"));
    fireEvent.click(screen.getByText("Import…"));

    expect(useTransfer.getState().importOpen).toBe(true);
    expect(open).not.toHaveBeenCalled();
  });

  it("shows the export plan before anything is written", async () => {
    render(<WorkspaceActions />);

    fireEvent.click(screen.getByLabelText("Workspace actions"));
    fireEvent.click(screen.getByText("Export…"));

    await waitFor(() =>
      expect(
        screen.getByText("2 secret value(s) stay on this machine"),
      ).toBeTruthy(),
    );
    expect(invoke).toHaveBeenCalledWith("plan_export", {
      workspace: "/ws",
      selection: null,
      format: "native",
    });
    expect(screen.getByText(/Acme API/)).toBeTruthy();
    expect(save).not.toHaveBeenCalled();
  });
});

const sources = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

describe("filesystem access", () => {
  it("never imports the scoped fs plugin", () => {
    const banned = ["@tauri-apps", "plugin-fs"].join("/");
    const offenders = Object.entries(sources)
      .filter(([, text]) => text.includes(banned))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });
});
