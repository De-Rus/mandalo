import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useToasts } from "../store/toast";
import { useTransfer } from "../store/transfer";
import { DropTarget } from "./DropTarget";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const SPEC = '{"openapi":"3.1.0","paths":{}}';

function fileTransfer(file: File | null) {
  return {
    types: file ? ["Files"] : ["text/plain"],
    files: { item: () => file, length: file ? 1 : 0 },
  };
}

describe("DropTarget", () => {
  beforeEach(() => {
    invoke.mockReset();
    useTransfer.setState({ importOpen: false, dropped: null, dropSeq: 0 });
    useToasts.setState({ items: [] });
  });

  afterEach(cleanup);

  it("shows a drop target while a file is over the window", () => {
    render(<DropTarget />);
    expect(screen.queryByText("Drop to import")).toBeNull();

    fireEvent.dragEnter(window, { dataTransfer: fileTransfer(new File([""], "a.json")) });
    expect(screen.getByText("Drop to import")).toBeTruthy();

    fireEvent.dragLeave(window, { dataTransfer: fileTransfer(new File([""], "a.json")) });
    expect(screen.queryByText("Drop to import")).toBeNull();
  });

  it("ignores a drag that carries no file", () => {
    render(<DropTarget />);
    fireEvent.dragEnter(window, { dataTransfer: fileTransfer(null) });
    expect(screen.queryByText("Drop to import")).toBeNull();
  });

  it("opens the import dialog with the dropped document", async () => {
    render(<DropTarget />);
    const file = new File([SPEC], "petstore.json", { type: "application/json" });

    fireEvent.drop(window, { dataTransfer: fileTransfer(file) });

    await waitFor(() => expect(useTransfer.getState().importOpen).toBe(true));
    expect(useTransfer.getState().dropped).toMatchObject({
      name: "petstore.json",
      text: SPEC,
      origin: "file",
    });
    expect(screen.queryByText("Drop to import")).toBeNull();
  });

  it("reports a file it cannot read instead of opening an empty dialog", async () => {
    render(<DropTarget />);
    const file = new File([""], "huge.json");
    Object.defineProperty(file, "size", { value: 64 * 1024 * 1024 + 1 });

    fireEvent.drop(window, { dataTransfer: fileTransfer(file) });

    await waitFor(() =>
      expect(useToasts.getState().items.map((t) => t.text).join(" ")).toMatch(
        /over the .* byte limit/,
      ),
    );
    expect(useTransfer.getState().importOpen).toBe(false);
  });
});
