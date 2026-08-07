import {
  autocompletion,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { javascript, javascriptLanguage } from "@codemirror/lang-javascript";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { keymap } from "@codemirror/view";
import { lintScriptSource, PM_SURFACE, type ScriptKind } from "../lib/script/lint";
import { insertAt, type Snippet } from "../lib/snippets";
import { useCodeMirror } from "./CodeMirrorEditor";

function pmCompletions(kind: ScriptKind): (context: CompletionContext) => CompletionResult | null {
  return (context) => {
    const match = context.matchBefore(/pm\.(?:\w+\.)?\w*$/);
    if (!match) return null;
    const chain = match.text.slice(3).split(".");
    const lastStart = match.to - chain[chain.length - 1]!.length;
    if (chain.length === 1) {
      const options: Completion[] = PM_SURFACE.filter(
        (entry) => !(entry.postOnly === true && kind === "pre"),
      ).map((entry) => ({ label: entry.label, type: entry.children ? "namespace" : "function", detail: entry.detail }));
      return { from: lastStart, options, validFor: /^\w*$/ };
    }
    const parent = PM_SURFACE.find((entry) => entry.label === chain[0]);
    if (!parent?.children) return null;
    return {
      from: lastStart,
      options: parent.children.map((child) => ({
        label: child.label,
        type: "method",
        detail: child.detail,
      })),
      validFor: /^\w*$/,
    };
  };
}

function pmLinter(kind: ScriptKind) {
  return linter((view): Diagnostic[] =>
    view.state.doc.toString().trim() === ""
      ? []
      : lintScriptSource(view.state.doc.toString(), kind).map((finding) => ({
          from: finding.from,
          to: finding.to,
          severity: finding.severity,
          message: finding.message,
        })),
  );
}

interface ScriptEditorProps {
  value: string;
  onChange: (value: string) => void;
  snippets: Snippet[];
  placeholder: string;
  label: string;
  kind: ScriptKind;
}

export function ScriptEditor({
  value,
  onChange,
  snippets,
  placeholder,
  label,
  kind,
}: ScriptEditorProps) {
  const { hostRef, handle } = useCodeMirror({
    value,
    onChange,
    label,
    placeholder,
    extensions: [
      javascript(),
      javascriptLanguage.data.of({ autocomplete: pmCompletions(kind) }),
      autocompletion(),
      keymap.of(completionKeymap),
      pmLinter(kind),
      lintGutter(),
    ],
  });

  const insert = (snippet: Snippet) => {
    const next = insertAt(handle.text(), handle.cursor(), snippet.code);
    handle.replace(next.text, next.cursor);
  };

  return (
    <div className="script-layout">
      <div className="script-main">
        <div className="code-editor cm-host" ref={hostRef} />
      </div>
      <div className="snippets">
        <span className="section-title">Snippets</span>
        {snippets.map((snippet) => (
          <button
            key={snippet.label}
            className="snippet-btn"
            onClick={() => insert(snippet)}
          >
            {snippet.label}
          </button>
        ))}
      </div>
    </div>
  );
}
