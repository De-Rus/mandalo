import { beforeEach, describe, expect, it, vi } from "vitest";
import { plainVars, secretNames, useEnv, varLabel } from "./env";

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
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [
            {
              name: "staging",
              vars: { a: { shared: true, secret: false, value: "1", set: true } },
            },
          ],
          skipped: [],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().init("/ws");

    const s = useEnv.getState();
    expect(s.workspace).toBe("/ws");
    expect(s.envs.map((e) => e.name)).toEqual(["staging"]);
    expect(s.error).toBeNull();
  });

  it("init surfaces skipped environment files as a warning line", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [{ name: "staging", vars: {} }],
          skipped: ["/ws/environments/bad.toml: expected an equals"],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().init("/ws");

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

  it("surfaces a failed reload instead of rejecting into nothing", async () => {
    useEnv.setState({ workspace: "/ws" });
    invoke.mockRejectedValue("environments dir is unreadable");

    await useEnv.getState().refresh();

    expect(useEnv.getState().error).toContain("unreadable");
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

describe("environment declarations", () => {
  const STAGING = {
    name: "staging",
    vars: {
      baseUrl: { shared: true, secret: false as const, value: "https://x.dev", set: true },
      token: {
        shared: false,
        secret: true as const,
        value: null,
        hosts: ["x.dev"],
        set: true,
      },
      adminToken: {
        shared: false,
        secret: true as const,
        value: null,
        hosts: [],
        set: false,
      },
    },
  };

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    localStorage.clear();
    useEnv.setState({
      workspace: "/ws",
      envs: [STAGING],
      selected: "staging",
      error: null,
    });
  });

  it("hands interpolation the plain values only", () => {
    expect(plainVars(STAGING)).toEqual({ baseUrl: "https://x.dev" });
  });

  it("names the secrets without exposing a value", () => {
    expect(secretNames(STAGING)).toEqual(["token", "adminToken"]);
    expect(varLabel(STAGING.vars.token)).toBe("••••••••");
    expect(varLabel(STAGING.vars.adminToken)).toBe("not set on this machine");
  });

  it("stores a secret out of band and reloads the declarations", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [STAGING], skipped: [] });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().storeSecret("staging", "token", "s3cr3t");

    expect(invoke).toHaveBeenCalledWith("set_secret", {
      workspace: "/ws",
      env: "staging",
      key: "token",
      value: "s3cr3t",
    });
    expect(invoke.mock.calls.map((c) => c[0])).toContain("list_environments");
  });

  it("clears, binds and deletes through the declaration commands", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [STAGING], skipped: [] });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().forgetSecret("staging", "token");
    await useEnv.getState().bindHost("staging", "token", "api.x.dev");
    await useEnv.getState().removeVar("staging", "adminToken");

    const commands = invoke.mock.calls.map((c) => c[0]);
    expect(commands).toContain("clear_secret");
    expect(commands).toContain("bind_secret_host");
    expect(commands).toContain("delete_var");
    expect(invoke).toHaveBeenCalledWith("bind_secret_host", {
      workspace: "/ws",
      env: "staging",
      key: "token",
      host: "api.x.dev",
    });
  });

  it("adds a variable to an existing environment, keeping the ones already there", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [STAGING], skipped: [] });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().addVar("staging", "  userId  ", "7");

    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: { name: "staging", vars: { baseUrl: "https://x.dev", userId: "7" } },
    });
    expect(invoke.mock.calls.map((c) => c[0])).toContain("list_environments");
  });

  it("refuses to write a secret's value into the file, loudly", async () => {
    await expect(
      useEnv.getState().addVar("staging", "token", "s3cr3t"),
    ).rejects.toThrow(/declared secret/);

    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("save_environment");
    expect(useEnv.getState().error).toContain("declared secret");
  });

  it("refuses an unnamed variable and an unknown environment", async () => {
    await expect(
      useEnv.getState().addVar("staging", "   ", "1"),
    ).rejects.toThrow(/name is required/);
    await expect(useEnv.getState().addVar("nope", "a", "1")).rejects.toThrow(
      /not loaded/,
    );
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("save_environment");
  });

  it("creates an environment with its first variable and selects it", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({
          items: [STAGING, { name: "prod", vars: {} }],
          skipped: [],
        });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().createEnv("  prod  ", { baseUrl: "https://x.com" });

    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: { name: "prod", vars: { baseUrl: "https://x.com" } },
    });
    expect(useEnv.getState().selected).toBe("prod");
    expect(localStorage.getItem("mandalo.env.selected")).toBe("prod");
  });

  it("refuses an empty or already taken environment name", async () => {
    await expect(useEnv.getState().createEnv(" ")).rejects.toThrow(
      /name is required/,
    );
    await expect(useEnv.getState().createEnv("staging")).rejects.toThrow(
      /already exists/,
    );
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("save_environment");
    expect(useEnv.getState().error).toContain("already exists");
  });

  it("never writes a declaration object back into the environment file", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [STAGING], skipped: [] });
      return Promise.resolve(undefined);
    });

    await useEnv.getState().applyVarWrites({ userId: "7" }, []);

    expect(invoke).toHaveBeenCalledWith("save_environment", {
      workspace: "/ws",
      env: {
        name: "staging",
        vars: { baseUrl: "https://x.dev", userId: "7" },
      },
    });
  });
});
