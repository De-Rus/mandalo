import { useId, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { describeVar, varTone, type VarDescription } from "../lib/vars";
import { useActiveEnv } from "../store/env";

const MASK = "••••••••";
const GAP = 6;
const WIDTH = 260;

export function useVarLookup(
  vars: Record<string, string>,
): (name: string) => VarDescription {
  const env = useActiveEnv();
  return (name) => describeVar(name, env, vars);
}

function where(description: VarDescription): string {
  return description.env === null
    ? "no environment selected"
    : `environment ${description.env}`;
}

function Lines({ description }: { description: VarDescription }): ReactNode {
  if (description.state === "secret")
    return (
      <>
        <div className="var-pop-value var-pop-mask">{MASK}</div>
        <div className="var-pop-source">
          secret — stored in your OS keychain
        </div>
        <div className="var-pop-note">
          {description.secretSet
            ? "Set on this machine."
            : "Not set on this machine — the request will fail until you set it."}
        </div>
      </>
    );
  if (description.state === "dynamic")
    return (
      <>
        <div className="var-pop-value">generated for every run</div>
        <div className="var-pop-source">built-in dynamic variable</div>
      </>
    );
  if (description.state === "missing")
    return (
      <>
        <div className="var-pop-value var-pop-missing">not set</div>
        <div className="var-pop-source">not defined in {where(description)}</div>
        <div className="var-pop-note">
          Add it in the environment editor, or switch to an environment that has
          it.
        </div>
      </>
    );
  return (
    <>
      <div className="var-pop-value">
        {description.value === "" ? (
          <span className="var-pop-empty">empty string</span>
        ) : (
          description.value
        )}
      </div>
      <div className="var-pop-source">from {where(description)}</div>
    </>
  );
}

function Popover({
  description,
  box,
  id,
}: {
  description: VarDescription;
  box: DOMRect;
  id: string;
}) {
  const left = Math.max(
    GAP,
    Math.min(box.left, (window.innerWidth || WIDTH + GAP * 2) - WIDTH - GAP),
  );
  return createPortal(
    <div
      className="var-pop"
      role="tooltip"
      id={id}
      style={{ left, top: box.bottom + GAP, width: WIDTH }}
    >
      <div className="var-pop-name">{`{{${description.name}}}`}</div>
      <Lines description={description} />
    </div>,
    document.body,
  );
}

interface HoverProps {
  description: VarDescription;
  className: string;
  focusable?: boolean;
  onMouseDown?: (e: React.MouseEvent) => void;
  children: ReactNode;
}

function Hoverable({
  description,
  className,
  focusable = false,
  onMouseDown,
  children,
}: HoverProps) {
  const [box, setBox] = useState<DOMRect | null>(null);
  const id = useId();
  const show = (e: React.SyntheticEvent<HTMLElement>) =>
    setBox(e.currentTarget.getBoundingClientRect());
  const hide = () => setBox(null);
  return (
    <span
      className={className}
      tabIndex={focusable ? 0 : undefined}
      aria-describedby={box ? id : undefined}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={focusable ? show : undefined}
      onBlur={focusable ? hide : undefined}
      onMouseDown={onMouseDown}
    >
      {children}
      {box && <Popover description={description} box={box} id={id} />}
    </span>
  );
}

export function VarToken({
  description,
  text,
  onMouseDown,
}: {
  description: VarDescription;
  text: string;
  onMouseDown?: (e: React.MouseEvent) => void;
}) {
  return (
    <Hoverable
      description={description}
      className={`var-token ${varTone(description.state)}`}
      onMouseDown={onMouseDown}
    >
      {text}
    </Hoverable>
  );
}

export function VarPill({ description }: { description: VarDescription }) {
  const value =
    description.state === "secret"
      ? MASK
      : description.state === "dynamic"
        ? "per run"
        : description.state === "missing"
          ? "not set"
          : description.value;
  return (
    <Hoverable
      description={description}
      className={`var-pill var-pill-${description.state}`}
      focusable
    >
      {description.name}
      <span className="var-pill-value">{value}</span>
    </Hoverable>
  );
}
