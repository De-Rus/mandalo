import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EnvironmentView } from "../lib/api";
import { describeVar } from "../lib/vars";
import { useEnv } from "../store/env";
import { VarToken } from "./VarTokens";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const SECRET = "s3cr3t-do-not-render";

const env: EnvironmentView = {
  name: "hosted",
  vars: {
    baseUrl: {
      shared: true,
      secret: false,
      value: "https://api.mandalo.dev",
      set: true,
      source: "file",
    },
    apiKey: {
      shared: false,
      secret: true,
      value: null,
      hosts: ["api.acme.com"],
      set: true,
      source: "local",
    },
    adminKey: {
      shared: false,
      secret: true,
      value: null,
      hosts: [],
      set: false,
    },
    devUrl: {
      shared: false,
      secret: false,
      value: null,
      hosts: [],
      set: true,
      source: "local",
    },
    unsetLocal: {
      shared: false,
      secret: false,
      value: null,
      hosts: [],
      set: false,
    },
  },
};

function token(name: string, view: EnvironmentView | null = env): HTMLElement {
  const { container } = render(
    <VarToken description={describeVar(name, view)} text={`{{${name}}}`} />,
  );
  return container.querySelector(".var-token") as HTMLElement;
}

function hoverToken(name: string, view: EnvironmentView = env): HTMLElement {
  const el = token(name, view);
  fireEvent.mouseEnter(el);
  return el;
}

