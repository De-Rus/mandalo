import { useRef, useState } from "react";
import type { Kind } from "../lib/api";
import type { RequestDraft } from "../lib/draft";
import { applyUrl, urlWithParams } from "../lib/spec";
import {
  draftVarNames,
  splitVars,
  unresolvedVars,
  type VarDescription,
} from "../lib/vars";
import { isStreamKind, placeholderUrl } from "../lib/stream";
import { useActiveEnv } from "../store/env";
import type { Phase } from "../store/stream";
import { Code, Plug, Send, Warn } from "./Icons";
import { CurlDialog } from "./CurlDialog";
import { useVarLookup, VarToken } from "./VarTokens";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

const KINDS: [Kind, string][] = [
  ["http", "HTTP"],
  ["graphql", "GraphQL"],
  ["grpc", "gRPC"],
  ["websocket", "WebSocket"],
  ["sse", "SSE"],
  ["mqtt", "MQTT"],
];

function placeholderFor(kind: Kind): string {
  if (isStreamKind(kind)) return placeholderUrl(kind);
  if (kind === "grpc") return "http://localhost:50051";
  if (kind === "graphql") return "{{baseUrl}}/graphql";
  return "{{baseUrl}}/users?page=1";
}

/**
 * The one thing that stays on screen. A value you can look up on demand is fine
 * to hide behind a hover; a variable nothing holds a value for will fail the
 * request, and finding that out by pressing Send is worse. It covers the whole
 * draft — headers, params, body, auth — not just the tokens the URL happens to
 * draw, because those are the ones a hover can never reach.
 */
function UnresolvedWarning({ unresolved }: { unresolved: VarDescription[] }) {
  const missing = unresolved.filter((d) => d.state === "missing");
  const reason =
    missing.length === unresolved.length
      ? unresolved[0].env === null
        ? "not defined — no environment is selected"
        : `not defined in environment ${unresolved[0].env}`
      : missing.length === 0
        ? "declared, but nothing on this machine holds a value for them"
        : "not defined, or not set on this machine";
  return (
    <div className="var-warning" role="status">
      <Warn size={13} />
      <span className="var-warning-text">
        <span className="var-warning-tokens">
          {unresolved.map((description) => (
            <VarToken
              key={description.name}
              description={description}
              text={`{{${description.name}}}`}
            />
          ))}
        </span>
        {`${reason}. The request will fail until ${unresolved.length === 1 ? "it is" : "they are"} set.`}
      </span>
    </div>
  );
}

/**
 * A stream is connect → exchange → disconnect, so its primary button is a
 * latch, not a trigger. It says what pressing it will do, never what the
 * connection is currently doing — that lives in the pane below.
 */
function primaryFor(
  phase: Phase | null,
): { label: string; busy: boolean; danger: boolean } {
  switch (phase) {
    case "connected":
    case "reconnecting":
      return { label: "Disconnect", busy: false, danger: true };
    case "opening":
    case "connecting":
      return { label: "Connecting", busy: true, danger: false };
    default:
      return { label: "Connect", busy: false, danger: false };
  }
}

interface UrlBarProps {
  draft: RequestDraft;
  vars: Record<string, string>;
  sending: boolean;
  dirty: boolean;
  streamPhase: Phase | null;
  onPatch: (patch: Partial<RequestDraft>) => void;
  onSend: () => void;
  onSave: () => void;
}

export function UrlBar({
  draft,
  vars,
  sending,
  dirty,
  streamPhase,
  onPatch,
  onSend,
  onSave,
}: UrlBarProps) {
  const overlayRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [echo, setEcho] = useState<{ typed: string; composed: string } | null>(
    null,
  );
  const [curlOpen, setCurlOpen] = useState(false);
  const composed = urlWithParams(draft.url, draft.params);
  /**
   * WHY: the displayed string is derived from url + params, so a keystroke the
   * round-trip normalises ("?" before its key, a value-less pair) would be
   * rewritten under the caret. Keeping the literal text while the draft still
   * composes to what that text produced lets typing survive; any edit from the
   * Params table changes `composed` and drops the echo.
   */
  const shown = echo !== null && echo.composed === composed ? echo.typed : composed;
  const segments = splitVars(shown, vars);
  const lookup = useVarLookup(vars);
  const env = useActiveEnv();
  const unresolved = unresolvedVars(draftVarNames(draft), env, vars);

  const type = (text: string) => {
    const patch = applyUrl(text, draft.params);
    setEcho({ typed: text, composed: urlWithParams(patch.url, patch.params) });
    onPatch(patch);
  };

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
        <div
          className={`url-composer ${
            draft.kind === "http" ? "" : "url-composer-plain"
          }`}
        >
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
              className="input mono url-input"
              value={shown}
              aria-label="URL"
              placeholder={placeholderFor(draft.kind)}
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => type(e.target.value)}
              onScroll={(e) => {
                if (overlayRef.current)
                  overlayRef.current.scrollLeft = e.currentTarget.scrollLeft;
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") onSend();
              }}
            />
            <div className="url-overlay" ref={overlayRef}>
              {shown === "" ? (
                <span className="url-placeholder" aria-hidden="true">
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
                        return (
                          <span key={i} aria-hidden="true">
                            {segment.text}
                          </span>
                        );
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
        </div>
        {isStreamKind(draft.kind) ? (
          (() => {
            const primary = primaryFor(streamPhase);
            return (
              <button
                className={`btn btn-primary btn-send ${primary.danger ? "btn-danger" : ""}`}
                disabled={primary.busy || draft.url.trim() === ""}
                title={`${primary.label} (⌘⏎)`}
                onClick={onSend}
              >
                {primary.busy ? <span className="spinner" /> : <Plug size={13} />}
                {primary.label}
              </button>
            );
          })()
        ) : (
          <button
            className="btn btn-primary btn-send"
            disabled={sending || draft.url.trim() === ""}
            title="Send (⌘⏎)"
            onClick={onSend}
          >
            {sending ? <span className="spinner" /> : <Send size={13} />}
            {sending ? "Sending" : "Send"}
          </button>
        )}
        <button
          className="btn btn-send"
          title="Save (⌘S)"
          disabled={!dirty}
          onClick={onSave}
        >
          Save
        </button>
        <button
          className="btn-ghost btn-icon btn-send"
          title={
            draft.kind === "grpc" || isStreamKind(draft.kind)
              ? `A ${draft.kind} request cannot be written as curl`
              : "Generate curl"
          }
          disabled={draft.kind === "grpc" || isStreamKind(draft.kind)}
          aria-label="Generate curl"
          onClick={() => setCurlOpen(true)}
        >
          <Code size={14} />
        </button>
      </div>
      {unresolved.length > 0 && <UnresolvedWarning unresolved={unresolved} />}
      {curlOpen && (
        <CurlDialog draft={draft} onClose={() => setCurlOpen(false)} />
      )}
    </>
  );
}
