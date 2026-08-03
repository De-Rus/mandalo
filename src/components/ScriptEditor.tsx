import { useRef } from "react";
import { insertAt, type Snippet } from "../lib/snippets";

interface ScriptEditorProps {
  value: string;
  onChange: (value: string) => void;
  snippets: Snippet[];
  placeholder: string;
  label: string;
}

export function ScriptEditor({
  value,
  onChange,
  snippets,
  placeholder,
  label,
}: ScriptEditorProps) {
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const lines = value === "" ? 1 : value.split("\n").length;

  const insert = (snippet: Snippet) => {
    const area = areaRef.current;
    const cursor = area ? area.selectionStart : value.length;
    const next = insertAt(value, cursor, snippet.code);
    onChange(next.text);
    requestAnimationFrame(() => {
      if (!area) return;
      area.focus();
      area.setSelectionRange(next.cursor, next.cursor);
    });
  };

  return (
    <div className="script-layout">
      <div className="script-main">
        <div className="code-editor">
          <div className="code-gutter" ref={gutterRef} aria-hidden="true">
            {Array.from({ length: lines }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
          <textarea
            ref={areaRef}
            className="code-area"
            value={value}
            aria-label={label}
            placeholder={placeholder}
            spellCheck={false}
            onScroll={(e) => {
              if (gutterRef.current)
                gutterRef.current.scrollTop = e.currentTarget.scrollTop;
            }}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Tab") return;
              e.preventDefault();
              const area = e.currentTarget;
              const start = area.selectionStart;
              const end = area.selectionEnd;
              const next = `${value.slice(0, start)}  ${value.slice(end)}`;
              onChange(next);
              requestAnimationFrame(() =>
                area.setSelectionRange(start + 2, start + 2),
              );
            }}
          />
        </div>
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
