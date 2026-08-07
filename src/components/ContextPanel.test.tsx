import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "../store/env";
import { useLayout } from "../store/layout";
import { ContextPanel } from "./ContextPanel";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const save = vi.fn();

describe("ContextPanel", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    save.mockReset();
    save.mockResolvedValue(undefined);
    localStorage.clear();
    useLayout.setState({ contextOpen: true });
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
              source: "file",
            },
            apiKey: {
              shared: false,
              secret: true,
              value: null,
              hosts: [],
              set: true,
              source: "local",
            },
            adminKey: {
              shared: false,
              secret: true,
              value: null,
              hosts: [],
              set: false,
              source: "local",
            },
          },
        },
      ],
      selected: "staging",
      error: null,
      save,
    });
  });

  afterEach(cleanup);

  it("masks a held secret and never offers an editable value box for it", () => {
    render(<ContextPanel />);

    expect(screen.getByText("••••••••")).toBeTruthy();
    expect(screen.getByText("not set on this machine")).toBeTruthy();
    expect(screen.queryByLabelText("Value for apiKey")).toBeNull();
    expect(screen.queryByLabelText("Value for adminKey")).toBeNull();
    expect(screen.getByLabelText("Value for baseUrl")).toBeTruthy();
  });

  it("commits a shared value through save with the merged plain vars", async () => {
    render(<ContextPanel />);
    const input = screen.getByLabelText("Value for baseUrl");

    fireEvent.change(input, { target: { value: "https://prod.dev" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        name: "staging",
        vars: { baseUrl: "https://prod.dev" },
      }),
    );
  });

  it("does not write when the value is unchanged", () => {
    render(<ContextPanel />);
    fireEvent.blur(screen.getByLabelText("Value for baseUrl"));

    expect(save).not.toHaveBeenCalled();
  });

  it("explains the empty case when no environment is selected", () => {
    useEnv.setState({ selected: null });
    render(<ContextPanel />);

    expect(screen.getByText("No environment selected")).toBeTruthy();
    expect(screen.getByText(/Pick one in the header/)).toBeTruthy();
  });

  it("says so when the selected environment is gone from the workspace", () => {
    useEnv.setState({ envs: [] });
    render(<ContextPanel />);

    expect(screen.getByText(/is gone from the workspace/)).toBeTruthy();
    expect(screen.queryByText("No environment selected")).toBeNull();
  });

  it("closes on Escape from inside the panel", () => {
    render(<ContextPanel />);

    fireEvent.keyDown(screen.getByLabelText("Value for baseUrl"), {
      key: "Escape",
    });
    expect(useLayout.getState().contextOpen).toBe(true);

    fireEvent.keyDown(screen.getByLabelText("Environment panel"), {
      key: "Escape",
    });
    expect(useLayout.getState().contextOpen).toBe(false);
  });
});
