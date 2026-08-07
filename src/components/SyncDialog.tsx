import { useEffect, useMemo, useState } from "react";
import {
  errorMessage,
  planSync,
  runSync,
  setWorkspaceShare,
  workspaceShare,
  type ConflictItem,
  type PlannedFile,
  type ShareFormat,
  type SyncAction,
  type SyncPlan,
  type SyncSelection,
} from "../lib/api";
import { useCollection, flushPendingSaves } from "../store/collection";
import { useGit } from "../store/git";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { Branch, Close, Warn } from "./Icons";
import { ConflictDialog } from "./ConflictDialog";

function actionLabel(action: SyncAction): string {
  switch (action) {
    case "nothing":
      return "Nothing to sync";
    case "commit":
      return "Commit locally (no remote)";
    case "push":
      return "Push commits already on this branch";
    case "commitAndPush":
      return "Commit and push";
    case "pull":
      return "Pull remote changes";
    case "branchAndPush":
      return "Commit on a new branch and push";
  }
}

function changeLabel(change: PlannedFile["change"]): string {
  switch (change) {
    case "new":
      return "A";
    case "modified":
      return "M";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "typeChange":
      return "T";
    case "conflicted":
      return "C";
  }
}

function outcomeMessage(
  outcome: Awaited<ReturnType<typeof runSync>>,
): string {
  switch (outcome.kind) {
    case "nothingToDo":
      return "Nothing to sync";
    case "committed":
      return `Committed ${outcome.sha.slice(0, 7)} locally`;
    case "pushed":
      return `Pushed ${outcome.sha.slice(0, 7)}`;
    case "pulled":
      return `Pulled ${outcome.sha.slice(0, 7)}`;
    case "conflicted":
      return `Conflict in ${outcome.files.length} file(s)`;
    case "rejected":
      return outcome.reason;
  }
}

interface SyncDialogProps {
  onClose: () => void;
}

/**
 * plan → review → run. Share format lives in `mandalo.toml` `[share]` and is
 * toggled here — the only UI for that setting — so Sync and Export stay aligned.
 */
