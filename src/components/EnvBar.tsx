import { useEffect, useState } from "react";
import { useEnv } from "../store/env";
import { EnvModal } from "./EnvModal";

export function EnvBar() {
  const envs = useEnv((s) => s.envs);
  const selected = useEnv((s) => s.selected);
  const select = useEnv((s) => s.select);
  const init = useEnv((s) => s.init);
  const error = useEnv((s) => s.error);
  const dismissError = useEnv((s) => s.dismissError);
  const [managing, setManaging] = useState(false);

  useEffect(() => {
    void init();
  }, [init]);

  return (
    <div className="env-bar">
      {error && (
        <span className="env-error">
          <span className="env-error-text" title={error}>
            {error}
          </span>
          <button
            className="btn-ghost"
            aria-label="Dismiss error"
            onClick={dismissError}
          >
            ×
          </button>
        </span>
      )}
      <select
        className="select"
        value={selected ?? ""}
        title="Active environment"
        onChange={(e) => select(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">No environment</option>
        {envs.map((env) => (
          <option key={env.name} value={env.name}>
            {env.name}
          </option>
        ))}
      </select>
      <button className="btn-ghost" onClick={() => setManaging(true)}>
        Manage
      </button>
      {managing && <EnvModal onClose={() => setManaging(false)} />}
    </div>
  );
}
