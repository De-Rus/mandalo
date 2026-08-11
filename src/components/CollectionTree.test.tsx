import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CollectionNode } from "../lib/api";
import { CollectionTree, type TreeActions } from "./CollectionTree";

const COLLECTIONS: CollectionNode[] = [
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
        method: "DELETE",
        path: "health.toml",
      },
    ],
  },
];

function actions(): TreeActions {
  return {
    onOpen: vi.fn(),
    onRenameRequest: vi.fn(),
    onDeleteRequest: vi.fn(),
    onDuplicateRequest: vi.fn(),
    onMoveRequest: vi.fn(),
    onNewRequestIn: vi.fn(),
    onRenameCollection: vi.fn(),
    onDeleteCollection: vi.fn(),
    onExportCollection: vi.fn(),
    onNewFolder: vi.fn(),
    onRenameFolder: vi.fn(),
    onDeleteFolder: vi.fn(),
    onDropRequestInto: vi.fn(),
  };
}

describe("CollectionTree", () => {
  beforeEach(() => localStorage.clear());
  afterEach(cleanup);

  it("renders collections, folders, requests and method badges", () => {
    render(
      <CollectionTree collections={COLLECTIONS} activeId="r1" actions={actions()} />,
    );

    expect(screen.getByText("Acme API")).toBeTruthy();
    expect(screen.getByText("Users")).toBeTruthy();
    expect(screen.getByText("List users")).toBeTruthy();
    expect(screen.getByText("GET")).toBeTruthy();
    expect(screen.getByText("DELETE")).toBeTruthy();
  });

  it("collapses and expands a folder", () => {
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={actions()} />,
    );

    fireEvent.click(screen.getByLabelText("Collapse Users"));
    expect(screen.queryByText("List users")).toBeNull();

    fireEvent.click(screen.getByLabelText("Expand Users"));
    expect(screen.getByText("List users")).toBeTruthy();
  });

  it("opens a request when its row is clicked", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.click(screen.getByText("List users"));
    expect(a.onOpen).toHaveBeenCalledWith("r1");
  });

  it("renames a request inline on double click", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.doubleClick(screen.getByText("List users"));
    const input = screen.getByLabelText("Rename");
    fireEvent.change(input, { target: { value: "All users" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(a.onRenameRequest).toHaveBeenCalledWith("r1", "All users");
  });

  it("duplicates and moves a request from its row menu", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.click(screen.getByLabelText("Actions for List users"));
    fireEvent.click(screen.getByText("Duplicate"));
    expect(a.onDuplicateRequest).toHaveBeenCalledWith("r1");

    fireEvent.click(screen.getByLabelText("Actions for List users"));
    fireEvent.click(screen.getByText("Move to…"));
    expect(a.onMoveRequest).toHaveBeenCalledWith("r1");

    fireEvent.click(screen.getByLabelText("Actions for List users"));
    fireEvent.click(screen.getByText("Delete"));
    expect(a.onDeleteRequest).toHaveBeenCalledWith("r1");
  });

  it("offers folder and delete actions on a collection row", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.click(screen.getByLabelText("Actions for Acme API"));
    fireEvent.click(screen.getByText("New folder"));
    expect(a.onNewFolder).toHaveBeenCalledWith("acme", "");

    fireEvent.click(screen.getByLabelText("Actions for Acme API"));
    fireEvent.click(screen.getByText("Delete collection"));
    expect(a.onDeleteCollection).toHaveBeenCalledWith("acme");
  });

  it("right-clicking a collection offers the same actions as its ⋮ button", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.contextMenu(screen.getByText("Acme API"), {
      clientX: 40,
      clientY: 60,
    });
    fireEvent.click(screen.getByText("Export…"));
    expect(a.onExportCollection).toHaveBeenCalledWith("acme");
  });

  it("right-clicking a request row reaches its own actions", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    fireEvent.contextMenu(screen.getByText("Health check"));
    fireEvent.click(screen.getByText("Duplicate"));
    expect(a.onDuplicateRequest).toHaveBeenCalledWith("r2");
  });

  it("dragging a request onto a folder asks to move it there", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (type: string, value: string) => void data.set(type, value),
      getData: (type: string) => data.get(type) ?? "",
      get types() {
        return [...data.keys()];
      },
      effectAllowed: "",
      dropEffect: "",
    };

    fireEvent.dragStart(screen.getByText("Health check"), { dataTransfer });
    fireEvent.drop(screen.getByText("Users"), { dataTransfer });

    expect(a.onDropRequestInto).toHaveBeenCalledWith("r2", "acme", "users");
  });

  it("ignores a drop that carries no request of ours", () => {
    const a = actions();
    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={a} />,
    );

    const dataTransfer = {
      setData: () => {},
      getData: () => "",
      types: ["Files"],
      effectAllowed: "",
      dropEffect: "",
    };
    fireEvent.drop(screen.getByText("Users"), { dataTransfer });

    expect(a.onDropRequestInto).not.toHaveBeenCalled();
  });

  it("remembers collapsed rows across mounts", () => {
    const { unmount } = render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={actions()} />,
    );
    fireEvent.click(screen.getByLabelText("Collapse Users"));
    unmount();

    render(
      <CollectionTree collections={COLLECTIONS} activeId={null} actions={actions()} />,
    );
    expect(screen.queryByText("List users")).toBeNull();
  });
});
