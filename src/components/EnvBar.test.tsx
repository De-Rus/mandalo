import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "../store/env";
import { EnvBar } from "./EnvBar";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("EnvBar", () => {
  beforeEach(() => {
    invoke.mockReset();
    localStorage.clear();
    useEnv.setState({
      workspace: null,
      envs: [],
      selected: null,
      error: null,
    });
  });

  afterEach(cleanup);

  it("shows a warning line for environment files the backend could not parse", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [{ name: "staging", vars: {} }],
          skipped: ["/ws/environments/bad.toml: expected an equals"],
        });
      return Promise.resolve(undefined);
    });

    render(<EnvBar />);

    await waitFor(() =>
      expect(screen.getByText(/bad\.toml/)).toBeTruthy(),
    );
    expect(screen.getByText(/Skipped 1 unreadable environment file/)).toBeTruthy();
  });
});
