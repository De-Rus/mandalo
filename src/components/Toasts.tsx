import { Check, Close, Warn } from "./Icons";
import { useToasts } from "../store/toast";

export function Toasts() {
  const items = useToasts((s) => s.items);
  const dismiss = useToasts((s) => s.dismiss);
  if (items.length === 0) return null;

  return (
    <div className="toasts" role="status" aria-live="polite">
      {items.map((t) => (
        <div key={t.id} className={`toast toast-${t.tone}`}>
          <span className={t.tone === "error" ? "tone-error" : "tone-success"}>
            {t.tone === "error" ? <Warn size={13} /> : <Check size={13} />}
          </span>
          <span className="toast-text" title={t.text}>
            {t.text}
          </span>
          <button
            className="btn-ghost btn-icon btn-icon-sm"
            aria-label="Dismiss"
            onClick={() => dismiss(t.id)}
          >
            <Close size={11} />
          </button>
        </div>
      ))}
    </div>
  );
}
