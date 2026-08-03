import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Tree } from "../lib/api";
import { useCollection } from "../store/collection";
import { CommandPalette } from "./CommandPalette";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const TREE: Tree = {
  collections: [
    {
      id: "c1",
      slug: "acme",
      name: "Acme API",
      folders: [
        {
          name: "Users",
          path: "users",
          folders: [],
          requests: [
            {
              id: "r1",
              name: "List users",
              kind: "http",
              method: "GET",
              path: "users/list.toml",
            },
          ],
        },
      ],
      requests: [
        {
          id: "r2",
          name: "Health check",
          kind: "http",
          method: "GET",
          path: "health.toml",
        },
      ],
    },
  ],
  skipped: [],
};

describe("CommandPalette", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    localStorage.clear();
    useCollection.setState({
      workspace: "/ws",
      tree: TREE,
      drafts: {},
      activeId: null,
    });
  });

  afterEach(cleanup);

  it("lists every request with where it lives", () => {
    render(<CommandPalette onClose={() => {}} />);
    expect(screen.getByText("List users")).toBeTruthy();
    expect(screen.getByText("Acme API / Users")).toBeTruthy();
    expect(screen.getByText("Acme API")).toBeTruthy();
  });

  it("filters fuzzily as you type", () => {
    render(<CommandPalette onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Jump to a request"), {
      target: { value: "lus" },
    });
    expect(screen.getByText("List users")).toBeTruthy();
    expect(screen.queryByText("Health check")).toBeNull();
  });

  it("says so when nothing matches", () => {
    render(<CommandPalette onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Jump to a request"), {
      target: { value: "zzzz" },
    });
    expect(screen.getByText(/No request matches/)).toBeTruthy();
  });

  it("opens the highlighted request on Enter and closes", () => {
    const onClose = vi.fn();
    render(<CommandPalette onClose={onClose} />);
    const input = screen.getByLabelText("Jump to a request");

    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(useCollection.getState().activeId).toBe("r1");
    expect(onClose).toHaveBeenCalled();
  });
});
