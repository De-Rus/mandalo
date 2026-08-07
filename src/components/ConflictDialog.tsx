import { useEffect, useMemo, useState } from "react";
import {
  applyConflictChoices,
  conflictPreviews,
  errorMessage,
  type ConflictChoice,
  type ConflictItem,
  type Kind,
} from "../lib/api";
import {
  buildRequestMerge,
  defaultPick,
  isRequestConflictPath,
  matchRequestUnits,
  type MatchedUnit,
  type UnitPick,
} from "../lib/conflictUnits";
import { lineDiff, type DiffLine } from "../lib/diff";
import { useModalGuard } from "../store/ui";
import { Close, Warn } from "./Icons";
import { MethodBadge } from "./MethodBadge";

function asKind(_method: string | null): Kind {
  return "http";
}

function DiffPane({
  label,
  lines,
  side,
  selected,
  onPick,
}: {
  label: string;
  lines: DiffLine[];
  side: "ours" | "theirs";
  selected: boolean;
  onPick: () => void;
}) {
  return (
    <button
      type="button"
      className={`conflict-diff-pane ${selected ? "conflict-diff-pane-on" : ""}`}
      aria-pressed={selected}
      onClick={onPick}
    >
      <span className="conflict-side-label">{label}</span>
      <pre className="conflict-diff mono" aria-label={`${label} diff`}>
        {lines.length === 0 ? (
          <span className="conflict-gone">Removed</span>
        ) : (
          lines.map((line, i) => {
            const show =
              side === "ours" ? line.op !== "insert" : line.op !== "delete";
            if (!show) {
              return (
                <span key={i} className="conflict-diff-gap">
                  {"\n"}
                </span>
              );
            }
            const cls =
              line.op === "equal"
                ? "conflict-diff-eq"
                : line.op === "delete"
                  ? "conflict-diff-del"
                  : "conflict-diff-ins";
            const mark =
              line.op === "equal" ? " " : line.op === "delete" ? "-" : "+";
            return (
              <span key={i} className={cls}>
                {mark}
                {line.text}
                {"\n"}
              </span>
            );
          })
        )}
      </pre>
    </button>
  );
}

function RequestCard({
  label,
  unit,
  selected,
  onPick,
}: {
  label: string;
  unit: { name: string; method: string | null; detail: string | null; text: string };
  selected: boolean;
  onPick: () => void;
}) {
  return (
    <button
      type="button"
      className={`conflict-req-card ${selected ? "conflict-req-card-on" : ""}`}
      aria-pressed={selected}
      onClick={onPick}
    >
      <span className="conflict-side-label">{label}</span>
      <span className="conflict-req-head">
        {unit.method ? (
          <MethodBadge
            item={{ kind: asKind(unit.method), method: unit.method }}
          />
        ) : (
          <span className="conflict-chip">REQ</span>
        )}
        <span className="conflict-name">{unit.name}</span>
      </span>
      {unit.detail && (
        <span className="conflict-detail mono">{unit.detail}</span>
      )}
      <pre className="conflict-req-body mono">{unit.text}</pre>
    </button>
  );
}

