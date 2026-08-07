import { useState } from "react";
import { errorMessage } from "../lib/api";
import { useEnv } from "../store/env";
import { toast } from "../store/toast";
import { ConfirmModal } from "./ConfirmModal";
import { EnvModal } from "./EnvModal";
import { Close, Layers, Pencil, Trash, Warn } from "./Icons";

export function EnvList() {
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const remove = useEnv((s) => s.remove);
  const error = useEnv((s) => s.error);
  const dismissError = useEnv((s) => s.dismissError);

  const [managing, setManaging] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);

  return (
    <>
      {error && (
        <div className="notice notice-error sidebar-warning" title={error}>
          <Warn size={13} />
          <span className="notice-text">{error}</span>
          <button
            className="btn-ghost btn-icon btn-icon-sm"
            aria-label="Dismiss error"
            onClick={dismissError}
          >
            <Close size={11} />
          </button>
        </div>
      )}
      {envs.length === 0 ? (
        <div className="empty tree-empty">
          <span className="empty-icon">
            <Layers size={30} />
          </span>
          <span className="empty-title">No environments yet</span>
          <p className="empty-line">
            Create one from the header (+). Environments live as TOML files in{" "}
            <code>&lt;workspace&gt;/environments/</code>.
          </p>
        </div>
      ) : (
        <div className="sb-env-list" role="listbox" aria-label="Environments">
          {envs.map((env) => {
            const infos = Object.values(env.vars);
            const secrets = infos.filter((v) => v.secret).length;
            const active = env.name === selected;
            return (
              <div
                key={env.name}
                role="option"
                aria-selected={active}
                className={`sb-env-row ${active ? "sb-env-row-active" : ""}`}
              >
                <button
                  className="sb-env-name"
                  title={env.name}
                  onClick={() => select(env.name)}
                >
                  {env.name}
                </button>
                <span className="sb-env-count">
                  {infos.length} vars
                  {secrets > 0 && ` · ${secrets} secret`}
                </span>
                <button
                  type="button"
                  className="btn-ghost btn-icon btn-icon-sm sb-env-edit"
                  aria-label={`Edit ${env.name}`}
                  title="Edit environment"
                  onClick={() => {
                    select(env.name);
                    setManaging(true);
                  }}
                >
                  <Pencil size={12} />
                </button>
                <button
                  type="button"
                  className="btn-ghost btn-icon btn-icon-sm sb-env-remove"
                  aria-label={`Delete ${env.name}`}
                  title="Delete environment"
                  onClick={() => setConfirm(env.name)}
                >
                  <Trash size={12} />
                </button>
              </div>
            );
          })}
        </div>
      )}
      {managing && <EnvModal onClose={() => setManaging(false)} />}
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
    </>
  );
}
