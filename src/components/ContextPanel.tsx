import { useState, type KeyboardEvent } from "react";
import { errorMessage } from "../lib/api";
import { plainVars, useActiveEnv, useEnv } from "../store/env";
import { CONTEXT_WIDTH, useLayout } from "../store/layout";
import { ContextVarRow } from "./ContextVarRow";
import { EnvModal } from "./EnvModal";
import { Layers, Search } from "./Icons";

const FILTER_THRESHOLD = 8;

export function ContextPanel() {
  const setContextOpen = useLayout((s) => s.setContextOpen);
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const save = useEnv((s) => s.save);
  const removeVar = useEnv((s) => s.removeVar);
  const storeSecret = useEnv((s) => s.storeSecret);
  const forgetSecret = useEnv((s) => s.forgetSecret);
  const env = useActiveEnv();
  const [managing, setManaging] = useState(false);
  const [filter, setFilter] = useState("");
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [addError, setAddError] = useState<string | null>(null);

  const missing = selected !== null && !envs.some((e) => e.name === selected);
  const entries = Object.entries(env?.vars ?? {}).sort(([a], [b]) =>
    a.localeCompare(b),
  );
  const query = filter.trim().toLowerCase();
  const shown =
    query === ""
      ? entries
      : entries.filter(([key]) => key.toLowerCase().includes(query));

  const commit = async (key: string, value: string) => {
    if (!selected) throw new Error("No environment selected");
    await save({ name: selected, vars: { ...plainVars(env), [key]: value } });
  };

  const add = async () => {
    const key = newKey.trim();
    setAddError(null);
    try {
      await commit(key, newValue);
      setNewKey("");
      setNewValue("");
    } catch (e) {
      setAddError(errorMessage(e));
    }
  };

  // WHY: the panel is docked, not modal — Escape only claims the key when focus
  // is already inside it, and hands focus back to the header toggle.
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    setContextOpen(false);
    document.querySelector<HTMLElement>(".ctx-toggle")?.focus();
  };

  return (
    <aside
      className="ctx-panel"
      style={{ width: CONTEXT_WIDTH }}
      aria-label="Environment panel"
      onKeyDown={onKeyDown}
    >
      <div className="ctx-body">
        {missing ? (
          <div className="notice notice-error ctx-notice">
            {`Environment “${selected}” is gone from the workspace.`}
          </div>
        ) : selected === null || env === null ? (
          <div className="empty">
            <span className="empty-icon">
              <Layers size={30} />
            </span>
            <span className="empty-title">No environment selected</span>
            <p className="empty-line">
              Pick one in the header to see and edit its variables.
            </p>
            <button
              className="btn btn-primary"
              onClick={() => setManaging(true)}
            >
              Manage environments
            </button>
          </div>
        ) : (
          <>
            {entries.length > FILTER_THRESHOLD && (
              <div className="ctx-filter search-wrap">
                <span className="search-icon">
                  <Search size={13} />
                </span>
                <input
                  className="input search-input"
                  placeholder="Filter variables"
                  aria-label="Filter variables"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                />
              </div>
            )}
            {entries.length === 0 ? (
              <div className="empty">
                <span className="empty-icon">
                  <Layers size={30} />
                </span>
                <span className="empty-title">No variables yet</span>
                <p className="empty-line">
                  Add one below — it is written to{" "}
                  <code>{`<workspace>/environments/${selected}.toml`}</code>.
                </p>
              </div>
            ) : shown.length === 0 ? (
              <p className="empty-line ctx-nomatch">{`Nothing matches “${filter.trim()}”`}</p>
            ) : (
              <>
                <div className="ctx-list-head">
                  <span>Variable</span>
                  <span>Value</span>
                  <span>Scope</span>
                </div>
                <div className="ctx-list">
                  {shown.map(([key, info]) => (
                    <ContextVarRow
                      key={key}
                      envName={selected}
                      name={key}
                      info={info}
                      commit={commit}
                      storeSecret={storeSecret}
                      forgetSecret={forgetSecret}
                      removeVar={removeVar}
                      onEditInModal={() => setManaging(true)}
                    />
                  ))}
                </div>
              </>
            )}
            <div className="ctx-add">
              <input
                className="input mono"
                placeholder="variable"
                aria-label="New variable name"
                value={newKey}
                onChange={(e) => setNewKey(e.target.value)}
              />
              <input
                className="input mono"
                placeholder="value"
                aria-label="New variable value"
                value={newValue}
                onChange={(e) => setNewValue(e.target.value)}
              />
              <button
                className="btn btn-sm"
                disabled={newKey.trim() === ""}
                onClick={() => void add()}
              >
                Add
              </button>
            </div>
            {addError && <p className="inline-error ctx-add-error">{addError}</p>}
          </>
        )}
      </div>
      {managing && <EnvModal onClose={() => setManaging(false)} />}
    </aside>
  );
}
