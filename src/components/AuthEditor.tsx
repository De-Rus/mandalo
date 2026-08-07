import type { AuthDraft, AuthType } from "../lib/draft";

interface AuthEditorProps {
  auth: AuthDraft;
  onChange: (auth: AuthDraft) => void;
}

export function AuthEditor({ auth, onChange }: AuthEditorProps) {
  const patch = (p: Partial<AuthDraft>) => onChange({ ...auth, ...p });

  return (
    <div className="auth-editor">
      {auth.inherited && (
        <div className="field auth-inherited" role="note">
          <span className="field-label">Source</span>
          <span className="settings-hint">
            Inherited from the collection — the type and values below come from
            it, and follow it when it changes.{" "}
            <button
              className="btn btn-sm"
              onClick={() => patch({ inherited: false })}
            >
              Use its own
            </button>
          </span>
        </div>
      )}
      <label className="field">
        <span className="field-label">Type</span>
        <select
          className="select"
          value={auth.type}
          disabled={auth.inherited}
          onChange={(e) => patch({ type: e.target.value as AuthType })}
        >
          <option value="none">None</option>
          <option value="bearer">Bearer Token</option>
          <option value="basic">Basic Auth</option>
          <option value="apikey">API Key</option>
        </select>
      </label>
      {auth.type === "none" && (
        <p className="empty-line">This request is sent without authentication.</p>
      )}
      {auth.type === "bearer" && (
        <label className="field">
          <span className="field-label">Token</span>
          <input
            className="input mono"
            value={auth.token}
            readOnly={auth.inherited}
            placeholder="eyJhbGciOi…"
            onChange={(e) => patch({ token: e.target.value })}
          />
        </label>
      )}
      {auth.type === "basic" && (
        <>
          <label className="field">
            <span className="field-label">Username</span>
            <input
              className="input"
              value={auth.username}
              readOnly={auth.inherited}
              onChange={(e) => patch({ username: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Password</span>
            <input
              className="input"
              type="password"
              value={auth.password}
              readOnly={auth.inherited}
              onChange={(e) => patch({ password: e.target.value })}
            />
          </label>
        </>
      )}
      {auth.type === "apikey" && (
        <>
          <label className="field">
            <span className="field-label">Key</span>
            <input
              className="input mono"
              value={auth.key}
              readOnly={auth.inherited}
              placeholder="X-Api-Key"
              onChange={(e) => patch({ key: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Value</span>
            <input
              className="input mono"
              value={auth.value}
              readOnly={auth.inherited}
              onChange={(e) => patch({ value: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Add to</span>
            <select
              className="select"
              value={auth.placement}
              disabled={auth.inherited}
              onChange={(e) =>
                patch({ placement: e.target.value as "header" | "query" })
              }
            >
              <option value="header">Header</option>
              <option value="query">Query params</option>
            </select>
          </label>
        </>
      )}
    </div>
  );
}
