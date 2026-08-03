import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "../store/env";
import { EnvModal } from "./EnvModal";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("EnvModal", () => {
  beforeEach(() => {
    invoke.mockReset();
    localStorage.clear();
    useEnv.setState({
      workspace: "/ws",
      envs: [{ name: "old", vars: { a: "1" } }],
      selected: "old",
      error: null,
    });
  });

  afterEach(cleanup);

  it("renaming saves the new name, deletes the old file, and moves the selection", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [{ name: "new", vars: { a: "1" } }], skipped: [] });
      return Promise.resolve(undefined);
    });
    render(<EnvModal onClose={() => {}} />);

    fireEvent.click(screen.getByText("Edit"));
    fireEvent.change(screen.getByPlaceholderText("staging"), {
      target: { value: "new" },
    });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("delete_environment", {
        workspace: "/ws",
        name: "old",
      }),
    );
    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: { name: "new", vars: { a: "1" } },
    });
    expect(useEnv.getState().selected).toBe("new");
  });

  it("saving without a rename never deletes", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [{ name: "old", vars: { a: "2" } }], skipped: [] });
      return Promise.resolve(undefined);
    });
    render(<EnvModal onClose={() => {}} />);

    fireEvent.click(screen.getByText("Edit"));
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_environment", {
        workspace: "/ws",
        env: { name: "old", vars: { a: "1" } },
      }),
    );
    expect(
      invoke.mock.calls.map((c) => c[0]),
    ).not.toContain("delete_environment");
    expect(useEnv.getState().selected).toBe("old");
  });
});
