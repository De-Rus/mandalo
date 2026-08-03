import { prettyJson } from "../lib/format";

interface BodyEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  label?: string;
}

export function BodyEditor({
  value,
  onChange,
  placeholder,
  label = "Raw body",
}: BodyEditorProps) {
  const formatted = prettyJson(value);
  const canFormat = formatted !== null && formatted !== value;

  return (
    <>
      <div className="body-toolbar">
        <span className="section-title">{label}</span>
        <span className="header-spacer" />
        <button
          className="btn btn-sm"
          disabled={!canFormat}
          title={canFormat ? "Pretty-print the JSON body" : "Body is not JSON"}
          onClick={() => formatted !== null && onChange(formatted)}
        >
          Beautify
        </button>
      </div>
      <div className="body-editor">
        <textarea
          className="textarea mono fill"
          value={value}
          placeholder={placeholder ?? '{\n  "hello": "world"\n}'}
          spellCheck={false}
          onChange={(e) => onChange(e.target.value)}
        />
      </div>
    </>
  );
}
