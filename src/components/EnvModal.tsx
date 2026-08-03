import { useState } from "react";
import { errorMessage } from "../lib/api";
import { uid } from "../lib/draft";
import { useEnv } from "../store/env";
import { useModalGuard } from "../store/ui";

interface VarRow {
  id: string;
  key: string;
  value: string;
}

function toRows(vars: Record<string, string>): VarRow[] {
  const rows = Object.entries(vars).map(([key, value]) => ({
    id: uid(),
    key,
    value,
  }));
  rows.push({ id: uid(), key: "", value: "" });
  return rows;
}

function toVars(rows: VarRow[]): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const row of rows) {
    if (row.key.trim() !== "") vars[row.key.trim()] = row.value;
  }
  return vars;
}

export function EnvModal({ onClose }: { onClose: () => void }) {
  useModalGuard();
  const envs = useEnv((s) => s.envs);
  const save = useEnv((s) => s.save);
  const remove = useEnv((s) => s.remove);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [rows, setRows] = useState<VarRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const startEdit = (envName: string | null) => {
    setError(null);
    if (envName === null) {
      setEditingName("");
      setName("");
      setRows(toRows({}));
    } else {
      const env = envs.find((e) => e.name === envName);
      setEditingName(envName);
      setName(envName);
      setRows(toRows(env?.vars ?? {}));
    }
  };

  const editRow = (index: number, patch: Partial<VarRow>) => {
    const next = rows.map((r, i) => (i === index ? { ...r, ...patch } : r));
    const last = next[next.length - 1];
    if (last.key !== "" || last.value !== "")
      next.push({ id: uid(), key: "", value: "" });
    setRows(next);
  };

  const doSave = async () => {
    const newName = name.trim();
    if (newName === "") {
      setError("Environment name is required");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await save({ name: newName, vars: toVars(rows) });
      if (editingName && editingName !== newName) {
        if (selected === editingName) select(newName);
        await remove(editingName);
      }
      setEditingName(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const doDelete = async (envName: string) => {
    setError(null);
    try {
      await remove(envName);
      if (editingName === envName) setEditingName(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Environments</h2>
          <button className="btn-ghost" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>
        {editingName === null ? (
          <div className="modal-body">
            {envs.length === 0 && (
              <p className="empty-line">
                No environments yet. Create one to reuse variables like{" "}
                <code>{"{{baseUrl}}"}</code> across requests.
              </p>
            )}
            {envs.map((env) => (
              <div key={env.name} className="env-item">
                <span className="env-item-name">{env.name}</span>
                <span className="env-item-count">
                  {Object.keys(env.vars).length} vars
                </span>
                <button className="btn-ghost" onClick={() => startEdit(env.name)}>
                  Edit
                </button>
                <button
                  className="btn-ghost danger"
                  onClick={() => void doDelete(env.name)}
                >
                  Delete
                </button>
              </div>
            ))}
            <button className="btn" onClick={() => startEdit(null)}>
              + New environment
            </button>
            {error && <p className="inline-error">{error}</p>}
          </div>
        ) : (
          <div className="modal-body">
            <label className="field">
              <span className="field-label">Name</span>
              <input
                className="input"
                value={name}
                placeholder="staging"
                autoFocus={editingName === ""}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <div className="kv-editor">
              {rows.map((row, i) => (
                <div className="kv-row kv-row-plain" key={row.id}>
                  <input
                    className="kv-input mono"
                    placeholder="variable"
                    value={row.key}
                    onChange={(e) => editRow(i, { key: e.target.value })}
                  />
                  <input
                    className="kv-input mono"
                    placeholder="value"
                    value={row.value}
                    onChange={(e) => editRow(i, { value: e.target.value })}
                  />
                  <button
                    className="kv-delete"
                    aria-label="Delete variable"
                    onClick={() =>
                      setRows(
                        rows.length > 1
                          ? rows.filter((_, j) => j !== i)
                          : toRows({}),
                      )
                    }
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
            {error && <p className="inline-error">{error}</p>}
            <div className="modal-actions">
              <button className="btn-ghost" onClick={() => setEditingName(null)}>
                Back
              </button>
              <button className="btn btn-primary" disabled={saving} onClick={() => void doSave()}>
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
