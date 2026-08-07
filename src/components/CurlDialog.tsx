import { useEffect, useState } from "react";
import { errorMessage, toCurl } from "../lib/api";
import { toSaved } from "../lib/collection";
import type { RequestDraft } from "../lib/draft";
import { isStreamKind } from "../lib/stream";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { Close, Copy } from "./Icons";

interface CurlDialogProps {
  draft: RequestDraft;
  onClose: () => void;
}

/**
 * curl only — the product choice is one pasteable shell line, not thirty
 * language templates. Variables resolve the same way Send does; unresolved
 * ones fail here instead of landing in somebody's clipboard.
 */
export function CurlDialog({ draft, onClose }: CurlDialogProps) {
  useModalGuard();
  const workspace = useCollection((s) => s.workspace);
  const env = useEnv((s) => s.selected);
  const [command, setCommand] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);

  useEffect(() => {
    if (!workspace) {
      setBusy(false);
      setError("Open a workspace first.");
      return;
    }
    if (draft.kind === "grpc" || isStreamKind(draft.kind)) {
      setBusy(false);
      setError(`A ${draft.kind} request cannot be written as a curl command.`);
      return;
    }
    let cancelled = false;
    setBusy(true);
    void toCurl(workspace, toSaved(draft), env)
      .then((text) => {
        if (!cancelled) {
          setCommand(text);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setCommand(null);
          setError(errorMessage(e));
        }
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [workspace, draft, env]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal curl-dialog"
        style={{ width: 560 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Generate curl</h2>
          <button
            className="btn-ghost btn-icon"
            aria-label="Close"
            onClick={onClose}
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {busy && (
            <div className="skeleton-stack">
              <div className="skeleton" style={{ width: "80%" }} />
              <div className="skeleton" style={{ width: "60%" }} />
            </div>
          )}
          {!busy && error && (
            <p className="empty-line" role="alert">
              {error}
            </p>
          )}
          {!busy && command !== null && (
            <pre className="curl-command mono" tabIndex={0}>
              {command}
            </pre>
          )}
          <div className="modal-actions">
            <button className="btn" onClick={onClose}>
              Close
            </button>
            <button
              className="btn btn-primary"
              disabled={command === null}
              onClick={() => {
                if (command === null) return;
                void navigator.clipboard.writeText(command).then(
                  () => toast("success", "curl command copied"),
                  (e) => toast("error", errorMessage(e)),
                );
              }}
            >
              <Copy size={13} />
              Copy
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
