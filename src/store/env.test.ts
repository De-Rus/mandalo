import { beforeEach, describe, expect, it, vi } from "vitest";
import { useEnv } from "./env";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("env store", () => {
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

  it("init loads environments and clears the error line when nothing is skipped", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [{ name: "staging", vars: { a: "1" } }],
          skipped: [],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().init();

    const s = useEnv.getState();
    expect(s.workspace).toBe("/ws");
    expect(s.envs.map((e) => e.name)).toEqual(["staging"]);
    expect(s.error).toBeNull();
  });

  it("init surfaces skipped environment files as a warning line", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "default_workspace_dir") return Promise.resolve("/ws");
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [{ name: "staging", vars: {} }],
          skipped: ["/ws/environments/bad.toml: expected an equals"],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().init();

    const s = useEnv.getState();
    expect(s.envs.map((e) => e.name)).toEqual(["staging"]);
    expect(s.error).toContain("bad.toml");
    expect(s.error).toContain("expected an equals");
  });

  it("keeps the warning line after a save reloads the list", async () => {
    useEnv.setState({ workspace: "/ws" });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [{ name: "staging", vars: {} }],
          skipped: ["/ws/environments/bad.toml: expected an equals"],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().save({ name: "staging", vars: {} });

    expect(useEnv.getState().error).toContain("bad.toml");
  });

  it("keeps the warning line after a delete reloads the list", async () => {
    useEnv.setState({ workspace: "/ws", selected: "gone" });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [],
          skipped: ["/ws/environments/bad.toml: expected an equals"],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().remove("gone");

    const s = useEnv.getState();
    expect(s.selected).toBeNull();
    expect(s.error).toContain("bad.toml");
  });
});