describe("variable hover", () => {
  afterEach(cleanup);

  it("shows nothing until the token is hovered", () => {
    token("baseUrl");
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows a shared value and the environment it came from", () => {
    hoverToken("baseUrl");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("{{baseUrl}}");
    expect(tip.textContent).toContain("https://api.mandalo.dev");
    expect(tip.textContent).toContain("shared — from environment hosted");
  });

  it("hides the popover again on mouse out", () => {
    const el = hoverToken("baseUrl");
    fireEvent.mouseLeave(el);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("reaches keyboard users through the token itself", () => {
    const el = token("baseUrl");
    expect(el.getAttribute("tabindex")).toBe("0");
    fireEvent.focus(el);
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("https://api.mandalo.dev");
    expect(el.getAttribute("aria-describedby")).toBe(tip.id);
    fireEvent.blur(el);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("dismisses the popover with Escape", () => {
    const el = token("baseUrl");
    fireEvent.focus(el);
    fireEvent.keyDown(el, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("names the environment an unresolved variable is missing from", () => {
    hoverToken("nope");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("not set");
    expect(tip.textContent).toContain("not defined in environment hosted");
  });

  it("keeps the red styling for an unresolved token", () => {
    expect(token("nope").className).toContain("var-bad");
  });
});

describe("a secret's value never reaches the screen", () => {
  afterEach(cleanup);

  it("renders the masked branch and says the value stays on this machine", () => {
    hoverToken("apiKey");
    const tip = screen.getByRole("tooltip");
    expect(tip.querySelector(".var-pop-mask")?.textContent).toBe("••••••••");
    expect(tip.textContent).toContain("the value stays on this machine");
    expect(tip.textContent).toContain("Only sent to api.acme.com.");
  });

  it("never puts the value in the DOM, even if the backend sent one", () => {
    const leaky: EnvironmentView = {
      name: "hosted",
      vars: {
        apiKey: {
          shared: false,
          secret: true,
          value: SECRET,
          hosts: [],
          set: true,
          source: "local",
        },
      },
    };
    hoverToken("apiKey", leaky);
    expect(document.body.textContent).not.toContain(SECRET);
    expect(document.body.textContent).not.toContain(SECRET.slice(0, 6));
  });

  it("masks with a fixed width that cannot reveal the length", () => {
    hoverToken("apiKey");
    expect(
      screen.getByRole("tooltip").querySelector(".var-pop-mask")?.textContent
        ?.length,
    ).toBe(8);
  });

  it("says a secret is not set on this machine and names the file", () => {
    hoverToken("adminKey");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("Not set on this machine");
    expect(tip.textContent).toContain("~/.config/mandalo/secrets.toml");
    expect(tip.textContent).toContain("Not bound to a host yet");
  });
});

describe("a local variable is not a secret", () => {
  afterEach(cleanup);

  it("is not masked, and says it is local rather than confidential", () => {
    hoverToken("devUrl");
    const tip = screen.getByRole("tooltip");
    expect(tip.querySelector(".var-pop-mask")).toBeNull();
    expect(tip.textContent).toContain("set on this machine");
    expect(tip.textContent).toContain("not shared with your team");
    expect(tip.textContent).toContain("not confidential");
  });

  it("says which layer the value came from", () => {
    hoverToken("devUrl");
    expect(screen.getByRole("tooltip").textContent).toContain(
      "~/.config/mandalo/secrets.toml",
    );
  });

  it("says an exported variable wins over the file", () => {
    const exported: EnvironmentView = {
      name: "hosted",
      vars: {
        devUrl: {
          shared: false,
          secret: false,
          value: null,
          hosts: [],
          set: true,
          source: "environment",
        },
      },
    };
    hoverToken("devUrl", exported);
    expect(screen.getByRole("tooltip").textContent).toContain(
      "MANDALO_SECRET__",
    );
  });

  it("warns when nothing holds a value for it", () => {
    hoverToken("unsetLocal");
    expect(screen.getByRole("tooltip").textContent).toContain(
      "the request will fail",
    );
  });

  it("does not colour a local variable as an error", () => {
    expect(token("devUrl").className).toContain("var-local");
    expect(token("devUrl").className).not.toContain("var-bad");
  });
});

describe("adding a missing variable from the token", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_environments")
        return Promise.resolve({ items: [env], skipped: [] });
      return Promise.resolve(undefined);
    });
    localStorage.clear();
    useEnv.setState({
      workspace: "/ws",
      envs: [env],
      selected: "hosted",
      error: null,
    });
  });

  afterEach(cleanup);

  it("writes the value into the selected environment, keeping what it already had", async () => {
    fireEvent.click(token("userId"));

    fireEvent.change(screen.getByLabelText("Value for userId"), {
      target: { value: "7" },
    });
    fireEvent.click(screen.getByText("Add to hosted"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_environment", {
        workspace: "/ws",
        env: {
          name: "hosted",
          vars: { baseUrl: "https://api.mandalo.dev", userId: "7" },
        },
      }),
    );
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("creates an environment holding the variable when none is selected", async () => {
    useEnv.setState({ envs: [], selected: null });
    fireEvent.click(token("baseUrl", null));

    fireEvent.change(screen.getByLabelText("New environment name"), {
      target: { value: "staging" },
    });
    fireEvent.change(screen.getByLabelText("Value for baseUrl"), {
      target: { value: "https://staging.dev" },
    });
    fireEvent.click(screen.getByText("Create environment"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("save_environment", {
        workspace: "/ws",
        env: { name: "staging", vars: { baseUrl: "https://staging.dev" } },
      }),
    );
    expect(useEnv.getState().selected).toBe("staging");
  });

  it("shows the failure in place instead of pretending the write happened", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "save_environment")
        return Promise.reject("environments dir is read-only");
      return Promise.resolve({ items: [env], skipped: [] });
    });
    fireEvent.click(token("userId"));
    fireEvent.click(screen.getByText("Add to hosted"));

    await waitFor(() =>
      expect(screen.getByText(/read-only/)).toBeTruthy(),
    );
    expect(screen.getByRole("tooltip")).toBeTruthy();
  });

  it("never offers to write a secret's value into the file", () => {
    fireEvent.click(token("adminKey"));
    expect(screen.queryByLabelText("Value for adminKey")).toBeNull();
  });

  it("closes the pinned popover on Escape", () => {
    const el = token("userId");
    fireEvent.click(el);
    expect(screen.getByLabelText("Value for userId")).toBeTruthy();
    fireEvent.keyDown(el, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
});