export function SyncDialog({ onClose }: SyncDialogProps) {
  useModalGuard();
  const workspace = useCollection((s) => s.workspace);
  const refresh = useGit((s) => s.refresh);

  const [plan, setPlan] = useState<SyncPlan | null>(null);
  const [except, setExcept] = useState<string[]>([]);
  const [message, setMessage] = useState("");
  const [force, setForce] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shareFormat, setShareFormat] = useState<ShareFormat>("native");
  const [shareReady, setShareReady] = useState(false);
  const [planTick, setPlanTick] = useState(0);
  const [conflicts, setConflicts] = useState<{
    files: string[];
    items: ConflictItem[];
  } | null>(null);

  const selection: SyncSelection | null = useMemo(
    () => (except.length === 0 ? null : { except }),
    [except],
  );

  useEffect(() => {
    if (!workspace) return;
    let live = true;
    void workspaceShare(workspace)
      .then((share) => {
        if (!live) return;
        setShareFormat(share?.format === "postman" ? "postman" : "native");
        setShareReady(true);
      })
      .catch(() => {
        if (!live) return;
        setShareReady(true);
      });
    return () => {
      live = false;
    };
  }, [workspace]);

  useEffect(() => {
    if (!workspace || !shareReady) return;
    let live = true;
    setBusy(true);
    setError(null);
    const sel: SyncSelection | null =
      except.length === 0 ? null : { except };
    void flushPendingSaves()
      .then(() => planSync(workspace, sel))
      .then((next) => {
        if (!live) return;
        setPlan(next);
        if (next.shareDir) setShareFormat("postman");
        if (next.conflicted.length > 0) {
          setConflicts({
            files: next.conflicted,
            items: next.conflictItems ?? [],
          });
        }
        setBusy(false);
      })
      .catch((e) => {
        if (!live) return;
        setError(errorMessage(e));
        setPlan(null);
        setBusy(false);
      });
    return () => {
      live = false;
    };
  }, [workspace, except, shareReady, planTick]);

  const chooseShare = async (next: ShareFormat) => {
    if (!workspace || next === shareFormat) return;
    setBusy(true);
    setError(null);
    try {
      await setWorkspaceShare(
        workspace,
        next === "postman" ? "postman" : null,
      );
      setShareFormat(next);
      setPlanTick((n) => n + 1);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  const toggle = (path: string) => {
    setExcept((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path],
    );
  };

  const sync = async () => {
    if (!workspace || !plan) return;
    if (
      (plan.action === "commit" ||
        plan.action === "commitAndPush" ||
        plan.action === "branchAndPush") &&
      message.trim() === ""
    ) {
      setError("Write a commit message before syncing.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await flushPendingSaves();
      const outcome = await runSync(
        workspace,
        plan.token,
        message.trim() || "sync",
        selection,
        force,
      );
      if (outcome.kind === "conflicted") {
        setConflicts({
          files: outcome.files,
          items: outcome.items ?? [],
        });
        setBusy(false);
        return;
      }
      const text = outcomeMessage(outcome);
      if (outcome.kind === "rejected") toast("error", text);
      else toast("success", text);
      await refresh(workspace);
      if (outcome.kind !== "rejected") onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const needsMessage =
    plan !== null &&
    (plan.action === "commit" ||
      plan.action === "commitAndPush" ||
      plan.action === "branchAndPush");

  if (conflicts && workspace) {
    return (
      <ConflictDialog
        workspace={workspace}
        files={conflicts.files}
        items={conflicts.items}
        onClose={() => setConflicts(null)}
        onResolved={() => {
          setConflicts(null);
          toast("success", "Choices kept — sync again to finish");
          setPlanTick((n) => n + 1);
          void refresh(workspace);
        }}
      />
    );
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-wide"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Sync</h2>
          <button
            className="btn-ghost btn-icon"
            aria-label="Close"
            onClick={onClose}
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body sync-dialog">
          <div className="sync-output" role="radiogroup" aria-label="Sync output">
            <button
              type="button"
              role="radio"
              aria-checked={shareFormat === "native"}
              className={`sync-output-card ${shareFormat === "native" ? "sync-output-card-on" : ""}`}
              disabled={busy}
              onClick={() => void chooseShare("native")}
            >
              <span className="sync-output-title">Mandalo</span>
              <span className="sync-output-desc">
                Commit the workspace files as they are
              </span>
            </button>
            <button
              type="button"
              role="radio"
              aria-checked={shareFormat === "postman"}
              className={`sync-output-card ${shareFormat === "postman" ? "sync-output-card-on" : ""}`}
              disabled={busy}
              onClick={() => void chooseShare("postman")}
            >
              <span className="sync-output-title">Postman</span>
              <span className="sync-output-desc">
                Also write a v2.1 mirror under{" "}
                <code>{plan?.shareDir ?? "postman"}/</code>
              </span>
            </button>
          </div>

          {!plan && !error && (
            <p className="import-status">Planning sync…</p>
          )}

          {plan && (
            <>
              <div className="sync-meta">
                <Branch size={13} />
                <span>
                  {actionLabel(plan.action)}
                  {plan.branch ? (
                    <>
                      {" · "}
                      <span className="mono">{plan.branch}</span>
                    </>
                  ) : null}
                  {plan.ahead > 0 || plan.behind > 0
                    ? ` · ↑${plan.ahead} ↓${plan.behind}`
                    : ""}
                </span>
              </div>

              {plan.conflicted.length > 0 && (
                <div className="notice notice-error notice-wrap conflict-notice">
                  <Warn size={13} />
                  <span className="notice-text">
                    {plan.conflicted.length} file
                    {plan.conflicted.length === 1 ? "" : "s"} need a side pick
                    before sync can continue.
                  </span>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={busy}
                    onClick={() =>
                      setConflicts({
                        files: plan.conflicted,
                        items: [],
                      })
                    }
                  >
                    Resolve
                  </button>
                </div>
              )}

              <div className="report-section">
                <span className="field-label">
                  Files ({plan.included} included
                  {plan.excluded > 0 ? `, ${plan.excluded} left out` : ""})
                </span>
                {plan.files.length === 0 ? (
                  <p className="empty-line">Working tree is clean.</p>
                ) : (
                  <ul className="report-list export-pick sync-files">
                    {plan.files.map((file) => (
                      <li key={file.path}>
                        <label className="export-pick-row">
                          <input
                            type="checkbox"
                            className="checkbox"
                            checked={!except.includes(file.path)}
                            disabled={file.change === "conflicted"}
                            onChange={() => toggle(file.path)}
                          />
                          <span className="sync-change mono">
                            {changeLabel(file.change)}
                          </span>
                          <span className="mono">{file.path}</span>
                        </label>
                      </li>
                    ))}
                  </ul>
                )}
              </div>

              {needsMessage && (
                <label className="field">
                  <span className="field-label">Commit message</span>
                  <input
                    className="input"
                    autoFocus
                    placeholder="What changed?"
                    value={message}
                    onChange={(e) => setMessage(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !busy) {
                        e.preventDefault();
                        void sync();
                      }
                    }}
                  />
                </label>
              )}

              {plan.findings.length > 0 && (
                <>
                  <div className="notice notice-error notice-wrap">
                    <Warn size={13} />
                    <span className="notice-text">
                      {plan.findings.length} value(s) look like credentials in
                      files that would be committed.
                    </span>
                  </div>
                  <ul className="report-list report-findings">
                    {plan.findings.map((finding, i) => (
                      <li key={i}>
                        <span className="finding-rule">{finding.rule}</span>
                        <span className="finding-path mono">
                          {finding.path}:{finding.line}
                        </span>
                        <span className="finding-excerpt mono">
                          {finding.excerpt}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <label className="import-ack">
                    <input
                      type="checkbox"
                      className="checkbox"
                      checked={force}
                      onChange={(e) => setForce(e.target.checked)}
                    />
                    <span>
                      I understand these values will leave this machine — sync
                      anyway
                    </span>
                  </label>
                </>
              )}
            </>
          )}

          {error && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error}</span>
            </div>
          )}

          <div className="modal-actions">
            <button className="btn-ghost" onClick={onClose} disabled={busy}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={
                busy ||
                !plan ||
                plan.action === "nothing" ||
                plan.conflicted.length > 0 ||
                (plan.blocked && !force) ||
                (needsMessage && message.trim() === "")
              }
              onClick={() => void sync()}
            >
              {busy ? "Working…" : "Sync"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
