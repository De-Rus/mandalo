import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SyncStatus } from "../lib/api";
import { useCollection } from "../store/collection";
import { useGit } from "../store/git";
import { GitBar } from "./GitBar";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const CLEAN: SyncStatus = {
  isRepo: true,
  branch: "main",
  detached: false,
  remoteUrl: "https://github.com/acme/apis.git",
  staged: 0,
  unstaged: 0,
  untracked: 0,
  ahead: 0,
  behind: 0,
  conflicted: [],
  dirtyFiles: [],
  dirtyTotal: 0,
  identity: { name: "Ada", email: "ada@ex.com", isFallback: false },
};

function mockGit(status: SyncStatus, github?: { connected: boolean; login: string | null }) {
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "git_status") return Promise.resolve(status);
    if (cmd === "github_status")
      return Promise.resolve(
        github ?? { connected: false, login: null },
      );
    if (cmd === "github_list_repos") return Promise.resolve([]);
    return Promise.resolve(undefined);
  });
}

describe("GitBar", () => {
  beforeEach(() => {
    invoke.mockReset();
    useCollection.setState({ workspace: "/ws" });
    useGit.setState({ status: null, error: null, busy: false });
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    cleanup();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("shows branch and clean state", async () => {
    mockGit(CLEAN);
    render(<GitBar />);
    await waitFor(() => expect(screen.getByText("main")).toBeTruthy());
    expect(screen.getByText("clean")).toBeTruthy();
  });

  it("shows dirty count in amber terms", async () => {
    mockGit({
      ...CLEAN,
      dirtyTotal: 3,
      dirtyFiles: ["a.http", "b.toml", "c.http"],
      ahead: 2,
      behind: 1,
    });
    render(<GitBar />);
    await waitFor(() => expect(screen.getByText("3 changed")).toBeTruthy());
    expect(screen.getByText("↑2 ↓1")).toBeTruthy();
  });

  it("offers Sync when the tree is dirty", async () => {
    mockGit({
      ...CLEAN,
      dirtyTotal: 2,
      dirtyFiles: ["a.http", "b.toml"],
    });
    render(<GitBar />);
    await waitFor(() => expect(screen.getByText("2 changed")).toBeTruthy());
    expect(screen.getByRole("button", { name: "Sync" })).toBeTruthy();
  });

  it("opens Resolve from the bar when status reports config conflicts", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "git_status")
        return Promise.resolve({
          ...CLEAN,
          behind: 1,
          dirtyTotal: 1,
          dirtyFiles: ["environments/local.toml"],
          conflicted: ["environments/local.toml"],
        });
      if (cmd === "github_status")
        return Promise.resolve({ connected: false, login: null });
      if (cmd === "conflict_previews")
        return Promise.resolve([
          {
            path: "environments/local.toml",
            ours: {
              exists: true,
              kind: "environment",
              name: "local",
              text: 'value = "bob"\n',
            },
            theirs: {
              exists: true,
              kind: "environment",
              name: "local",
              text: 'value = "alice"\n',
            },
          },
        ]);
      return Promise.resolve(undefined);
    });
    render(<GitBar />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /1 conflict/ })).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: "Resolve" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sync" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Resolve" }));
    await waitFor(() => expect(screen.getByText(/value = "bob"/)).toBeTruthy());
    expect(screen.getByText(/value = "alice"/)).toBeTruthy();
  });

  it("offers Initialize when the folder is not a git repo", async () => {
    mockGit({ ...CLEAN, isRepo: false, remoteUrl: null });
    render(<GitBar />);
    await waitFor(() => expect(screen.getByText("not a git repository")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Initialize" }));
    expect(screen.getByRole("heading", { name: "Initialize git" })).toBeTruthy();
  });

  it("offers Connect remote when the repo has no origin", async () => {
    mockGit({ ...CLEAN, remoteUrl: null });
    render(<GitBar />);
    await waitFor(() => expect(screen.getByText("local only")).toBeTruthy());
    expect(screen.queryByRole("button", { name: "Sync" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Connect remote…" }));
    expect(screen.getByRole("heading", { name: "Connect remote" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "GitHub" })).toBeTruthy();
  });

  it("connects a remote through a pasted URL", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "git_status")
        return Promise.resolve({ ...CLEAN, remoteUrl: null });
      if (cmd === "github_status")
        return Promise.resolve({ connected: false, login: null });
      if (cmd === "git_init") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    render(<GitBar />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Connect remote…" })).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Connect remote…" }));
    fireEvent.click(screen.getByRole("tab", { name: "URL" }));
    fireEvent.change(screen.getByPlaceholderText("https://github.com/org/repo.git"), {
      target: { value: "https://github.com/acme/apis.git" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_init", {
        workspace: "/ws",
        remoteUrl: "https://github.com/acme/apis.git",
      }),
    );
  });

  it("shows a GitHub sign-in entry on the bar", async () => {
    mockGit({ ...CLEAN, remoteUrl: null });
    render(<GitBar />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Sign in to GitHub" })).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Sign in to GitHub" }));
    expect(screen.getByRole("heading", { name: "Connect remote" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "GitHub" })).toBeTruthy();
  });

  it("connects by creating a GitHub repository", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "git_status")
        return Promise.resolve({ ...CLEAN, remoteUrl: null });
      if (cmd === "github_status")
        return Promise.resolve({ connected: true, login: "ada" });
      if (cmd === "github_list_repos") return Promise.resolve([]);
      if (cmd === "github_create_repo")
        return Promise.resolve({
          name: "apis",
          fullName: "ada/apis",
          private: true,
          cloneUrl: "https://github.com/ada/apis.git",
          htmlUrl: "https://github.com/ada/apis",
          defaultBranch: "main",
        });
      if (cmd === "git_init") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    render(<GitBar />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "@ada" })).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Connect remote…" }));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Create repository" })).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("tab", { name: "Create repository" }));
    fireEvent.change(screen.getByPlaceholderText("my-api-collection"), {
      target: { value: "apis" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create and connect" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("github_create_repo", {
        name: "apis",
        private: true,
      }),
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_init", {
        workspace: "/ws",
        remoteUrl: "https://github.com/ada/apis.git",
      }),
    );
  });
});
