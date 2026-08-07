import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "../store/env";
import { EnvBar } from "./EnvBar";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("EnvBar", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    localStorage.clear();
    useEnv.setState({
      workspace: "/ws",
      envs: [
        {
          name: "staging",
          vars: {
            baseUrl: {
              shared: true,
              secret: false,
              value: "https://staging.dev",
              set: true,
            },
            token: { shared: true, secret: false, value: "", set: true },
          },
        },
      ],
      selected: "staging",
      error: null,
    });
  });

  afterEach(cleanup);

  it("shows a warning line for environment files the backend could not parse", () => {
    useEnv.setState({
      error: "Skipped 1 unreadable environment file(s): /ws/bad.toml: oops",
    });
    render(<EnvBar />);

    expect(screen.getByText(/bad\.toml/)).toBeTruthy();
    expect(screen.getByText(/Skipped 1 unreadable environment file/)).toBeTruthy();
  });

  it("lists environments in a menu with delete on each row", () => {
    render(<EnvBar />);

    fireEvent.click(screen.getByLabelText("Active environment"));

    expect(screen.getByRole("menuitem", { name: /No Environment/ })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: /staging/ })).toBeTruthy();
    expect(screen.getByLabelText("Delete staging")).toBeTruthy();
  });

  it("creates an environment from the menu", () => {
    render(<EnvBar />);

    fireEvent.click(screen.getByLabelText("Active environment"));
    fireEvent.click(screen.getByText("New environment…"));

    expect(screen.getByPlaceholderText("staging")).toBeTruthy();
  });

  it("does not show the old quick-look eye", () => {
    render(<EnvBar />);
    expect(screen.queryByLabelText("Environment quick look")).toBeNull();
    expect(screen.queryByLabelText("New environment")).toBeNull();
  });
});
