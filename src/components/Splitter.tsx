import { useEffect, useRef, useState } from "react";

interface SplitterProps {
  orientation: "vertical" | "horizontal";
  onDrag: (clientPosition: number) => void;
  onNudge: (delta: number) => void;
  label: string;
}

export function Splitter({
  orientation,
  onDrag,
  onNudge,
  label,
}: SplitterProps) {
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef(onDrag);
  dragRef.current = onDrag;

  const vertical = orientation === "vertical";

  useEffect(() => {
    if (!dragging) return;
    const bodyClass = vertical ? "dragging-col" : "dragging-row";
    document.body.classList.add(bodyClass);
    const onMove = (e: MouseEvent) =>
      dragRef.current(vertical ? e.clientX : e.clientY);
    const onUp = () => setDragging(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      document.body.classList.remove(bodyClass);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging, vertical]);

  return (
    <div
      role="separator"
      aria-label={label}
      aria-orientation={vertical ? "vertical" : "horizontal"}
      tabIndex={0}
      className={`${vertical ? "splitter-v" : "splitter-h"} ${
        dragging ? (vertical ? "splitter-v-active" : "splitter-h-active") : ""
      }`}
      onMouseDown={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onKeyDown={(e) => {
        const back = vertical ? "ArrowLeft" : "ArrowUp";
        const forward = vertical ? "ArrowRight" : "ArrowDown";
        if (e.key !== back && e.key !== forward) return;
        e.preventDefault();
        onNudge((e.key === back ? -1 : 1) * (e.shiftKey ? 40 : 12));
      }}
    />
  );
}
