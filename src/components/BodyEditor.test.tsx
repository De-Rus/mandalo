import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { toggleComment } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { afterEach, describe, expect, it } from "vitest";
import { newDraft, type BodyDraft } from "../lib/draft";
import { BodyEditor } from "./BodyEditor";

function Harness({ initial = "" }: { initial?: string }) {
  const [draft, setDraft] = useState<BodyDraft>({ ...newDraft(), body: initial });
  return (
    <>
      <output data-testid="body">{draft.body}</output>
      <BodyEditor
        draft={draft}
        workspaceRoot="/ws"
        onChange={(patch) => setDraft((d) => ({ ...d, ...patch }))}
      />
    </>
  );
}

const content = () => screen.getByLabelText("Raw body");

afterEach(cleanup);

describe("BodyEditor raw mode", () => {
  it("edits the body in a CodeMirror document, not a textarea", () => {
    render(<Harness initial='{"a": 1}' />);
    expect(content().classList.contains("cm-content")).toBe(true);
    expect(content().textContent).toContain('{"a": 1}');
  });

  // closeBrackets acts on real typing, which reaches CodeMirror through the
  // inputHandler facet — dispatching a change would bypass the very thing under test.
  function type(view: EditorView, char: string): void {
    const at = view.state.selection.main;
    const insert = () =>
      view.state.update({ changes: { from: at.from, to: at.to, insert: char } });
    const handled = view.state
      .facet(EditorView.inputHandler)
      .some((handler) => handler(view, at.from, at.to, char, insert));
    if (!handled) view.dispatch(insert());
  }

  it("closes a brace, a bracket and a quote as they are typed", () => {
    render(<Harness />);
    const view = EditorView.findFromDOM(content())!;
    for (const [char, expected] of [
      ["{", "{}"],
      ["[", "[]"],
      ['"', '""'],
    ]) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "" } });
      type(view, char!);
      expect(view.state.doc.toString()).toBe(expected);
    }
  });

  it("toggles a line comment with the editor shortcut", () => {
    render(<Harness initial={'{\n  "a": 1\n}'} />);
    const view = EditorView.findFromDOM(content())!;
    view.dispatch({ selection: { anchor: view.state.doc.line(2).from } });

    expect(toggleComment(view)).toBe(true);
    expect(view.state.doc.line(2).text).toContain('//');

    expect(toggleComment(view)).toBe(true);
    expect(view.state.doc.line(2).text).not.toContain('//');
  });

  it("offers every body type Mándalo can send", () => {
    render(<Harness />);
    for (const name of ["raw", "x-www-form-urlencoded", "form-data", "binary"]) {
      expect(screen.getByRole("tab", { name })).toBeTruthy();
    }
  });

  it("remounts the editor after leaving raw and coming back", () => {
    render(<Harness initial='{"a": 1}' />);
    expect(content().textContent).toContain('{"a": 1}');

    fireEvent.click(screen.getByRole("tab", { name: "form-data" }));
    expect(screen.queryByLabelText("Raw body")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "raw" }));
    expect(content().classList.contains("cm-content")).toBe(true);
    expect(content().textContent).toContain('{"a": 1}');
  });
});
