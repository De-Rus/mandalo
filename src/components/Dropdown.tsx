import { useEffect, useRef, useState, type ReactNode } from "react";

interface DropdownProps {
  trigger: (state: { open: boolean; toggle: () => void }) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: "left" | "right";
  menuClassName?: string;
  panel?: boolean;
}

export function Dropdown({
  trigger,
  children,
  align = "left",
  menuClassName = "",
  panel = false,
}: DropdownProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node))
        setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const base = panel ? "popover" : `menu menu-${align}`;

  return (
    <div className="dropdown" ref={wrapRef}>
      {trigger({ open, toggle: () => setOpen((v) => !v) })}
      {open && (
        <div className={`${base} ${menuClassName}`} role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

interface MenuItemProps {
  icon?: ReactNode;
  hint?: string;
  danger?: boolean;
  onClick: () => void;
  children: ReactNode;
}

export function MenuItem({
  icon,
  hint,
  danger = false,
  onClick,
  children,
}: MenuItemProps) {
  return (
    <button
      className={`menu-item ${danger ? "danger" : ""}`}
      role="menuitem"
      onClick={onClick}
    >
      {icon !== undefined && <span className="menu-item-icon">{icon}</span>}
      <span className="menu-item-label">{children}</span>
      {hint !== undefined && <span className="menu-item-hint">{hint}</span>}
    </button>
  );
}
