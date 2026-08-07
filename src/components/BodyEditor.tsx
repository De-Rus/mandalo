import { autocompletion, closeBrackets } from "@codemirror/autocomplete";
import { json, jsonLanguage, jsonParseLinter } from "@codemirror/lang-json";
import { linter, lintGutter } from "@codemirror/lint";
import { foldGutter } from "@codemirror/language";
import { prettyJson } from "../lib/format";
import { useCodeMirror } from "./CodeMirrorEditor";
import type { BodyDraft, BodyType } from "../lib/draft";
import { FormDataEditor } from "./FormDataEditor";
import { KeyValueEditor } from "./KeyValueEditor";

interface RawBodyEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  label?: string;
}

export function RawBodyEditor({
  value,
  onChange,
  placeholder,
  label = "Raw body",
}: RawBodyEditorProps) {
  const formatted = prettyJson(value);
  const canFormat = formatted !== null && formatted !== value;
  const { hostRef } = useCodeMirror({
    value,
    onChange,
    label,
    placeholder: placeholder ?? '{\n  "hello": "world"\n}',
    extensions: JSON_EXTENSIONS,
  });

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
      <div className="body-editor cm-host" ref={hostRef} />
    </>
  );
}

// The linter only complains once there is something to parse, so an empty body
// and a body still being typed stay quiet.
const jsonDiagnostics = linter((view) =>
  view.state.doc.toString().trim() === "" ? [] : jsonParseLinter()(view),
);

const JSON_EXTENSIONS = [
  json(),
  // JSON has no comment syntax, so the editor has to name one for Mod-/ to work.
  // A commented body stops being valid JSON, and the linter says so immediately.
  jsonLanguage.data.of({ commentTokens: { line: "//" } }),
  closeBrackets(),
  autocompletion(),
  jsonDiagnostics,
  lintGutter(),
  foldGutter(),
];

const BODY_TYPES: [BodyType, string][] = [
  ["formdata", "form-data"],
  ["urlencoded", "x-www-form-urlencoded"],
  ["raw", "raw"],
  ["binary", "binary"],
];

interface BodyEditorProps {
  draft: BodyDraft;
  workspaceRoot: string | null;
  onChange: (patch: Partial<BodyDraft>) => void;
  placeholder?: string;
}

export function BodyEditor({ draft, workspaceRoot, onChange, placeholder }: BodyEditorProps) {
  const { bodyType, body, formRows, formDataRows, binaryFile, binaryContentType } = draft;
  const formatted = prettyJson(body);
  const canFormat = formatted !== null && formatted !== body;
  const { hostRef: rawHostRef } = useCodeMirror({
    value: body,
    onChange: (next) => onChange({ body: next }),
    label: "Raw body",
    placeholder: placeholder ?? '{\n  "hello": "world"\n}',
    extensions: JSON_EXTENSIONS,
  });

  return (
    <>
      <div className="body-toolbar">
        <div className="body-types" role="tablist" aria-label="Body type">
          {BODY_TYPES.map(([value, name]) => (
            <button
              key={value}
              role="tab"
              aria-selected={bodyType === value}
              className={`body-type ${bodyType === value ? "body-type-active" : ""}`}
              onClick={() => onChange({ bodyType: value })}
            >
              {name}
            </button>
          ))}
        </div>
        <span className="header-spacer" />
        {bodyType === "raw" && (
          <button
            className="btn btn-sm"
            disabled={!canFormat}
            title={canFormat ? "Pretty-print the JSON body" : "Body is not JSON"}
            onClick={() => formatted !== null && onChange({ body: formatted })}
          >
            Beautify
          </button>
        )}
      </div>
      {bodyType === "raw" ? (
        <div className="body-editor cm-host" ref={rawHostRef} />
      ) : null}
      {bodyType === "urlencoded" && (
        <KeyValueEditor
          rows={formRows}
          onChange={(rows) => onChange({ formRows: rows })}
          keyPlaceholder="field"
          valuePlaceholder="value"
          keyLabel="Field"
          valueLabel="Value"
        />
      )}
      {bodyType === "formdata" && (
        <FormDataEditor
          rows={formDataRows}
          onChange={(rows) => onChange({ formDataRows: rows })}
          workspaceRoot={workspaceRoot}
        />
      )}
      {bodyType === "binary" && (
        <div className="fd-binary">
          <input
            className="kv-input"
            aria-label="Binary body file"
            placeholder="path/inside/workspace.bin"
            spellCheck={false}
            value={binaryFile}
            onChange={(e) => onChange({ binaryFile: e.target.value })}
          />
          <input
            className="kv-input fd-ct"
            aria-label="Binary content type"
            placeholder="content type (auto)"
            spellCheck={false}
            value={binaryContentType}
            onChange={(e) => onChange({ binaryContentType: e.target.value })}
          />
          <p className="fd-hint">
            The file is read at send time, workspace-relative, capped at 64&nbsp;MB.
          </p>
        </div>
      )}
    </>
  );
}
