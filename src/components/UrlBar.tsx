import { useRef } from "react";
import type { Kind } from "../lib/api";
import type { RequestDraft } from "../lib/draft";
import { draftVarNames, splitVars } from "../lib/vars";
import { Send } from "./Icons";
import { useVarLookup, VarPill, VarToken } from "./VarTokens";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

const KINDS: [Kind, string][] = [
  ["http", "HTTP"],
  ["graphql", "GraphQL"],
  ["grpc", "gRPC"],
];

function placeholderFor(kind: Kind): string {
  if (kind === "grpc") return "http://localhost:50051";
  if (kind === "graphql") return "{{baseUrl}}/graphql";
  return "{{baseUrl}}/users?page=1";
}

interface UrlBarProps {
  draft: RequestDraft;
  vars: Record<string, string>;
  sending: boolean;
  dirty: boolean;
  onPatch: (patch: Partial<RequestDraft>) => void;
  onSend: () => void;
  onSave: () => void;
}

export function UrlBar({
  draft,
  vars,
  sending,
  dirty,
  onPatch,
  onSend,
  onSave,
}: UrlBarProps) {
  const overlayRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const segments = splitVars(draft.url, vars);
  const lookup = useVarLookup(vars);
  const used = draftVarNames(draft);

  const caretTo = (index: number) => (e: React.MouseEvent) => {
    e.preventDefault();
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    input.setSelectionRange(index, index);
  };

  return (
    <>
      <div className="url-bar">
        <select
          className="select kind-select"
          value={draft.kind}
          aria-label="Request kind"
          onChange={(e) => onPatch({ kind: e.target.value as Kind })}
        >
          {KINDS.map(([kind, label]) => (
            <option key={kind} value={kind}>
              {label}
            </option>
          ))}
        </select>
        {draft.kind === "http" && (
          <select
            className={`select method-select m-${draft.method.toLowerCase()}`}
            value={draft.method}
            aria-label="Method"
            onChange={(e) => onPatch({ method: e.target.value })}
          >
            {METHODS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        )}
        <div className="url-field">
          <input
            ref={inputRef}
            className={`input mono url-input ${
              draft.kind === "http" ? "" : "url-input-plain"
            }`}
            value={draft.url}
            aria-label="URL"
            placeholder={placeholderFor(draft.kind)}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => onPatch({ url: e.target.value })}
            onScroll={(e) => {
              if (overlayRef.current)
                overlayRef.current.scrollLeft = e.currentTarget.scrollLeft;
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") onSend();
            }}
          />
          <div className="url-overlay" ref={overlayRef} aria-hidden="true">
            {draft.url === "" ? (
              <span className="url-placeholder">
                {placeholderFor(draft.kind)}
              </span>
            ) : (
              <span className="url-overlay-inner">
                {(() => {
                  let at = 0;
                  return segments.map((segment, i) => {
                    const end = at + segment.text.length;
                    at = end;
                    if (segment.name === null)
                      return <span key={i}>{segment.text}</span>;
                    return (
                      <VarToken
                        key={i}
                        description={lookup(segment.name)}
                        text={segment.text}
                        onMouseDown={caretTo(end)}
                      />
                    );
                  });
                })()}
              </span>
            )}
          </div>
        </div>
        <button
          className="btn btn-primary btn-send"
          disabled={sending || draft.url.trim() === ""}
          title="Send (⌘⏎)"
          onClick={onSend}
        >
          {sending ? <span className="spinner" /> : <Send size={13} />}
          {sending ? "Sending" : "Send"}
        </button>
        <button
          className="btn btn-send"
          title="Save (⌘S)"
          disabled={!dirty}
          onClick={onSave}
        >
          Save
        </button>
      </div>
      {used.length > 0 && (
        <div className="var-strip">
          {used.map((name) => (
            <VarPill key={name} description={lookup(name)} />
          ))}
        </div>
      )}
    </>
  );
}
