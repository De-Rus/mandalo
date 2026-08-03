import { prettyJson } from "../lib/format";

interface BodyEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function BodyEditor({ value, onChange, placeholder }: BodyEditorProps) {
  const formatted = prettyJson(value);

  return (
    <div className="body-editor">
      <textarea
        className="textarea mono"
        value={value}
        placeholder={placeholder ?? '{\n  "hello": "world"\n}'}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
      />
      {formatted !== null && formatted !== value && (
        <button className="btn-ghost format-btn" onClick={() => onChange(formatted)}>
          Format JSON
        </button>
      )}
    </div>
  );
}
