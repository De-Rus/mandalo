import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "../store/env";
import { EnvList } from "./EnvList";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("EnvList", () => {
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
            baseUrl: { shared: true, secret: false, value: "https://s.dev", set: true },
            token: { shared: false, secret: true, value: null, hosts: [], set: true },
          },
        },
        { name: "prod", vars: {} },
      ],
      selected: "staging",
      error: null,
    });
  });

  afterEach(cleanup);

  it("marks the active environment and selects another on click", () => {
    render(<EnvList />);

    expect(screen.getByRole("option", { name: /staging/ }).getAttribute("aria-selected")).toBe(
      "true",
    );

    fireEvent.click(screen.getByRole("button", { name: "prod" }));

    expect(useEnv.getState().selected).toBe("prod");
  });

  it("counts variables and names the secret ones", () => {
    render(<EnvList />);

    expect(screen.getByText("2 vars · 1 secret")).toBeTruthy();
    expect(screen.getByText("0 vars")).toBeTruthy();
  });

  it("opens the environment editor without leaving the tab", () => {
    render(<EnvList />);

    fireEvent.click(screen.getByLabelText("Edit staging"));

    expect(screen.getByRole("heading", { name: "Environments" })).toBeTruthy();
  });

  it("offers delete on each environment row", () => {
    render(<EnvList />);
    expect(screen.getByLabelText("Delete staging")).toBeTruthy();
    expect(screen.getByLabelText("Delete prod")).toBeTruthy();
  });

  it("shows the unreadable-file error with a dismiss button", () => {
    useEnv.setState({ error: "Skipped 1 unreadable environment file(s): bad.toml" });
    render(<EnvList />);

    expect(screen.getByText(/bad\.toml/)).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Dismiss error"));
    expect(useEnv.getState().error).toBeNull();
  });

  it("explains what an environment is when there are none", () => {
    useEnv.setState({ envs: [] });
    render(<EnvList />);

    expect(screen.getByText("No environments yet")).toBeTruthy();
    expect(screen.getByText(/Create one from the header/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "New environment" })).toBeNull();
  });

  it("does not offer create in the sidebar list", () => {
    render(<EnvList />);
    expect(screen.queryByRole("button", { name: "New environment" })).toBeNull();
  });
});
