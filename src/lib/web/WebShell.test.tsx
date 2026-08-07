import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../../App", () => ({ default: () => <div /> }));

import { useRemote } from "../../store/remote";
import { WebShell } from "./WebShell";

afterEach(() => {
  cleanup();
  useRemote.setState({ dialogOpen: false, prefill: "" });
  window.history.replaceState(null, "", "/app");
});

describe("the web ribbon", () => {
  it("offers no one-click action that erases the workspace", () => {
    const view = render(<WebShell />);

    for (const control of view.getAllByRole("button"))
      expect(control.textContent ?? "").not.toMatch(/reset|erase|wipe|clear/i);
  });
});

describe("the deep link", () => {
  it("opens the review for ?repo=owner/name and never the collection itself", () => {
    window.history.replaceState(null, "", "/app?repo=acme/collections");

    render(<WebShell />);

    expect(useRemote.getState().dialogOpen).toBe(true);
    expect(useRemote.getState().prefill).toBe("acme/collections");
  });

  it("takes the repository out of the address bar once it has been read", () => {
    window.history.replaceState(null, "", "/app?repo=acme/collections");

    render(<WebShell />);

    expect(window.location.search).toBe("");
  });

  it("does nothing at all without one", () => {
    render(<WebShell />);

    expect(useRemote.getState().dialogOpen).toBe(false);
  });
});
