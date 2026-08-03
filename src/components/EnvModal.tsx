import { useState } from "react";
import { errorMessage, type EnvironmentView, type VarInfo } from "../lib/api";
import { uid } from "../lib/draft";
import { plainVars, useEnv } from "../store/env";
import { useModalGuard } from "../store/ui";
import { Close, Plus } from "./Icons";

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

function secretEntries(env: EnvironmentView | undefined): [string, VarInfo][] {
  if (!env) return [];
  return Object.entries(env.vars).filter(([, info]) => info.secret);
}

function countLine(env: EnvironmentView): string {
  const total = Object.keys(env.vars).length;
  const secrets = secretEntries(env).length;
  if (secrets === 0) return `${total} vars`;
  return `${total} vars · ${secrets} secret`;
}

interface SecretRowProps {
  envName: string;
  name: string;
  info: VarInfo;
  onError: (message: string) => void;
}

function SecretRow({ envName, name, info, onError }: SecretRowProps) {
  const storeSecret = useEnv((s) => s.storeSecret);
  const forgetSecret = useEnv((s) => s.forgetSecret);
  const bindHost = useEnv((s) => s.bindHost);
  const removeVar = useEnv((s) => s.removeVar);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [binding, setBinding] = useState(false);
  const [host, setHost] = useState("");

  const run = async (task: Promise<unknown>) => {
    try {
      await task;
    } catch (e) {
      onError(errorMessage(e));
    }
  };

  return (
    <div className="env-secret">
      <div className="env-secret-head">
        <span className="env-secret-key mono">{name}</span>
        <span className="badge badge-secret">Secret</span>
        {info.set ? (
          <span className="env-secret-value mono">••••••••</span>
        ) : (
          <span className="env-secret-unset">not set on this machine</span>
        )}
        <span className="header-spacer" />
        <button className="btn-ghost btn-sm" onClick={() => setEditing(true)}>
          Set value
        </button>
        <button
          className="btn-ghost btn-sm"
          disabled={!info.set}
          onClick={() => void run(forgetSecret(envName, name))}
        >
          Clear
        </button>
        <button
          className="btn-ghost btn-icon"
          aria-label={`Delete ${name}`}
          onClick={() => void run(removeVar(envName, name))}
        >
          <Close size={12} />
        </button>
      </div>
      {editing && (
        <div className="env-secret-form">
          <input
            className="input mono"
            type="password"
            autoFocus
            placeholder="value — stored in the OS keychain, never in the file"
            aria-label={`Value for ${name}`}
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <button
            className="btn btn-primary btn-sm"
            disabled={value === ""}
            onClick={() => {
              void run(storeSecret(envName, name, value)).then(() => {
                setValue("");
                setEditing(false);
              });
            }}
          >
            Store
          </button>
          <button
            className="btn-ghost btn-sm"
            onClick={() => {
              setValue("");
              setEditing(false);
            }}
          >
            Cancel
          </button>
        </div>
      )}
      <div className="env-secret-hosts">
        <span className="env-secret-hosts-label">Sent only to</span>
        {(info.hosts ?? []).length === 0 ? (
          <span className="env-secret-unset">any host — not bound yet</span>
        ) : (
          (info.hosts ?? []).map((h) => (
            <span className="var-pill" key={h}>
              {h}
            </span>
          ))
        )}
        {binding ? (
          <>
            <input
              className="input mono env-host-input"
              autoFocus
              placeholder="api.acme.com"
              aria-label={`Host for ${name}`}
              value={host}
              onChange={(e) => setHost(e.target.value)}
            />
            <button
              className="btn btn-sm"
              disabled={host.trim() === ""}
              onClick={() => {
                void run(bindHost(envName, name, host.trim())).then(() => {
                  setHost("");
                  setBinding(false);
                });
              }}
            >
              Bind
            </button>
          </>
        ) : (
          <button className="btn-ghost btn-sm" onClick={() => setBinding(true)}>
            <Plus size={11} />
            Bind host
          </button>
        )}
      </div>
    </div>
  );
}

