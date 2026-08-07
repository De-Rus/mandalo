import { useState } from "react";
import { errorMessage } from "../lib/api";
import { useEnv } from "../store/env";
import { toast } from "../store/toast";
import { ConfirmModal } from "./ConfirmModal";
import { Dropdown, MenuItem } from "./Dropdown";
import { EnvModal } from "./EnvModal";
import { Check, ChevronDown, Close, Plus, Trash, Warn } from "./Icons";

export function EnvBar() {
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const remove = useEnv((s) => s.remove);
  const error = useEnv((s) => s.error);
  const dismissError = useEnv((s) => s.dismissError);
  const [creating, setCreating] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);

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
        <Dropdown
          align="right"
          menuClassName="env-menu"
          trigger={({ open, toggle }) => (
            <button
              type="button"
              className={`ws-trigger env-trigger ${selected ? "" : "env-trigger-empty"} ${open ? "ws-trigger-open" : ""}`}
              title="Active environment"
              aria-label="Active environment"
              aria-haspopup="menu"
              aria-expanded={open}
              onClick={toggle}
            >
              <span className="ws-name">
                {selected ?? "No Environment"}
              </span>
              <ChevronDown size={12} className="ws-caret" />
            </button>
          )}
        >
          {(close) => (
            <>
              <div className="menu-head">Environments</div>
              <div
                className={`menu-item ws-item env-item ${selected === null ? "menu-item-active" : ""}`}
                role="menuitem"
              >
                <button
                  type="button"
                  className="ws-item-pick"
                  onClick={() => {
                    close();
                    select(null);
                  }}
                >
                  <span className="menu-item-icon">
                    {selected === null ? <Check size={13} /> : null}
                  </span>
                  <span className="menu-item-label">No Environment</span>
                </button>
              </div>
              {envs.map((env) => (
                <div
                  key={env.name}
                  className={`menu-item ws-item env-item ${env.name === selected ? "menu-item-active" : ""}`}
                  role="menuitem"
                >
                  <button
                    type="button"
                    className="ws-item-pick"
                    onClick={() => {
                      close();
                      select(env.name);
                    }}
                  >
                    <span className="menu-item-icon">
                      {env.name === selected ? <Check size={13} /> : null}
                    </span>
                    <span className="menu-item-label">{env.name}</span>
                  </button>
                  <button
                    type="button"
                    className="btn-ghost btn-icon btn-icon-sm ws-item-remove"
                    aria-label={`Delete ${env.name}`}
                    title="Delete environment"
                    onClick={(e) => {
                      e.stopPropagation();
                      close();
                      setConfirm(env.name);
                    }}
                  >
                    <Trash size={12} />
                  </button>
                </div>
              ))}
              <div className="menu-sep" />
              <MenuItem
                icon={<Plus size={13} />}
                onClick={() => {
                  close();
                  setCreating(true);
                }}
              >
                New environment…
              </MenuItem>
            </>
          )}
        </Dropdown>
      </div>
      {creating && (
        <EnvModal create onClose={() => setCreating(false)} />
      )}
      {confirm !== null && (
        <ConfirmModal
          title="Delete environment"
          message={`“${confirm}” will be removed from disk, along with every variable it declares. This cannot be undone.`}
          onConfirm={() => {
            void remove(confirm).catch((e) => toast("error", errorMessage(e)));
          }}
          onClose={() => setConfirm(null)}
        />
      )}
    </div>
  );
}
