import { useState } from "react";
import { useModalGuard } from "../store/ui";
import { Close } from "./Icons";

interface PromptModalProps {
  title: string;
  label: string;
  initial?: string;
  placeholder?: string;
  confirmLabel?: string;
  onSubmit: (value: string) => void;
  onClose: () => void;
}

export function PromptModal({
  title,
  label,
  initial = "",
  placeholder,
  confirmLabel = "Create",
  onSubmit,
  onClose,
}: PromptModalProps) {
  useModalGuard();
  const [value, setValue] = useState(initial);

  const submit = () => {
    if (value.trim() === "") return;
    onSubmit(value.trim());
    onClose();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        style={{ width: 420 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{title}</h2>
          <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">{label}</span>
            <input
              className="input"
              autoFocus
              value={value}
              placeholder={placeholder}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") submit();
                if (e.key === "Escape") onClose();
              }}
            />
          </label>
          <div className="modal-actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={value.trim() === ""}
              onClick={submit}
            >
              {confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
