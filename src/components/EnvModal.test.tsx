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
      envs: [{ name: "old", vars: { a: { shared: true, secret: false, value: "1", set: true } } }],
      selected: "old",
      error: null,
    });
  });

  afterEach(cleanup);

  it("renaming saves the new name, deletes the old file, and moves the selection", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [
            { name: "new", vars: { a: { shared: true, secret: false, value: "1", set: true } } },
          ],
          skipped: [],
        });
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
        return Promise.resolve({
          items: [
            { name: "old", vars: { a: { shared: true, secret: false, value: "2", set: true } } },
          ],
          skipped: [],
        });
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

  it("shows a secret as masked with write-only actions and its bound hosts", async () => {
    useEnv.setState({
      envs: [
        {
          name: "old",
          vars: {
            a: { shared: true, secret: false, value: "1", set: true },
            token: {
              shared: false,
              secret: true,
              value: null,
              hosts: ["api.acme.com"],
              set: true,
            },
            adminToken: { shared: false, secret: true, value: null, hosts: [], set: false },
          },
        },
      ],
    });
    invoke.mockResolvedValue(undefined);
    render(<EnvModal onClose={() => {}} />);
    fireEvent.click(screen.getByText("Edit"));

    expect(screen.getByText("••••••••")).toBeTruthy();
    expect(screen.getByText("not set on this machine")).toBeTruthy();
    expect(screen.getByText("api.acme.com")).toBeTruthy();
    expect(screen.getByText("any host — not bound yet")).toBeTruthy();
    expect(screen.queryByDisplayValue("token")).toBeNull();

    fireEvent.click(screen.getAllByText("Set value")[0]);
    fireEvent.change(screen.getByLabelText("Value for token"), {
      target: { value: "s3cr3t" },
    });
    fireEvent.click(screen.getByText("Store"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("set_secret", {
        workspace: "/ws",
        env: "old",
        key: "token",
        value: "s3cr3t",
      }),
    );
  });

  it("binds a secret to a host from the editor", async () => {
    useEnv.setState({
      envs: [
        {
          name: "old",
          vars: {
            token: { shared: false, secret: true, value: null, hosts: [], set: true },
          },
        },
      ],
    });
    invoke.mockResolvedValue(undefined);
    render(<EnvModal onClose={() => {}} />);
    fireEvent.click(screen.getByText("Edit"));

    fireEvent.click(screen.getByText("Bind host"));
    fireEvent.change(screen.getByLabelText("Host for token"), {
      target: { value: "api.acme.com" },
    });
    fireEvent.click(screen.getByText("Bind"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("bind_secret_host", {
        workspace: "/ws",
        env: "old",
        key: "token",
        host: "api.acme.com",
      }),
    );
  });

  it("keeps a secret out of the plain rows it saves", async () => {
    useEnv.setState({
      envs: [
        {
          name: "old",
          vars: {
            a: { shared: true, secret: false, value: "1", set: true },
            token: { shared: false, secret: true, value: null, hosts: [], set: true },
          },
        },
      ],
    });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [], skipped: [] });
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
  });
});