function RequestUnitRow({
  unit,
  pick,
  onPick,
}: {
  unit: MatchedUnit;
  pick: UnitPick;
  onPick: (pick: UnitPick) => void;
}) {
  if (unit.kind === "same" && unit.ours) {
    return (
      <div className="conflict-unit conflict-unit-same">
        <span className="conflict-unit-tag">Same on both sides</span>
        <RequestCard
          label="Kept"
          unit={unit.ours}
          selected
          onPick={() => onPick("ours")}
        />
      </div>
    );
  }

  if (unit.kind === "oursOnly" && unit.ours) {
    return (
      <div className="conflict-unit">
        <span className="conflict-unit-tag">Only yours</span>
        <div className="conflict-unit-actions">
          <button
            type="button"
            className={`conflict-pick ${pick === "ours" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("ours")}
          >
            Add
          </button>
          <button
            type="button"
            className={`conflict-pick ${pick === "skip" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("skip")}
          >
            Skip
          </button>
        </div>
        <RequestCard
          label="Yours"
          unit={unit.ours}
          selected={pick === "ours"}
          onPick={() => onPick("ours")}
        />
      </div>
    );
  }

  if (unit.kind === "theirsOnly" && unit.theirs) {
    return (
      <div className="conflict-unit">
        <span className="conflict-unit-tag">Only remote</span>
        <div className="conflict-unit-actions">
          <button
            type="button"
            className={`conflict-pick ${pick === "theirs" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("theirs")}
          >
            Add
          </button>
          <button
            type="button"
            className={`conflict-pick ${pick === "skip" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("skip")}
          >
            Skip
          </button>
        </div>
        <RequestCard
          label="Theirs"
          unit={unit.theirs}
          selected={pick === "theirs"}
          onPick={() => onPick("theirs")}
        />
      </div>
    );
  }

  if (unit.ours && unit.theirs) {
    const lines = lineDiff(unit.ours.text, unit.theirs.text);
    return (
      <div className="conflict-unit">
        <span className="conflict-unit-tag">Changed — {unit.name}</span>
        <div className="conflict-unit-actions" role="group" aria-label="Keep">
          <button
            type="button"
            className={`conflict-pick ${pick === "ours" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("ours")}
          >
            Yours
          </button>
          <button
            type="button"
            className={`conflict-pick ${pick === "theirs" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("theirs")}
          >
            Theirs
          </button>
          <button
            type="button"
            className={`conflict-pick ${pick === "both" ? "conflict-pick-on" : ""}`}
            onClick={() => onPick("both")}
          >
            Both
          </button>
        </div>
        <div className="conflict-req-meta">
          {unit.ours.method && (
            <MethodBadge
              item={{ kind: asKind(unit.ours.method), method: unit.ours.method }}
            />
          )}
          {unit.ours.detail && (
            <span className="conflict-detail mono">{unit.ours.detail}</span>
          )}
          {unit.theirs.detail && unit.theirs.detail !== unit.ours.detail && (
            <span className="conflict-detail mono">
              → {unit.theirs.detail}
            </span>
          )}
        </div>
        <div className="conflict-pair">
          <DiffPane
            label="Yours"
            side="ours"
            lines={lines}
            selected={pick === "ours" || pick === "both"}
            onPick={() => onPick("ours")}
          />
          <DiffPane
            label="Theirs"
            side="theirs"
            lines={lines}
            selected={pick === "theirs" || pick === "both"}
            onPick={() => onPick("theirs")}
          />
        </div>
      </div>
    );
  }

  return null;
}

function ConfigFileRow({
  item,
  choice,
  onPick,
}: {
  item: ConflictItem;
  choice: ConflictChoice | undefined;
  onPick: (c: ConflictChoice) => void;
}) {
  const lines = useMemo(
    () => lineDiff(item.ours.text ?? "", item.theirs.text ?? ""),
    [item],
  );
  return (
    <li className="conflict-row">
      <span className="conflict-path mono">{item.path}</span>
      <p className="conflict-file-hint">Config — pick one whole file</p>
      <div className="conflict-pair">
        <DiffPane
          label="Yours"
          side="ours"
          lines={lines}
          selected={choice === "ours"}
          onPick={() => onPick("ours")}
        />
        <DiffPane
          label="Theirs"
          side="theirs"
          lines={lines}
          selected={choice === "theirs"}
          onPick={() => onPick("theirs")}
        />
      </div>
    </li>
  );
}

function RequestFileRow({
  item,
  picks,
  onPickUnit,
}: {
  item: ConflictItem;
  picks: Record<string, UnitPick>;
  onPickUnit: (unitId: string, pick: UnitPick) => void;
}) {
  const units = useMemo(
    () => matchRequestUnits(item.ours.text ?? "", item.theirs.text ?? ""),
    [item],
  );
  return (
    <li className="conflict-row">
      <span className="conflict-path mono">{item.path}</span>
      <p className="conflict-file-hint">
        Requests — keep yours, theirs, or add both
      </p>
      <div className="conflict-units">
        {units.map((unit) => (
          <RequestUnitRow
            key={unit.id}
            unit={unit}
            pick={picks[unit.id] ?? defaultPick(unit.kind)}
            onPick={(p) => onPickUnit(unit.id, p)}
          />
        ))}
      </div>
    </li>
  );
}

interface ConflictDialogProps {
  workspace: string;
  files: string[];
  items?: ConflictItem[];
  onResolved: () => void;
  onClose: () => void;
}

/**
 * Mandalo diffs local vs remote itself — requests one-by-one (yours / theirs /
 * both), config as a line diff. Not a git pull/merge UI.
 */
export function ConflictDialog({
  workspace,
  files,
  items: seeded,
  onResolved,
  onClose,
}: ConflictDialogProps) {
  useModalGuard();
  const [items, setItems] = useState<ConflictItem[]>(seeded ?? []);
  const [fileChoices, setFileChoices] = useState<Record<string, ConflictChoice>>(
    {},
  );
  const [unitPicks, setUnitPicks] = useState<
    Record<string, Record<string, UnitPick>>
  >({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (seeded && seeded.length > 0) {
      setItems(seeded);
      return;
    }
    let live = true;
    setBusy(true);
    void conflictPreviews(workspace, files)
      .then((next) => {
        if (!live) return;
        setItems(next);
        setBusy(false);
      })
      .catch((e) => {
        if (!live) return;
        setError(errorMessage(e));
        setBusy(false);
      });
    return () => {
      live = false;
    };
  }, [workspace, files, seeded]);

  const unitsByPath = useMemo(() => {
    const map = new Map<string, MatchedUnit[]>();
    for (const item of items) {
      if (isRequestConflictPath(item.path)) {
        map.set(
          item.path,
          matchRequestUnits(item.ours.text ?? "", item.theirs.text ?? ""),
        );
      }
    }
    return map;
  }, [items]);

  const ready = items.every((item) => {
    if (isRequestConflictPath(item.path)) {
      const units = unitsByPath.get(item.path) ?? [];
      return units.length > 0;
    }
    return fileChoices[item.path] != null;
  });

  const apply = async () => {
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      await applyConflictChoices(
        workspace,
        items.map((item) => {
          if (isRequestConflictPath(item.path)) {
            const units = unitsByPath.get(item.path) ?? [];
            const picks = unitPicks[item.path] ?? {};
            const content = buildRequestMerge(units, picks);
            return {
              path: item.path,
              choice: "ours" as const,
              content,
            };
          }
          return {
            path: item.path,
            choice: fileChoices[item.path]!,
          };
        }),
      );
      onResolved();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal-wide conflict-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Resolve</h2>
          <button
            className="btn-ghost btn-icon"
            aria-label="Close"
            onClick={onClose}
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body conflict-body">
          <p className="conflict-hint">
            Mandalo compared each request (and config) to the remote. Keep
            yours, theirs, or add both — then Sync.
          </p>

          {busy && items.length === 0 && (
            <p className="import-status">Loading diffs…</p>
          )}

          <ul className="conflict-list">
            {items.map((item) =>
              isRequestConflictPath(item.path) ? (
                <RequestFileRow
                  key={item.path}
                  item={item}
                  picks={unitPicks[item.path] ?? {}}
                  onPickUnit={(unitId, pick) =>
                    setUnitPicks((prev) => ({
                      ...prev,
                      [item.path]: { ...prev[item.path], [unitId]: pick },
                    }))
                  }
                />
              ) : (
                <ConfigFileRow
                  key={item.path}
                  item={item}
                  choice={fileChoices[item.path]}
                  onPick={(c) =>
                    setFileChoices((prev) => ({ ...prev, [item.path]: c }))
                  }
                />
              ),
            )}
          </ul>

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
              disabled={busy || !ready || items.length === 0}
              onClick={() => void apply()}
            >
              {busy ? "Applying…" : "Keep these"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
