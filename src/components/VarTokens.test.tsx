import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { EnvironmentView } from "../lib/api";
import { describeVar } from "../lib/vars";
import { VarPill, VarToken } from "./VarTokens";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const env: EnvironmentView = {
  name: "hosted",
  vars: {
    baseUrl: { secret: false, value: "https://api.mandalo.dev", set: true },
    apiKey: { secret: true, value: null, hosts: [], set: true },
    adminKey: { secret: true, value: null, hosts: [], set: false },
  },
};

function hoverToken(name: string): HTMLElement {
  const { container } = render(
    <VarToken description={describeVar(name, env)} text={`{{${name}}}`} />,
  );
  const token = container.querySelector(".var-token") as HTMLElement;
  fireEvent.mouseEnter(token);
  return token;
}

describe("variable hover", () => {
  afterEach(cleanup);

  it("shows nothing until the token is hovered", () => {
    render(
      <VarToken description={describeVar("baseUrl", env)} text="{{baseUrl}}" />,
    );
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows the value and the environment it came from", () => {
    hoverToken("baseUrl");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("{{baseUrl}}");
    expect(tip.textContent).toContain("https://api.mandalo.dev");
    expect(tip.textContent).toContain("from environment hosted");
  });

  it("masks a secret and says where it lives, never its value", () => {
    hoverToken("apiKey");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("••••••••");
    expect(tip.textContent).toContain("secret — stored in your OS keychain");
    expect(tip.textContent).toContain("Set on this machine.");
    expect(document.body.textContent).not.toContain("value");
  });

  it("says a secret is not set on this machine", () => {
    hoverToken("adminKey");
    expect(screen.getByRole("tooltip").textContent).toContain(
      "Not set on this machine",
    );
  });

  it("names the environment an unresolved variable is missing from", () => {
    hoverToken("nope");
    const tip = screen.getByRole("tooltip");
    expect(tip.textContent).toContain("not set");
    expect(tip.textContent).toContain("not defined in environment hosted");
  });

  it("keeps the red styling for an unresolved token", () => {
    const { container } = render(
      <VarToken description={describeVar("nope", env)} text="{{nope}}" />,
    );
    expect(container.querySelector(".var-token")?.className).toContain("var-bad");
  });

  it("hides the popover again on mouse out", () => {
    const token = hoverToken("baseUrl");
    fireEvent.mouseLeave(token);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("reaches keyboard users through the focusable pill", () => {
    render(<VarPill description={describeVar("baseUrl", env)} />);
    const pill = screen.getByText("baseUrl");
    expect(pill.getAttribute("tabindex")).toBe("0");
    fireEvent.focus(pill);
    expect(screen.getByRole("tooltip").textContent).toContain(
      "https://api.mandalo.dev",
    );
  });

  it("never prints a secret in the pill either", () => {
    render(<VarPill description={describeVar("apiKey", env)} />);
    expect(screen.getByText("••••••••")).toBeTruthy();
  });
});
