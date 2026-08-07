import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, placeholder as cmPlaceholder } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useCallback, useEffect, useRef, useState } from "react";

export const syntaxColours = HighlightStyle.define([
  { tag: [tags.keyword, tags.operatorKeyword], color: "var(--tk-bool)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--tk-string)" },
  { tag: tags.number, color: "var(--tk-number)" },
  { tag: [tags.bool, tags.null], color: "var(--tk-bool)" },
  { tag: tags.comment, color: "var(--text-muted)", fontStyle: "italic" },
  {
    tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
    color: "var(--tk-key)",
  },
  { tag: tags.propertyName, color: "var(--tk-key)" },
  { tag: [tags.punctuation, tags.bracket], color: "var(--tk-punct)" },
]);

export interface CodeMirrorEditorProps {
  value: string;
  onChange: (value: string) => void;
  label: string;
  placeholder: string;
  /** Language, lint and completion extensions — everything past the shared base. */
  extensions: Extension[];
  className?: string;
}

export interface CodeMirrorHandle {
  /** Replaces the whole document and puts the caret at `cursor`. */
  replace: (text: string, cursor: number) => void;
  cursor: () => number;
  text: () => string;
}

export function useCodeMirror({
  value,
  onChange,
  label,
  placeholder,
  extensions,
}: CodeMirrorEditorProps): {
  hostRef: (node: HTMLDivElement | null) => void;
  handle: CodeMirrorHandle;
} {
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  const hostRef = useCallback((node: HTMLDivElement | null) => {
    setHost(node);
  }, []);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const valueRef = useRef(value);
  valueRef.current = value;
  const extensionsRef = useRef(extensions);
  extensionsRef.current = extensions;

  useEffect(() => {
    if (!host) return;
    const view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: valueRef.current,
        extensions: [
          lineNumbers(),
          history(),
          closeBrackets(),
          syntaxHighlighting(syntaxColours),
          cmPlaceholder(placeholder),
          EditorView.contentAttributes.of({ "aria-label": label }),
          EditorView.theme({}, { dark: true }),
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          }),
          ...extensionsRef.current,
        ],
      }),
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, [host, label, placeholder]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === value) return;
    view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
  }, [value]);

  return {
    hostRef,
    handle: {
      replace: (text, cursor) => {
        const view = viewRef.current;
        if (!view) return;
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: text },
          selection: { anchor: cursor },
        });
        view.focus();
      },
      cursor: () => viewRef.current?.state.selection.main.head ?? 0,
      text: () => viewRef.current?.state.doc.toString() ?? "",
    },
  };
}
