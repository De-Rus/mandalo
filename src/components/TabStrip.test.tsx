import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Tree } from "../lib/api";
import { newDraft, type RequestDraft } from "../lib/draft";
import { useCollection } from "../store/collection";
import { useTabs } from "../store/tabs";
import { TabStrip } from "./TabStrip";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function draft(id: string, name: string): RequestDraft {
  return { ...newDraft(name), id, collection: "acme", path: `${id}.toml` };
}

const TREE: Tree = {
  collections: [
    {
      id: "c1",
      slug: "acme",
      name: "Acme API",
      folders: [],
      requests: [
        { id: "a", name: "First", kind: "http", method: "GET", path: "a.toml" },
        { id: "b", name: "Second", kind: "http", method: "POST", path: "b.toml" },
        { id: "c", name: "Third", kind: "http", method: "GET", path: "c.toml" },
      ],
    },
  ],
  skipped: [],
};

describe("TabStrip", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    localStorage.clear();
    useTabs.setState({ openIds: ["a", "b", "c"], dirtyIds: ["b"] });
    useCollection.setState({
      workspace: "/ws",
      tree: TREE,
      drafts: {
        a: draft("a", "First"),
        b: { ...draft("b", "Second"), method: "POST" },
        c: draft("c", "Third"),
      },
      activeId: "a",
    });
  });

  afterEach(cleanup);

  it("renders one tab per open request with its method badge", () => {
    render(<TabStrip />);
    expect(screen.getByText("First")).toBeTruthy();
    expect(screen.getByText("Second")).toBeTruthy();
    expect(screen.getByText("POST")).toBeTruthy();
  });

  it("marks the unsaved tab with a dirty dot", () => {
    const { container } = render(<TabStrip />);
    expect(container.querySelectorAll(".rtab-dirty")).toHaveLength(1);
  });

  it("activates a tab on click", () => {
    render(<TabStrip />);
    fireEvent.click(screen.getByText("Second"));
    expect(useCollection.getState().activeId).toBe("b");
  });

  it("closing the active tab moves the selection to its neighbour", () => {
    render(<TabStrip />);
    fireEvent.click(screen.getByLabelText("Close First"));
    expect(useTabs.getState().openIds).toEqual(["b", "c"]);
    expect(useCollection.getState().activeId).toBe("b");
  });

  it("closing the last tab leaves nothing active", () => {
    useTabs.setState({ openIds: ["a"], dirtyIds: [] });
    render(<TabStrip />);
    fireEvent.click(screen.getByLabelText("Close First"));
    expect(useTabs.getState().openIds).toEqual([]);
    expect(useCollection.getState().activeId).toBeNull();
  });

  it("middle click closes a tab", () => {
    render(<TabStrip />);
    fireEvent(
      screen.getByText("Second"),
      new MouseEvent("auxclick", { bubbles: true, button: 1 }),
    );
    expect(useTabs.getState().openIds).toEqual(["a", "c"]);
  });

  it("offers the editor close actions on right click", () => {
    render(<TabStrip />);
    fireEvent.contextMenu(screen.getByText("First"));

    const menu = screen.getByRole("menu", { name: "Tab actions" });
    expect(menu.textContent).toContain("Close");
    expect(menu.textContent).toContain("Close Others");
    expect(menu.textContent).toContain("Close to the Right");
    expect(menu.textContent).toContain("Close All");
  });

  it("duplicates the right clicked tab", () => {
    const real = useCollection.getState().duplicateRequest;
    const duplicateRequest = vi.fn().mockResolvedValue(undefined);
    useCollection.setState({ duplicateRequest });
    try {
      render(<TabStrip />);
      fireEvent.contextMenu(screen.getByText("Second"));
      fireEvent.click(screen.getByText("Duplicate"));

      expect(duplicateRequest).toHaveBeenCalledWith("b");
      expect(useTabs.getState().openIds).toEqual(["a", "b", "c"]);
    } finally {
      useCollection.setState({ duplicateRequest: real });
    }
  });

  it("closes the other tabs, keeping the one clicked", () => {
    useTabs.setState({ dirtyIds: [] });
    render(<TabStrip />);
    fireEvent.contextMenu(screen.getByText("Third"));
    fireEvent.click(screen.getByText("Close Others"));

    expect(useTabs.getState().openIds).toEqual(["c"]);
    expect(useCollection.getState().activeId).toBe("c");
  });

  it("closes everything to the right of the tab clicked", () => {
    useTabs.setState({ dirtyIds: [] });
    render(<TabStrip />);
    fireEvent.contextMenu(screen.getByText("First"));
    fireEvent.click(screen.getByText("Close to the Right"));

    expect(useTabs.getState().openIds).toEqual(["a"]);
    expect(useCollection.getState().activeId).toBe("a");
  });

  it("closes every tab from the strip menu, without right clicking", () => {
    useTabs.setState({ dirtyIds: [] });
    render(<TabStrip />);
    fireEvent.mouseDown(screen.getByLabelText("Tab actions"));
    fireEvent.click(screen.getByText("Close All"));

    expect(useTabs.getState().openIds).toEqual([]);
    expect(useCollection.getState().activeId).toBeNull();
  });

  it("asks once, not once per tab, when several closing tabs are unsaved", () => {
    useTabs.setState({ dirtyIds: ["a", "b", "c"] });
    render(<TabStrip />);
    fireEvent.mouseDown(screen.getByLabelText("Tab actions"));
    fireEvent.click(screen.getByText("Close All"));

    expect(screen.getAllByText("Unsaved changes")).toHaveLength(1);
    expect(screen.getByText(/3 of the 3 tabs/)).toBeTruthy();
    expect(screen.queryByText(/tabs .* has edits/)).toBeNull();
    expect(useTabs.getState().openIds).toEqual(["a", "b", "c"]);

    fireEvent.click(screen.getByText("Save and close"));
    expect(useTabs.getState().openIds).toEqual([]);
  });

  it("says it in the singular when only one closing tab is unsaved", () => {
    render(<TabStrip />);
    fireEvent.mouseDown(screen.getByLabelText("Tab actions"));
    fireEvent.click(screen.getByText("Close All"));

    expect(screen.getByText(/One of the 3 tabs .* has edits/)).toBeTruthy();
  });

  it("keeps the tabs open when the unsaved prompt is dismissed", () => {
    useTabs.setState({ dirtyIds: ["a", "b"] });
    render(<TabStrip />);
    fireEvent.mouseDown(screen.getByLabelText("Tab actions"));
    fireEvent.click(screen.getByText("Close All"));
    fireEvent.click(screen.getByText("Cancel"));

    expect(useTabs.getState().openIds).toEqual(["a", "b", "c"]);
    expect(screen.queryByText("Unsaved changes")).toBeNull();
  });

  it("closes one unsaved tab without a prompt, as it always did", () => {
    render(<TabStrip />);
    fireEvent.click(screen.getByLabelText("Close Second"));

    expect(screen.queryByText("Unsaved changes")).toBeNull();
    expect(useTabs.getState().openIds).toEqual(["a", "c"]);
  });

  it("closes every tab on the close-all shortcut", () => {
    useTabs.setState({ dirtyIds: [] });
    render(<TabStrip />);
    fireEvent.keyDown(window, { key: "w", metaKey: true, shiftKey: true });

    expect(useTabs.getState().openIds).toEqual([]);
  });
});
