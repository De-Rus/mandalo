import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useWorkspaces } from "../store/workspace";
import { GithubReposDialog } from "./GithubReposDialog";

const invoke = vi.hoisted(() => vi.fn());
const pickDirectory = vi.hoisted(() => vi.fn());
const openUrl = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => pickDirectory(...args),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrl(...args),
}));

const REPO = {
  name: "apis",
  fullName: "acme/apis",
  private: true,
  cloneUrl: "https://github.com/acme/apis.git",
  htmlUrl: "https://github.com/acme/apis",
  defaultBranch: "main",
};

describe("GithubReposDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    openUrl.mockReset();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    useWorkspaces.setState({ items: [], activeId: null, error: null });
    useCollection.setState({
      workspace: null,
      switchWorkspace: vi.fn(async () => {}),
    } as never);
    invoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "default_workspace_dir":
          return Promise.resolve("/Users/me/Mandalo");
        case "github_status":
          return Promise.resolve({ connected: true, login: "ada" });
        case "github_list_repos":
          return Promise.resolve([REPO]);
        case "github_create_repo":
          return Promise.resolve({
            ...REPO,
            name: args?.name,
            fullName: `ada/${args?.name}`,
            cloneUrl: `https://github.com/ada/${args?.name}.git`,
          });
        case "git_clone":
          return Promise.resolve(undefined);
        case "open_workspace":
          return Promise.resolve({
            workspace: {
              id: "ws1",
              path: `/Users/me/Mandalo/${REPO.name}`,
              name: REPO.name,
            },
            migrated: [],
          });
        case "set_active_workspace":
          return Promise.resolve({
            id: "ws1",
            path: `/Users/me/Mandalo/${REPO.name}`,
            name: REPO.name,
          });
        case "list_workspaces":
          return Promise.resolve({
            items: [
              {
                id: "ws1",
                path: `/Users/me/Mandalo/${REPO.name}`,
                name: REPO.name,
              },
            ],
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

  it("lists repos and clones the selected one", async () => {
    const onClose = vi.fn();
    render(<GithubReposDialog onClose={onClose} />);
    await waitFor(() => expect(screen.getByText("Signed in as")).toBeTruthy());
    await waitFor(() => expect(screen.getByText("acme/apis")).toBeTruthy());

    fireEvent.click(screen.getByText("acme/apis"));
    fireEvent.click(screen.getByText("Clone and open"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_clone", {
        url: REPO.cloneUrl,
        dest: "/Users/me/Mandalo/apis",
        token: null,
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("creates a private repo then clones it", async () => {
    const onClose = vi.fn();
    render(<GithubReposDialog onClose={onClose} />);
    await waitFor(() => expect(screen.getByText("Create repository")).toBeTruthy());
    fireEvent.click(screen.getByText("Create repository"));
    fireEvent.change(screen.getByPlaceholderText("my-api-collection"), {
      target: { value: "billing" },
    });
    fireEvent.click(screen.getByText("Create and open"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("github_create_repo", {
        name: "billing",
        private: true,
      }),
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "git_clone",
        expect.objectContaining({
          dest: "/Users/me/Mandalo/billing",
        }),
      ),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
