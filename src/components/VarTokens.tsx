import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { errorMessage, type VarSource } from "../lib/api";
import { describeVar, varTone, type VarDescription } from "../lib/vars";
import { useActiveEnv, useEnv } from "../store/env";

const MASK = "••••••••";
const GAP = 6;
const WIDTH = 260;

const SECRETS_FILE = "~/.config/mandalo/secrets.toml";

/** `.var-pop` is hover-only and ignores the pointer; the pinned form must not. */
const ADD_FORM: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: "var(--sp-2)",
  marginTop: "var(--sp-2)",
  pointerEvents: "auto",
};

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

function sourceLine(source: VarSource | null): string {
  if (source === "local") return `this machine — ${SECRETS_FILE}`;
  if (source === "environment")
    return "an exported MANDALO_SECRET__ variable, which wins over the file";
  return "the committed environment file";
}

function Hosts({ hosts }: { hosts: string[] }) {
  return (
    <div className="var-pop-note">
      {hosts.length === 0
        ? "Not bound to a host yet — it may be sent anywhere."
        : `Only sent to ${hosts.join(", ")}.`}
    </div>
  );
}

function Lines({ description }: { description: VarDescription }): ReactNode {
  if (description.state === "secret")
    return (
      <>
        <div className="var-pop-value var-pop-mask">{MASK}</div>
        <div className="var-pop-source">
          secret — the value stays on this machine, never in the workspace
        </div>
        <div className="var-pop-note">
          {description.held
            ? `Set here, in ${sourceLine(description.source)}.`
            : `Not set on this machine — the request will fail until you set it. Values live in ${SECRETS_FILE}.`}
        </div>
        <Hosts hosts={description.hosts} />
      </>
    );
  if (description.state === "local")
    return (
      <>
        <div className="var-pop-value var-pop-local">
          {description.held ? "set on this machine" : "not set"}
        </div>
        <div className="var-pop-source">
          local — not shared with your team, and not confidential
        </div>
        <div className="var-pop-note">
          {description.held
            ? `Its value comes from ${sourceLine(description.source)} and is not shown here, because only shared values reach the app.`
            : `Nothing holds a value for it — the request will fail until you set it in ${SECRETS_FILE}.`}
        </div>
        <Hosts hosts={description.hosts} />
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
          {description.env === null
            ? "Click the token to create an environment holding it."
            : `Click the token to add it to ${description.env}, or switch to an environment that has it.`}
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
      <div className="var-pop-source">
        shared — from {where(description)}, in the committed file
      </div>
    </>
  );
}

/**
 * The fix belongs where the problem is shown: an undefined variable is almost
 * always noticed on a token, and walking to the environment editor to define it
 * loses the request you were in the middle of.
 */
function AddToEnv({
  description,
  onDone,
}: {
  description: VarDescription;
  onDone: () => void;
}) {
  const addVar = useEnv((s) => s.addVar);
  const createEnv = useEnv((s) => s.createEnv);
  const [value, setValue] = useState("");
  const [envName, setEnvName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const target = description.env;

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      if (target === null)
        await createEnv(envName, { [description.name]: value });
      else await addVar(target, description.name, value);
      onDone();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <div className="var-pop-add" style={ADD_FORM}>
      {target === null && (
        <input
          className="input mono"
          autoFocus
          placeholder="environment name"
          aria-label="New environment name"
          value={envName}
          onChange={(e) => setEnvName(e.target.value)}
        />
      )}
      <input
        className="input mono"
        autoFocus={target !== null}
        placeholder="value"
        aria-label={`Value for ${description.name}`}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void submit();
        }}
      />
      <button
        className="btn btn-primary btn-sm"
        disabled={busy || (target === null && envName.trim() === "")}
        onClick={() => void submit()}
      >
        {target === null ? "Create environment" : `Add to ${target}`}
      </button>
      {error && <p className="inline-error">{error}</p>}
    </div>
  );
}

function Popover({
  description,
  box,
  id,
  panelRef,
  children,
}: {
  description: VarDescription;
  box: DOMRect;
  id: string;
  panelRef?: React.Ref<HTMLDivElement>;
  children?: ReactNode;
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
      ref={panelRef}
      style={{ left, top: box.bottom + GAP, width: WIDTH }}
    >
      <div className="var-pop-name">{`{{${description.name}}}`}</div>
      <Lines description={description} />
      {children}
    </div>,
    document.body,
  );
}

/**
 * Hover *and* focus, because the strip that used to carry this information for
 * keyboard users is gone: the token is the only affordance left, so it has to be
 * one a keyboard can reach.
 */
export function VarToken({
  description,
  text,
  onMouseDown,
}: {
  description: VarDescription;
  text: string;
  onMouseDown?: (e: React.MouseEvent) => void;
}) {
  const [box, setBox] = useState<DOMRect | null>(null);
  const [pinned, setPinned] = useState(false);
  const panel = useRef<HTMLDivElement | null>(null);
  const id = useId();
  const addable = description.state === "missing";
  const show = (e: React.SyntheticEvent<HTMLElement>) =>
    setBox(e.currentTarget.getBoundingClientRect());
  const hide = () => {
    if (!pinned) setBox(null);
  };
  const close = () => {
    setPinned(false);
    setBox(null);
  };
  const pin = (e: React.SyntheticEvent<HTMLElement>) => {
    if (!addable) return;
    setBox(e.currentTarget.getBoundingClientRect());
    setPinned(true);
  };

  useEffect(() => {
    if (!pinned) return;
    const onDown = (e: MouseEvent) => {
      if (!panel.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [pinned]);

  return (
    <span
      className={`var-token ${varTone(description.state)} ${addable ? "var-token-addable" : ""}`}
      tabIndex={0}
      aria-label={`variable ${description.name}`}
      aria-describedby={box ? id : undefined}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
      onClick={pin}
      onKeyDown={(e) => {
        if (e.key === "Escape") close();
        if (e.key === "Enter") pin(e);
      }}
      onMouseDown={onMouseDown}
    >
      {text}
      {box && (
        <Popover description={description} box={box} id={id} panelRef={panel}>
          {pinned && <AddToEnv description={description} onDone={close} />}
        </Popover>
      )}
    </span>
  );
}
