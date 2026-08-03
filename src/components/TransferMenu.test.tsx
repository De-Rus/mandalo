import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { useToasts } from "../store/toast";
import { TransferMenu } from "./TransferMenu";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { open, save } = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open, save }));

const BUNDLE = '{"mandaloBundle":1,"requests":[]}';

const REPORT = {
  imported: 1,
  environments: 0,
  skipped: [],
  warnings: [],
  summary: "Imported 1 request",
};

describe("TransferMenu", () => {
  beforeEach(() => {
    invoke.mockReset();
    open.mockReset();
    save.mockReset();
    localStorage.clear();
    useCollection.setState({ workspace: "/ws", activeId: null });
    useEnv.setState({ workspace: "/ws", envs: [], selected: null, error: null });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "read_text_file_for_import") return Promise.resolve(BUNDLE);
      if (cmd === "import_bundle") return Promise.resolve(REPORT);
      if (cmd === "export_bundle")
        return Promise.resolve({ json: BUNDLE, findings: [] });
      if (cmd === "list_tree")
        return Promise.resolve({ collections: [], skipped: [] });
      if (cmd === "list_environments")
        return Promise.resolve({ items: [], skipped: [] });
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      return Promise.resolve(undefined);
    });
  });

  afterEach(cleanup);

  it("imports through read_text_file_for_import with the dialog path", async () => {
    open.mockResolvedValue("/outside/workspace/bundle.json");
    render(<TransferMenu />);

    fireEvent.click(screen.getByLabelText("Import / Export"));
    fireEvent.click(screen.getByText("Import…"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("read_text_file_for_import", {
        path: "/outside/workspace/bundle.json",
      }),
    );
    expect(invoke).toHaveBeenCalledWith("import_bundle", {
      workspace: "/ws",
      json: BUNDLE,
    });
    await waitFor(() => expect(screen.getByText("Import complete")).toBeTruthy());
  });

  it("does nothing when the import dialog is cancelled", async () => {
    open.mockResolvedValue(null);
    render(<TransferMenu />);

    fireEvent.click(screen.getByLabelText("Import / Export"));
    fireEvent.click(screen.getByText("Import…"));

    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain(
      "read_text_file_for_import",
    );
  });

  it("exports through write_text_file_for_export with the dialog path", async () => {
    save.mockResolvedValue("/outside/workspace/out.json");
    render(<TransferMenu />);

    fireEvent.click(screen.getByLabelText("Import / Export"));
    fireEvent.click(screen.getByText("Export bundle…"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("write_text_file_for_export", {
        path: "/outside/workspace/out.json",
        contents: BUNDLE,
      }),
    );
    expect(useToasts.getState().items.map((t) => t.text)).toContain(
      "Bundle saved",
    );
  });

  it("makes the user confirm an export the scanner flagged", async () => {
    save.mockResolvedValue("/out.json");
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "export_bundle")
        return Promise.resolve({
          json: BUNDLE,
          findings: [
            {
              path: "environments/prod.toml",
              line: 4,
              rule: "aws-access-key-id",
              excerpt: "key = AKIA…",
            },
          ],
        });
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      return Promise.resolve(undefined);
    });
    render(<TransferMenu />);

    fireEvent.click(screen.getByLabelText("Import / Export"));
    fireEvent.click(screen.getByText("Export bundle…"));

    await waitFor(() =>
      expect(screen.getByText("Review before exporting")).toBeTruthy(),
    );
    expect(screen.getByText("aws-access-key-id")).toBeTruthy();
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain(
      "write_text_file_for_export",
    );

    fireEvent.click(screen.getByText("Export anyway"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("write_text_file_for_export", {
        path: "/out.json",
        contents: BUNDLE,
      }),
    );
  });

  it("does not write when the export dialog is cancelled", async () => {
    save.mockResolvedValue(null);
    render(<TransferMenu />);

    fireEvent.click(screen.getByLabelText("Import / Export"));
    fireEvent.click(screen.getByText("Export bundle…"));

    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain(
      "write_text_file_for_export",
    );
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
