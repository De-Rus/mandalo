import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { emptyFormDataRow, type FormDataRowDraft } from "../lib/draft";
import { FormDataEditor } from "./FormDataEditor";

const open = vi.fn();

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...args: unknown[]) => open(...args) }));

function Harness({ initial }: { initial?: FormDataRowDraft[] }) {
  const [rows, setRows] = useState<FormDataRowDraft[]>(initial ?? [emptyFormDataRow()]);
  return <FormDataEditor rows={rows} onChange={setRows} workspaceRoot="/ws" />;
}

afterEach(() => {
  cleanup();
  open.mockReset();
});

describe("FormDataEditor", () => {
  it("appends a fresh row once the last row is used", () => {
    render(<Harness />);
    fireEvent.change(screen.getAllByLabelText("Field name")[0]!, {
      target: { value: "title" },
    });
    expect(screen.getAllByLabelText("Field name")).toHaveLength(2);
  });

  it("adds several files under one key with the Add files button", async () => {
    open.mockResolvedValueOnce(["/ws/files/a.png", "/ws/files/b.png"]);
    render(<Harness />);
    fireEvent.change(screen.getAllByLabelText("Field type")[0]!, {
      target: { value: "file" },
    });
    fireEvent.change(screen.getAllByLabelText("Field name")[0]!, {
      target: { value: "photos" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add files…" }));
    const files = await screen.findByLabelText("Field files");
    expect(files.textContent).toContain("a.png");
    expect(files.textContent).toContain("b.png");
    expect(
      screen
        .getAllByLabelText("Field name")
        .filter((el) => (el as HTMLInputElement).value === "photos"),
    ).toHaveLength(1);
  });

  it("opens a modal from the file chips to remove one", () => {
    render(
      <Harness
        initial={[
          {
            id: "1",
            key: "photos",
            kind: "file",
            value: "",
            files: ["files/a.png", "files/b.png", "files/c.png"],
            contentType: "",
            enabled: true,
          },
          emptyFormDataRow(),
        ]}
      />,
    );
    const preview = screen.getByLabelText("Field files");
    expect(preview.textContent).toContain("a.png");
    expect(preview.textContent).toContain("b.png");
    expect(preview.textContent).toContain("+1");
    expect(preview.textContent).not.toContain("c.png");
    fireEvent.click(preview);
    expect(screen.getByRole("dialog", { name: "photos files" })).toBeTruthy();
    expect(screen.getByRole("dialog").textContent).toContain("files/c.png");
    fireEvent.click(screen.getByLabelText("Remove files/a.png"));
    expect(screen.getByRole("dialog").textContent).not.toContain("files/a.png");
    expect(screen.getByRole("dialog").textContent).toContain("files/b.png");
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByLabelText("Field files").textContent).toContain("b.png");
    expect(screen.getByLabelText("Field files").textContent).not.toContain("a.png");
  });

  it("deletes a row but always keeps one editable row", () => {
    render(<Harness />);
    fireEvent.click(screen.getAllByLabelText("Delete field")[0]!);
    expect(screen.getAllByLabelText("Field name")).toHaveLength(1);
  });
});
