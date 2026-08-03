import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { emptyRow, type KVRow } from "../lib/draft";
import { KeyValueEditor } from "./KeyValueEditor";

function Harness({ initial }: { initial?: KVRow[] }) {
  const [rows, setRows] = useState<KVRow[]>(initial ?? [emptyRow()]);
  return <KeyValueEditor rows={rows} onChange={setRows} />;
}

const keyInputs = () => screen.getAllByPlaceholderText("Key") as HTMLInputElement[];

afterEach(cleanup);

describe("KeyValueEditor", () => {
  it("auto-appends an empty row when the last row is filled", () => {
    render(<Harness />);
    expect(keyInputs()).toHaveLength(1);

    fireEvent.change(keyInputs()[0], { target: { value: "Accept" } });

    expect(keyInputs()).toHaveLength(2);
    expect(keyInputs()[0].value).toBe("Accept");
    expect(keyInputs()[1].value).toBe("");
  });

  it("toggles a row's enabled checkbox", () => {
    render(<Harness />);
    const check = screen.getAllByLabelText("Enable row")[0] as HTMLInputElement;
    expect(check.checked).toBe(true);

    fireEvent.click(check);
    expect(
      (screen.getAllByLabelText("Enable row")[0] as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("deletes a row but always keeps one empty row", () => {
    const rows = [
      { id: "1", key: "A", value: "1", enabled: true },
      { id: "2", key: "B", value: "2", enabled: true },
    ];
    render(<Harness initial={rows} />);

    fireEvent.click(screen.getAllByLabelText("Delete row")[0]);
    expect(keyInputs()).toHaveLength(1);
    expect(keyInputs()[0].value).toBe("B");

    fireEvent.click(screen.getAllByLabelText("Delete row")[0]);
    expect(keyInputs()).toHaveLength(1);
    expect(keyInputs()[0].value).toBe("");
  });

  it("round-trips through bulk edit, keeping disabled rows commented out", () => {
    render(
      <Harness
        initial={[
          { id: "1", key: "Accept", value: "application/json", enabled: true },
          { id: "2", key: "X-Debug", value: "1", enabled: false },
        ]}
      />,
    );

    fireEvent.click(screen.getByText("Bulk Edit"));
    const area = screen.getByLabelText("Key bulk edit") as HTMLTextAreaElement;
    expect(area.value).toBe("Accept:application/json\n//X-Debug:1");

    fireEvent.change(area, { target: { value: "Accept:text/plain\n//X-Debug:1" } });
    fireEvent.click(screen.getByText("Key-Value Edit"));

    expect(keyInputs().map((i) => i.value)).toEqual(["Accept", "X-Debug", ""]);
    expect(
      (screen.getAllByLabelText("Enable row")[1] as HTMLInputElement).checked,
    ).toBe(false);
  });
});
