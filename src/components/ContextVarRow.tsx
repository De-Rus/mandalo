import { useEffect, useRef, useState } from "react";
import { errorMessage, type VarInfo } from "../lib/api";
import { varLabel } from "../store/env";
import { Close } from "./Icons";

interface ScopeChip {
  text: string;
  cls: string;
}

function scopeChip(info: VarInfo): ScopeChip {
  if (info.shared) return { text: "File", cls: "ctx-scope-file" };
  if (info.source === "environment")
    return { text: "Process env", cls: "ctx-scope-env" };
  return { text: "This machine", cls: "ctx-scope-local" };
}

interface Props {
  envName: string;
  name: string;
  info: VarInfo;
  commit: (key: string, value: string) => Promise<void>;
  storeSecret: (env: string, key: string, value: string) => Promise<void>;
  forgetSecret: (env: string, key: string) => Promise<void>;
  removeVar: (env: string, key: string) => Promise<void>;
  onEditInModal: () => void;
}

export function ContextVarRow({
  envName,
  name,
  info,
  commit,
  storeSecret,
  forgetSecret,
  removeVar,
  onEditInModal,
}: Props) {
  const [text, setText] = useState(info.value ?? "");
  const [entry, setEntry] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setText(info.value ?? "");
  }, [info.value]);

  const run = async (task: Promise<unknown>) => {
    setError(null);
    try {
      await task;
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    }
  };

  const commitText = () => {
    if (text === (info.value ?? "")) return;
    void run(commit(name, text));
  };

  const chip = scopeChip(info);
  const editable = !info.secret && info.shared;
  const held = varLabel(info);

  return (
    <div className="ctx-var-wrap">
      <div className="ctx-var">
        <span className="ctx-var-name mono" title={name}>
          {name}
          {info.secret && <span className="badge badge-secret">Secret</span>}
        </span>
        <span className="ctx-var-value">
          {editable ? (
            <input
              ref={input}
              className="ctx-value-input mono"
              aria-label={`Value for ${name}`}
              title={text}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onBlur={commitText}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitText();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setText(info.value ?? "");
                  input.current?.blur();
                }
              }}
            />
          ) : info.secret ? (
            info.set ? (
              <span className="ctx-value-secret mono">{held}</span>
            ) : (
              <span className="ctx-value-unset">{held}</span>
            )
          ) : (
            <span className="ctx-value-unset">
              {info.set ? "stored on this machine" : "not set on this machine"}
            </span>
          )}
        </span>
        <span className="ctx-var-meta">
          <span className={`ctx-scope ${chip.cls}`}>{chip.text}</span>
          <button
            className="btn-ghost btn-icon btn-icon-sm"
            aria-label={`Delete ${name}`}
            onClick={() => void run(removeVar(envName, name))}
          >
            <Close size={11} />
          </button>
        </span>
      </div>
      {!editable && (
        <div className="ctx-var-tools">
          <button
            className="btn-ghost btn-sm"
            onClick={() => setEntry(entry === null ? "" : null)}
          >
            Set value
          </button>
          {info.secret && (
            <button
              className="btn-ghost btn-sm"
              disabled={!info.set}
              onClick={() => void run(forgetSecret(envName, name))}
            >
              Clear
            </button>
          )}
          <button className="btn-ghost btn-sm" onClick={onEditInModal}>
            Edit in environment editor
          </button>
        </div>
      )}
      {entry !== null && (
        <div className="ctx-var-entry">
          <input
            className="input mono"
            type={info.secret ? "password" : "text"}
            autoFocus
            aria-label={`New value for ${name}`}
            placeholder={
              info.secret
                ? "value — kept on this machine, never in the file"
                : "value"
            }
            value={entry}
            onChange={(e) => setEntry(e.target.value)}
          />
          <button
            className="btn btn-primary btn-sm"
            disabled={entry === ""}
            onClick={() => {
              const task = info.secret
                ? storeSecret(envName, name, entry)
                : commit(name, entry);
              void run(task).then((ok) => {
                if (ok) setEntry(null);
              });
            }}
          >
            Store
          </button>
          <button className="btn-ghost btn-sm" onClick={() => setEntry(null)}>
            Cancel
          </button>
        </div>
      )}
      {(info.hosts ?? []).length > 0 && (
        <div className="ctx-var-hosts">
          <span className="ctx-var-hosts-label">Sent only to</span>
          {(info.hosts ?? []).map((h) => (
            <span className="var-pill" key={h}>
              {h}
            </span>
          ))}
        </div>
      )}
      {error && <p className="inline-error">{error}</p>}
    </div>
  );
}
