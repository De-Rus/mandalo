import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { ImportDialog } from "./ImportDialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { open, save } = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open, save }));

const SPEC = '{"openapi":"3.1.0","paths":{}}';
const COLLECTION = '{"info":{"name":"Acme"},"item":[]}';

const REPORT = {
  imported: 4,
  collections: 1,
  environments: 0,
  skipped: [],
  warnings: [],
  summary: "Imported 4 requests",
};

function base(cmd: string): Promise<unknown> | undefined {
  if (cmd === "import_openapi" || cmd === "import_postman" || cmd === "import_bundle")
    return Promise.resolve(REPORT);
  if (cmd === "list_tree") return Promise.resolve({ collections: [], skipped: [] });
  if (cmd === "list_environments") return Promise.resolve({ items: [], skipped: [] });
  if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
  return undefined;
}

describe("ImportDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    open.mockReset();
    useCollection.setState({ workspace: "/ws", activeId: null });
    useEnv.setState({ workspace: "/ws", envs: [], selected: null, error: null });
    invoke.mockImplementation((cmd: string) => base(cmd) ?? Promise.resolve(undefined));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    cleanup();
  });

  it("fetches a URL through the Rust side and says what it received", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "fetch_text_for_import")
        return Promise.resolve({
          url: "https://example.com/openapi.json",
          contentType: "application/json",
          bytes: SPEC.length,
          text: SPEC,
        });
      return base(cmd) ?? Promise.resolve(undefined);
    });
    render(<ImportDialog dropped={null} onClose={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText("https://example.com/openapi.json"), {
      target: { value: "https://example.com/openapi.json" },
    });
    fireEvent.click(screen.getByText("Fetch"));

    await waitFor(() =>
      expect(screen.getByText("https://example.com/openapi.json")).toBeTruthy(),
    );
    expect(screen.getByText("30 B")).toBeTruthy();
    expect(screen.getByText(/declares an openapi or swagger version/)).toBeTruthy();
  });

  it("reports a fetch that failed and imports nothing", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "fetch_text_for_import")
        return Promise.reject("example.com resolves to 169.254.169.254, a metadata endpoint");
      return base(cmd) ?? Promise.resolve(undefined);
    });
    render(<ImportDialog dropped={null} onClose={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText("https://example.com/openapi.json"), {
      target: { value: "https://example.com/openapi.json" },
    });
    fireEvent.click(screen.getByText("Fetch"));

    await waitFor(() => expect(screen.getByText(/metadata endpoint/)).toBeTruthy());
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("import_openapi");
  });

  it("imports a dropped document with the importer its content names", async () => {
    render(
      <ImportDialog
        dropped={{ origin: "file", name: "petstore.json", text: SPEC, bytes: 30 }}
        onClose={() => {}}
      />,
    );

    expect(screen.getByText("petstore.json")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Import" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("import_openapi", {
        workspace: "/ws",
        source: SPEC,
      }),
    );
    await waitFor(() => expect(screen.getByText("Import complete")).toBeTruthy());
    expect(screen.getByText(/read as OpenAPI \/ Swagger/)).toBeTruthy();
  });

  it("lets the reader override an importer it guessed", async () => {
    render(
      <ImportDialog
        dropped={{ origin: "text", name: "Pasted document", text: SPEC, bytes: 30 }}
        onClose={() => {}}
      />,
    );

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "postman" } });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("import_postman", {
        workspace: "/ws",
        json: SPEC,
      }),
    );
  });

  it("says a document whose format is unnamed is a guess", () => {
    render(
      <ImportDialog
        dropped={{ origin: "text", name: "Pasted document", text: "{}", bytes: 2 }}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/Nothing in this document names its format/)).toBeTruthy();
  });

  it("reads a pasted document", async () => {
    render(<ImportDialog dropped={null} onClose={() => {}} />);

    fireEvent.click(screen.getByText("Paste"));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: COLLECTION } });
    fireEvent.click(screen.getByText("Read document"));

    await waitFor(() => expect(screen.getByText("Pasted document")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("import_postman", {
        workspace: "/ws",
        json: COLLECTION,
      }),
    );
  });

  it("keeps the file picker as one of the ways in", async () => {
    open.mockResolvedValue("/home/dani/petstore.json");
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "read_text_file_for_import") return Promise.resolve(SPEC);
      return base(cmd) ?? Promise.resolve(undefined);
    });
    render(<ImportDialog dropped={null} onClose={() => {}} />);

    fireEvent.click(screen.getByText("From file"));
    fireEvent.click(screen.getByText("Choose file…"));

    await waitFor(() => expect(screen.getByText("petstore.json")).toBeTruthy());
  });
});
