import { useState } from "react";
import { emptyRow, uid, type KVRow } from "../lib/draft";
import { Close } from "./Icons";

export function toBulk(rows: KVRow[]): string {
  return rows
    .filter((r) => r.key.trim() !== "" || r.value.trim() !== "")
    .map((r) => `${r.enabled ? "" : "//"}${r.key}:${r.value}`)
    .join("\n");
}

export function fromBulk(text: string): KVRow[] {
  const rows: KVRow[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    const enabled = !line.startsWith("//");
    const rest = enabled ? line : line.slice(2);
    const at = rest.indexOf(":");
    rows.push({
      id: uid(),
      key: at === -1 ? rest.trim() : rest.slice(0, at).trim(),
      value: at === -1 ? "" : rest.slice(at + 1).trim(),
      enabled,
    });
  }
  rows.push(emptyRow());
  return rows;
}

interface KeyValueEditorProps {
  rows: KVRow[];
  onChange: (rows: KVRow[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  keyLabel?: string;
  valueLabel?: string;
}

export function KeyValueEditor({
  rows,
  onChange,
  keyPlaceholder = "Key",
  valuePlaceholder = "Value",
  keyLabel = "Key",
  valueLabel = "Value",
}: KeyValueEditorProps) {
  const [bulk, setBulk] = useState<string | null>(null);

  const edit = (index: number, patch: Partial<KVRow>) => {
    const next = rows.map((r, i) => (i === index ? { ...r, ...patch } : r));
    const last = next[next.length - 1];
    if (last.key !== "" || last.value !== "") next.push(emptyRow());
    onChange(next);
  };

  const remove = (index: number) => {
    const next = rows.filter((_, i) => i !== index);
    onChange(next.length > 0 ? next : [emptyRow()]);
  };

  return (
    <div className="kv">
      <div className="kv-toolbar">
        <button
          className="btn-link"
          onClick={() => {
            if (bulk === null) {
              setBulk(toBulk(rows));
              return;
            }
            onChange(fromBulk(bulk));
            setBulk(null);
          }}
        >
          {bulk === null ? "Bulk Edit" : "Key-Value Edit"}
        </button>
      </div>
      {bulk !== null ? (
        <textarea
          className="textarea mono kv-bulk"
          aria-label={`${keyLabel} bulk edit`}
          placeholder={`${keyPlaceholder}:${valuePlaceholder}`}
          spellCheck={false}
          value={bulk}
          onChange={(e) => setBulk(e.target.value)}
        />
      ) : (
        <>
          <div className="kv-head">
            <span />
            <span>{keyLabel}</span>
            <span>{valueLabel}</span>
            <span />
          </div>
          {rows.map((row, i) => (
            <div
              className={`kv-row ${row.enabled ? "" : "kv-row-off"}`}
              key={row.id}
            >
              <span className="kv-check-cell">
                <input
                  type="checkbox"
                  className="checkbox"
                  aria-label="Enable row"
                  checked={row.enabled}
                  onChange={(e) => edit(i, { enabled: e.target.checked })}
                />
              </span>
              <input
                className="kv-input"
                placeholder={keyPlaceholder}
                value={row.key}
                spellCheck={false}
                onChange={(e) => edit(i, { key: e.target.value })}
              />
              <input
                className="kv-input"
                placeholder={valuePlaceholder}
                value={row.value}
                spellCheck={false}
                onChange={(e) => edit(i, { value: e.target.value })}
              />
              <button
                className="btn-ghost btn-icon kv-del"
                aria-label="Delete row"
                title="Delete row"
                onClick={() => remove(i)}
              >
                <Close size={12} />
              </button>
            </div>
          ))}
        </>
      )}
    </div>
  );
}
