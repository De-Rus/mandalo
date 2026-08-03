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
              secret: false,
              value: "https://staging.dev",
              set: true,
            },
            token: { secret: false, value: "", set: true },
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

  it("lists the active environment variables in the quick-look popover", () => {
    render(<EnvBar />);
    expect(screen.queryByText("baseUrl")).toBeNull();

    fireEvent.click(screen.getByLabelText("Environment quick look"));

    expect(screen.getByText("baseUrl")).toBeTruthy();
    expect(screen.getByText("https://staging.dev")).toBeTruthy();
    expect(screen.getByText("—")).toBeTruthy();
  });

  it("masks a secret and says when this machine has no value for it", () => {
    useEnv.setState({
      envs: [
        {
          name: "staging",
          vars: {
            apiKey: { secret: true, value: null, hosts: [], set: true },
            adminKey: { secret: true, value: null, hosts: [], set: false },
          },
        },
      ],
    });
    render(<EnvBar />);
    fireEvent.click(screen.getByLabelText("Environment quick look"));

    expect(screen.getByText("••••••••")).toBeTruthy();
    expect(screen.getByText("not set on this machine")).toBeTruthy();
  });

  it("explains the empty case when no environment is selected", () => {
    useEnv.setState({ selected: null });
    render(<EnvBar />);

    fireEvent.click(screen.getByLabelText("Environment quick look"));
    expect(
      screen.getByText("No environment", { selector: ".popover-title" }),
    ).toBeTruthy();
    expect(screen.getByText(/Select an environment/)).toBeTruthy();
  });

  it("opens the full editor from the popover", () => {
    render(<EnvBar />);
    fireEvent.click(screen.getByLabelText("Environment quick look"));
    fireEvent.click(screen.getByText("Edit"));

    expect(screen.getByText("Environments")).toBeTruthy();
  });
});
