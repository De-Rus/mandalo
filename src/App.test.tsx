import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

afterEach(cleanup);

// Every other suite renders one component. Nothing rendered the shell, so a
// throw during its first paint reached users as a white window with a green
// test run behind it.
describe("the app shell", () => {
  it("paints on a cold start, before any workspace has answered", () => {
    invoke.mockImplementation(() => new Promise(() => {}));

    const view = render(<App />);

    expect(view.container.querySelector(".app")).toBeTruthy();
  });

  it("paints when every backend call fails", async () => {
    invoke.mockRejectedValue(new Error("no backend here"));

    const view = render(<App />);
    await Promise.resolve();

    expect(view.container.querySelector(".app")).toBeTruthy();
  });
});
