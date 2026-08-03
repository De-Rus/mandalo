import type { AuthDraft, AuthType } from "../lib/draft";

interface AuthEditorProps {
  auth: AuthDraft;
  onChange: (auth: AuthDraft) => void;
}

export function AuthEditor({ auth, onChange }: AuthEditorProps) {
  const patch = (p: Partial<AuthDraft>) => onChange({ ...auth, ...p });

  return (
    <div className="auth-editor">
      <label className="field">
        <span className="field-label">Type</span>
        <select
          className="select"
          value={auth.type}
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
              onChange={(e) => patch({ username: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Password</span>
            <input
              className="input"
              type="password"
              value={auth.password}
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
              placeholder="X-Api-Key"
              onChange={(e) => patch({ key: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Value</span>
            <input
              className="input mono"
              value={auth.value}
              onChange={(e) => patch({ value: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Add to</span>
            <select
              className="select"
              value={auth.placement}
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
