import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useWorkspaces } from "../store/workspace";
import { CloneRepoDialog } from "./CloneRepoDialog";

const invoke = vi.hoisted(() => vi.fn());
const pickDirectory = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => pickDirectory(...args),
}));

const DEST = "/Users/me/Mandalo/apis";

describe("CloneRepoDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    pickDirectory.mockReset();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    useWorkspaces.setState({ items: [], activeId: null, error: null });
    useCollection.setState({
      workspace: null,
      switchWorkspace: vi.fn(async () => {}),
    } as never);
    invoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "default_workspace_dir":
          return Promise.resolve("/Users/me/Mandalo");
        case "github_status":
          return Promise.resolve({ connected: false, login: null });
        case "git_clone":
          return Promise.resolve(undefined);
        case "open_workspace":
          return Promise.resolve({
            workspace: { id: "ws1", path: DEST, name: "apis" },
            migrated: [],
          });
        case "set_active_workspace":
          return Promise.resolve({ id: "ws1", path: DEST, name: "apis" });
        case "list_workspaces":
          return Promise.resolve({
            items: [{ id: "ws1", path: DEST, name: "apis" }],
            active: "ws1",
          });
        default:
          return Promise.reject(new Error(cmd));
      }
    });
  });

  afterEach(() => {
    cleanup();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("clones into parent/repo and opens the workspace", async () => {
    const onClose = vi.fn();
    render(<CloneRepoDialog onClose={onClose} />);
    await waitFor(() =>
      expect(screen.getByDisplayValue("/Users/me/Mandalo")).toBeTruthy(),
    );

    fireEvent.change(
      screen.getByPlaceholderText("https://github.com/owner/name.git"),
      { target: { value: "https://github.com/acme/apis.git" } },
    );
    expect(screen.getByText(/Mandalo\/apis/)).toBeTruthy();

    fireEvent.click(screen.getByText("Clone and open"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_clone", {
        url: "https://github.com/acme/apis.git",
        dest: DEST,
        token: null,
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(useCollection.getState().switchWorkspace).toHaveBeenCalledWith(DEST);
  });
});