export function EnvModal({ onClose }: { onClose: () => void }) {
  useModalGuard();
  const envs = useEnv((s) => s.envs);
  const save = useEnv((s) => s.save);
  const remove = useEnv((s) => s.remove);
  const storeSecret = useEnv((s) => s.storeSecret);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [rows, setRows] = useState<VarRow[]>([]);
  const [promoting, setPromoting] = useState<string | null>(null);
  const [promoteValue, setPromoteValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const editing = envs.find((e) => e.name === editingName);
  const secrets = secretEntries(editing);

  const startEdit = (envName: string | null) => {
    setError(null);
    setPromoting(null);
    if (envName === null) {
      setEditingName("");
      setName("");
      setRows(toRows({}));
      return;
    }
    setEditingName(envName);
    setName(envName);
    setRows(toRows(plainVars(envs.find((e) => e.name === envName))));
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

  const doPromote = async (key: string) => {
    setError(null);
    try {
      await storeSecret(name.trim(), key, promoteValue);
      setRows(rows.filter((r) => r.key.trim() !== key));
      setPromoteValue("");
      setPromoting(null);
    } catch (e) {
      setError(errorMessage(e));
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
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
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
                <span className="env-item-count">{countLine(env)}</span>
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
            <div className="kv">
              <div className="kv-head kv-head-env">
                <span>Variable</span>
                <span>Value</span>
                <span />
              </div>
              {rows.map((row, i) => (
                <div className="kv-row kv-row-env" key={row.id}>
                  <input
                    className="kv-input mono"
                    placeholder="variable"
                    aria-label="Variable name"
                    value={row.key}
                    onChange={(e) => editRow(i, { key: e.target.value })}
                  />
                  <input
                    className="kv-input mono"
                    placeholder="value"
                    aria-label="Variable value"
                    value={row.value}
                    onChange={(e) => editRow(i, { value: e.target.value })}
                  />
                  <span className="kv-row-actions">
                    <button
                      className="btn-ghost btn-sm"
                      disabled={row.key.trim() === "" || editingName === ""}
                      title={
                        editingName === ""
                          ? "Save the environment before storing a secret"
                          : "Move this value into the OS keychain"
                      }
                      onClick={() => {
                        setPromoteValue(row.value);
                        setPromoting(row.key.trim());
                      }}
                    >
                      Make secret
                    </button>
                    <button
                      className="btn-ghost btn-icon"
                      aria-label="Delete variable"
                      onClick={() =>
                        setRows(
                          rows.length > 1
                            ? rows.filter((_, j) => j !== i)
                            : toRows({}),
                        )
                      }
                    >
                      <Close size={12} />
                    </button>
                  </span>
                </div>
              ))}
            </div>
            {promoting !== null && (
              <div className="env-secret-form">
                <input
                  className="input mono"
                  type="password"
                  autoFocus
                  aria-label={`Value for ${promoting}`}
                  placeholder={`value for ${promoting} — stored in the OS keychain`}
                  value={promoteValue}
                  onChange={(e) => setPromoteValue(e.target.value)}
                />
                <button
                  className="btn btn-primary btn-sm"
                  disabled={promoteValue === ""}
                  onClick={() => void doPromote(promoting)}
                >
                  Store
                </button>
                <button
                  className="btn-ghost btn-sm"
                  onClick={() => {
                    setPromoting(null);
                    setPromoteValue("");
                  }}
                >
                  Cancel
                </button>
              </div>
            )}
            {secrets.length > 0 && (
              <>
                <div className="section-title">
                  Secrets
                  <span className="section-hint">
                    declared in the file, values live in this machine's keychain
                  </span>
                </div>
                {secrets.map(([key, info]) => (
                  <SecretRow
                    key={key}
                    envName={editingName}
                    name={key}
                    info={info}
                    onError={setError}
                  />
                ))}
              </>
            )}
            {error && <p className="inline-error">{error}</p>}
            <div className="modal-actions">
              <button className="btn-ghost" onClick={() => setEditingName(null)}>
                Back
              </button>
              <button
                className="btn btn-primary"
                disabled={saving}
                onClick={() => void doSave()}
              >
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
