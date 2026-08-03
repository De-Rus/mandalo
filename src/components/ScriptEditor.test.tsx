import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { TEST_SNIPPETS } from "../lib/snippets";
import { ScriptEditor } from "./ScriptEditor";

function Harness({ initial = "" }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <ScriptEditor
      label="Test script"
      value={value}
      onChange={setValue}
      snippets={TEST_SNIPPETS}
      placeholder="// tests"
    />
  );
}

const area = () => screen.getByLabelText("Test script") as HTMLTextAreaElement;

afterEach(cleanup);

describe("ScriptEditor", () => {
  it("inserts a snippet at the cursor", () => {
    render(<Harness />);
    fireEvent.click(screen.getByText("Status code: Code is 200"));
    expect(area().value).toContain("pm.response.to.have.status(200)");
  });

  it("appends a second snippet on its own line", () => {
    render(<Harness initial="const a = 1;" />);
    area().setSelectionRange(12, 12);
    fireEvent.click(screen.getByText("Response time is less than 200ms"));
    expect(area().value.startsWith("const a = 1;\n")).toBe(true);
  });

  it("inserts two spaces for Tab instead of leaving the editor", () => {
    render(<Harness initial="ab" />);
    const textarea = area();
    textarea.setSelectionRange(1, 1);
    fireEvent.keyDown(textarea, { key: "Tab" });
    expect(area().value).toBe("a  b");
  });

  it("numbers every line in the gutter", () => {
    const { container } = render(<Harness initial={"a\nb\nc"} />);
    expect(container.querySelectorAll(".code-gutter div")).toHaveLength(3);
  });
});
