import { emptyRow, type KVRow } from "../lib/draft";

interface KeyValueEditorProps {
  rows: KVRow[];
  onChange: (rows: KVRow[]) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
}

export function KeyValueEditor({
  rows,
  onChange,
  keyPlaceholder = "Key",
  valuePlaceholder = "Value",
}: KeyValueEditorProps) {
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
    <div className="kv-editor">
      {rows.map((row, i) => (
        <div className="kv-row" key={row.id}>
          <input
            type="checkbox"
            className="kv-check"
            aria-label="Enable row"
            checked={row.enabled}
            onChange={(e) => edit(i, { enabled: e.target.checked })}
          />
          <input
            className="kv-input mono"
            placeholder={keyPlaceholder}
            value={row.key}
            onChange={(e) => edit(i, { key: e.target.value })}
          />
          <input
            className="kv-input mono"
            placeholder={valuePlaceholder}
            value={row.value}
            onChange={(e) => edit(i, { value: e.target.value })}
          />
          <button
            className="kv-delete"
            aria-label="Delete row"
            title="Delete row"
            onClick={() => remove(i)}
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
