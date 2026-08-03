import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

const MARGIN = 6;

interface ContextMenuProps {
  x: number;
  y: number;
  label: string;
  onClose: () => void;
  children: (close: () => void) => ReactNode;
}

export function ContextMenu({
  x,
  y,
  label,
  onClose,
  children,
}: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState({ left: x, top: y });

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  useEffect(() => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;
    const left = Math.max(MARGIN, Math.min(x, window.innerWidth - box.width - MARGIN));
    const top = Math.max(MARGIN, Math.min(y, window.innerHeight - box.height - MARGIN));
    setAt({ left, top });
  }, [x, y]);

  return createPortal(
    <div
      ref={ref}
      className="menu menu-context"
      role="menu"
      aria-label={label}
      style={{ left: at.left, top: at.top }}
    >
      {children(onClose)}
    </div>,
    document.body,
  );
}
