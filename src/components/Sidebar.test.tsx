import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Tree } from "../lib/api";
import { useCollection } from "../store/collection";
import { useLayout } from "../store/layout";
import { Sidebar } from "./Sidebar";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

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
          folders: [
            { name: "Admin", path: "users/admin", folders: [], requests: [] },
          ],
          requests: [],
        },
      ],
      requests: [
        {
          id: "r1",
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

const EMPTY: Tree = { collections: [], skipped: [] };

const addRequest = vi.fn();
const duplicateRequest = vi.fn().mockResolvedValue(undefined);
const moveRequest = vi.fn().mockResolvedValue(undefined);

function setTree(tree: Tree): void {
  useCollection.setState({
    workspace: "/ws",
    tree,
    drafts: {},
    activeId: null,
    error: null,
    warning: null,
    addRequest,
    duplicateRequest,
    moveRequest,
  });
}

describe("Sidebar", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    localStorage.clear();
    addRequest.mockClear();
    duplicateRequest.mockClear();
    moveRequest.mockClear();
    useLayout.setState({ collectionsOpen: true });
    setTree(TREE);
  });

  afterEach(cleanup);

  it("duplicates a request from its row menu", () => {
    render(<Sidebar width={260} />);

    fireEvent.click(screen.getByLabelText("Actions for Health check"));
    fireEvent.click(screen.getByText("Duplicate"));

    expect(duplicateRequest).toHaveBeenCalledWith("r1");
  });

  it("moves a request to a folder chosen from the whole collection", () => {
    render(<Sidebar width={260} />);

    fireEvent.click(screen.getByLabelText("Actions for Health check"));
    fireEvent.click(screen.getByText("Move to…"));

    const select = screen.getByLabelText("Destination") as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual([
      "Acme API (current)",
      "Acme API / Users",
      "Acme API / Users / Admin",
    ]);

    fireEvent.change(select, { target: { value: "users/admin" } });
    fireEvent.click(screen.getByText("Move"));

    expect(moveRequest).toHaveBeenCalledWith("r1", "users/admin");
    expect(screen.queryByText("Move request")).toBeNull();
  });

  it("will not move a request to the folder it is already in", () => {
    render(<Sidebar width={260} />);

    fireEvent.click(screen.getByLabelText("Actions for Health check"));
    fireEvent.click(screen.getByText("Move to…"));

    expect((screen.getByText("Move") as HTMLButtonElement).disabled).toBe(true);
    expect(moveRequest).not.toHaveBeenCalled();
  });

  it("starts a request from the empty state and says where it lands", () => {
    setTree(EMPTY);
    render(<Sidebar width={260} />);

    expect(screen.getByText(/It lands in a collection called/)).toBeTruthy();
    fireEvent.click(screen.getByText("New request"));

    expect(addRequest).toHaveBeenCalledWith("http");
  });

  it("still offers naming the first collection yourself", () => {
    setTree(EMPTY);
    render(<Sidebar width={260} />);

    fireEvent.click(screen.getByText("Name it yourself instead"));
    expect(screen.getByText("New collection")).toBeTruthy();
  });
});
