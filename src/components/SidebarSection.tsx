import type { ReactNode } from "react";
import { ChevronDown, ChevronRight } from "./Icons";

interface SidebarSectionProps {
  id: string;
  title: string;
  open: boolean;
  onToggle: (open: boolean) => void;
  count?: number | undefined;
  /** Whether the open body takes the leftover height or only what it needs. */
  grow?: boolean;
  children: ReactNode;
}

export function SidebarSection({
  id,
  title,
  open,
  onToggle,
  count,
  grow = false,
  children,
}: SidebarSectionProps) {
  return (
    <section
      className={`sb-section ${open ? "sb-section-open" : ""} ${
        grow && open ? "sb-section-grow" : ""
      }`}
    >
      <h2 className="sb-section-head">
        <button
          className="sb-section-toggle"
          aria-expanded={open}
          aria-controls={`sb-body-${id}`}
          onClick={() => onToggle(!open)}
        >
          <span className="sb-section-chevron">
            {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          </span>
          <span className="sb-section-title">{title}</span>
          {count !== undefined && count > 0 && <span className="count">{count}</span>}
        </button>
      </h2>
      {open && (
        <div className="sb-section-body" id={`sb-body-${id}`}>
          {children}
        </div>
      )}
    </section>
  );
}
