import { toggleComment } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { TEST_SNIPPETS } from "../lib/snippets";
import { ScriptEditor } from "./ScriptEditor";

function Harness({ initial = "" }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <output data-testid="value">{value}</output>
      <ScriptEditor
        label="Test script"
        kind="post"
        value={value}
        onChange={setValue}
        snippets={TEST_SNIPPETS}
        placeholder="// tests"
      />
    </>
  );
}

const value = () => screen.getByTestId("value").textContent ?? "";

afterEach(cleanup);

describe("ScriptEditor", () => {
  it("mounts a CodeMirror editor with the initial value", () => {
    render(<Harness initial="const a = 1;" />);
    const content = screen.getByLabelText("Test script");
    expect(content.classList.contains("cm-content")).toBe(true);
    expect(content.textContent).toContain("const a = 1;");
  });

  it("inserts a snippet at the cursor", () => {
    render(<Harness />);
    fireEvent.click(screen.getByText("Status code: Code is 200"));
    expect(value()).toContain("pm.response.to.have.status(200)");
  });

  it("keeps existing code intact when inserting a snippet", () => {
    render(<Harness initial="const a = 1;" />);
    fireEvent.click(screen.getByText("Response time is less than 200ms"));
    expect(value()).toContain("const a = 1;");
    expect(value()).toContain("pm.expect(pm.response.responseTime)");
  });

  it("toggles a line comment with the editor shortcut", () => {
    render(<Harness initial={'const a = 1;'} />);
    const view = EditorView.findFromDOM(screen.getByLabelText("Test script"))!;

    expect(toggleComment(view)).toBe(true);
    expect(view.state.doc.toString()).toContain("//");

    expect(toggleComment(view)).toBe(true);
    expect(view.state.doc.toString()).toBe("const a = 1;");
  });

  it("shows the placeholder when empty", () => {
    render(<Harness />);
    expect(screen.getByLabelText("Test script").textContent).toContain("// tests");
  });
});
