import { useState } from "react";
import { useEnv, varLabel } from "../store/env";
import { Dropdown } from "./Dropdown";
import { EnvModal } from "./EnvModal";
import { Close, Eye, Warn } from "./Icons";

function QuickLook({ onEdit }: { onEdit: () => void }) {
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const vars = envs.find((e) => e.name === selected)?.vars ?? {};
  const entries = Object.entries(vars).sort(([a], [b]) => a.localeCompare(b));

  return (
    <Dropdown
      panel
      align="right"
      trigger={({ open, toggle }) => (
        <button
          className={`btn-ghost btn-icon ${open ? "menu-item-active" : ""}`}
          onClick={toggle}
          aria-label="Environment quick look"
          title="Environment quick look"
        >
          <Eye size={15} />
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="popover-head">
            <span className="popover-title">{selected ?? "No environment"}</span>
            <button
              className="btn-ghost btn-sm"
              onClick={() => {
                close();
                onEdit();
              }}
            >
              Edit
            </button>
          </div>
          <div className="popover-body">
            {entries.length === 0 ? (
              <p className="empty-line env-quicklook-empty">
                {selected
                  ? "This environment has no variables yet."
                  : "Select an environment to see its variables."}
              </p>
            ) : (
              entries.map(([key, info]) => (
                <div className="env-quicklook-row" key={key}>
                  <span className="env-quicklook-key">{key}</span>
                  {info.secret && (
                    <span className="badge badge-secret">Secret</span>
                  )}
                  <span
                    className={`env-quicklook-value ${
                      info.secret && !info.set ? "env-secret-unset" : ""
                    }`}
                  >
                    {varLabel(info) || "—"}
                  </span>
                </div>
              ))
            )}
          </div>
        </>
      )}
    </Dropdown>
  );
}

export function EnvBar() {
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const error = useEnv((s) => s.error);
  const dismissError = useEnv((s) => s.dismissError);
  const [managing, setManaging] = useState(false);

  return (
    <div className="header-right">
      {error && (
        <span className="notice notice-error" title={error}>
          <Warn size={13} />
          <span className="notice-text">{error}</span>
          <button
            className="btn-ghost btn-icon btn-icon-sm"
            aria-label="Dismiss error"
            onClick={dismissError}
          >
            <Close size={11} />
          </button>
        </span>
      )}
      <div className="env-cluster">
      <select
        className="select env-select"
        value={selected ?? ""}
        title="Active environment"
        aria-label="Active environment"
        onChange={(e) => select(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">No environment</option>
        {envs.map((env) => (
          <option key={env.name} value={env.name}>
            {env.name}
          </option>
        ))}
      </select>
      <QuickLook onEdit={() => setManaging(true)} />
      </div>
      {managing && <EnvModal onClose={() => setManaging(false)} />}
    </div>
  );
}
