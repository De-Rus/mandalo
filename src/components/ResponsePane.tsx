import { useEffect, useMemo, useRef, useState } from "react";
import {
  errorMessage,
  type TestResult,
} from "../lib/api";
import {
  bodyText,
  formatBytes,
  formatDuration,
  statusTone,
} from "../lib/format";
import type { JsonLeaf } from "../lib/json";
import { tallyTests } from "../lib/testTally";
import { useActiveRequest, useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { EMPTY_RUN, type ResponseState, type RunResult } from "../store/session";
import { toast } from "../store/toast";
import { Check, Copy, Inbox, Search, Warn } from "./Icons";
import { countMatches, JsonView, type LeafAction } from "./JsonView";
import { Tabs, type TabItem } from "./Tabs";

function Placeholder({
  icon,
  title,
  line,
}: {
  icon: React.ReactNode;
  title?: string;
  line: string;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">{icon}</span>
      {title && <span className="empty-title">{title}</span>}
      <p className="empty-line">{line}</p>
    </div>
  );
}

function HeadersView({ headers }: { headers: [string, string][] }) {
  if (headers.length === 0)
    return <Placeholder icon={<Inbox size={26} />} line="No headers in the response." />;
  return (
    <table className="headers-table">
      <tbody>
        {headers.map(([k, v], i) => (
          <tr key={i}>
            <td className="header-key">{k}</td>
            <td>{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function TestRow({ result }: { result: TestResult }) {
  return (
    <div className="test-row">
      <span
        className={`test-verdict ${result.passed ? "test-pass" : "test-fail"}`}
      >
        {result.passed ? "PASS" : "FAIL"}
      </span>
      <div className="test-row-body">
        <span className="test-name">{result.name}</span>
        {result.detail && (
          <span className="test-detail" title={result.detail}>
            {result.detail}
          </span>
        )}
      </div>
    </div>
  );
}

function testsOf(run: RunResult): TestResult[] {
  return [...run.tests, ...run.scriptTests];
}

function TestResultsView({ run }: { run: RunResult }) {
  const all = testsOf(run);
  if (all.length === 0 && run.runError === null)
    return (
      <Placeholder
        icon={<Check size={26} />}
        line="No tests ran. Write pm.test(…) in the Post-response Script tab of the request."
      />
    );
  const passed = all.filter((t) => t.passed).length;
  const failed = all.length - passed;
  return (
    <>
      {run.runError !== null && (
        <div className="notice notice-error notice-wrap">
          <Warn size={13} />
          <span className="notice-text">
            This run stopped before its tests could finish: {run.runError}
          </span>
        </div>
      )}
      <div className="test-summary">
        <span className={`chip ${failed > 0 ? "tone-muted" : "tone-success"}`}>
          {passed} passed
        </span>
        <span className={`chip ${failed > 0 ? "tone-error" : "tone-muted"}`}>
          {failed} failed
        </span>
      </div>
      {all.map((result, i) => (
        <TestRow key={i} result={result} />
      ))}
    </>
  );
}

function ConsoleView({ logs }: { logs: string[] }) {
  if (logs.length === 0)
    return (
      <Placeholder
        icon={<Inbox size={26} />}
        line="Nothing logged. Use console.log(…) in a pre-request or test script."
      />
    );
  return (
    <div className="code-view">
      <div className="code-view-body">
        {logs.map((line, i) => (
          <div key={i}>{line}</div>
        ))}
      </div>
    </div>
  );
}

/**
 * A secret with no host is a secret that can go anywhere. The run reports where
 * it just went so the answer is one click away, at the moment the user can tell
 * whether that host is the right one.
 */
function SecretNotices({ run }: { run: RunResult }) {
  const bindHost = useEnv((s) => s.bindHost);
  const selected = useEnv((s) => s.selected);
  const [binding, setBinding] = useState<string | null>(null);

  if (run.unboundSecrets.length === 0 && run.secretVarSets.length === 0)
    return null;

  return (
    <div className="response-notices">
      {run.unboundSecrets.map((secret) => {
        const id = `${secret.env}.${secret.name}`;
        return (
          <div className="notice" key={id}>
            <Warn size={13} />
            <span className="notice-text">
              {id} was sent to {secret.host} and is not bound to a host yet.
            </span>
            <button
              className="btn-ghost"
              disabled={binding !== null}
              onClick={() => {
                setBinding(id);
                void bindHost(secret.env, secret.name, secret.host)
                  .then(() =>
                    toast("success", `${id} can now only go to ${secret.host}`),
                  )
                  .catch((e) => toast("error", errorMessage(e)))
                  .finally(() => setBinding(null));
              }}
            >
              Bind to {secret.host}
            </button>
          </div>
        );
      })}
      {run.secretVarSets.map((name) => (
        <div className="notice" key={`set-${name}`}>
          <Warn size={13} />
          <span className="notice-text">
            {selected ? `${selected}.${name}` : name} was set by a script. Its
            value stayed in this run — a secret never lands in the environment
            file. Store it from the environment editor to keep it.
          </span>
        </div>
      ))}
    </div>
  );
}

const SECRET_WORDS = new Set([
  "token",
  "secret",
  "password",
  "passwd",
  "pwd",
  "key",
  "apikey",
  "auth",
  "authorization",
  "credential",
  "credentials",
  "bearer",
  "session",
  "cookie",
  "jwt",
  "otp",
  "signature",
  "sig",
  "refresh",
]);

const SECRET_MIN_LENGTH = 24;

/** The trailing key of a JSONPath — `$.data['api key']` is `api key`. */
export function leafKey(path: string): string {
  const segments = /\.([A-Za-z_][A-Za-z0-9_]*)|\['((?:[^'\\]|\\.)*)'\]/g;
  let key = "";
  for (const match of path.matchAll(segments))
    key = match[1] ?? match[2].replace(/\\(['\\])/g, "$1");
  return key;
}

export function leafValue(leaf: JsonLeaf): unknown {
  try {
    return JSON.parse(leaf.raw);
  } catch {
    return leaf.raw;
  }
}

/**
 * A credential must not pick a scope that writes it to a git-tracked file, nor
 * leave its literal in an assertion that lands in one. Guessing wide costs a
 * scope the user can widen; guessing narrow leaks a token into a commit.
 */
export function looksSecret(leaf: JsonLeaf): boolean {
  const words = leafKey(leaf.path)
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((word) => word !== "");
  if (words.some((word) => SECRET_WORDS.has(word))) return true;
  const value = leafValue(leaf);
  return (
    typeof value === "string" &&
    value.length >= SECRET_MIN_LENGTH &&
    !/\s/.test(value)
  );
}

export function captureName(path: string, taken: string[]): string {
  const base =
    leafKey(path)
      .replace(/[^A-Za-z0-9_-]/g, "_")
      .replace(/^_+|_+$/g, "") || "value";
  if (!taken.includes(base)) return base;
  let n = 2;
  while (taken.includes(`${base}_${n}`)) n++;
  return `${base}_${n}`;
}

/** `$.data.token` → `.data.token`, the accessor chain onto `pm.response.json()`. */
function accessor(path: string): string {
  return path.startsWith("$") ? path.slice(1) : path;
}

/**
 * A `.http` file has no line for a declarative capture — the format's answer is a
 * response script, so that is what these buttons write. The engine then runs the
 * very code the file shows.
 */
export function captureLine(leaf: JsonLeaf, taken: string[]): string {
  const name = captureName(leaf.path, taken);
  return `pm.environment.set("${name}", pm.response.json()${accessor(leaf.path)});`;
}

export function assertLine(leaf: JsonLeaf): string {
  const read = `pm.response.json()${accessor(leaf.path)}`;
  if (looksSecret(leaf))
    return `pm.test("${leafKey(leaf.path)} is present", function () {\n  pm.expect(${read}).to.not.be.undefined;\n});`;
  return `pm.test("${leafKey(leaf.path)} is ${String(leafValue(leaf))}", function () {\n  pm.expect(${read}).to.eql(${JSON.stringify(leafValue(leaf))});\n});`;
}

/** Appends a line to a script, keeping one blank line between statements. */
export function appendScript(script: string, line: string): string {
  const body = script.replace(/\s+$/, "");
  return body === "" ? line : `${body}\n${line}`;
}

interface BodyViewProps {
  body: string;
  binary: boolean;
  findOpen: boolean;
  onFindHandled: () => void;
  actions: LeafAction[];
  onAction: (action: LeafAction, leaf: JsonLeaf) => void;
}

type BodyMode = "pretty" | "raw" | "preview";

const BODY_MODES: { id: BodyMode; label: string }[] = [
  { id: "pretty", label: "Pretty" },
  { id: "raw", label: "Raw" },
  { id: "preview", label: "Preview" },
];

function BodyView({
  body,
  binary,
  findOpen,
  onFindHandled,
  actions,
  onAction,
}: BodyViewProps) {
  const [mode, setMode] = useState<BodyMode>("pretty");
  const [wrap, setWrap] = useState(false);
  const [numbers, setNumbers] = useState(true);
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const findRef = useRef<HTMLInputElement>(null);

  const text = useMemo(
    () => (mode === "pretty" ? bodyText(body, binary) : body),
    [body, binary, mode],
  );
  const matches = useMemo(() => countMatches(text, query), [text, query]);

  useEffect(() => {
    if (!findOpen) return;
    setMode((m) => (m === "preview" ? "pretty" : m));
    findRef.current?.focus();
    findRef.current?.select();
    onFindHandled();
  }, [findOpen, onFindHandled]);

  useEffect(() => setCurrent(0), [query]);

  return (
    <>
      <div className="response-body-toolbar">
        <div className="segmented" role="group" aria-label="Body view">
          {BODY_MODES.map((m) => (
            <button
              key={m.id}
              className={`segment ${mode === m.id ? "segment-active" : ""}`}
              aria-pressed={mode === m.id}
              onClick={() => setMode(m.id)}
            >
              {m.label}
            </button>
          ))}
        </div>
        <button
          className={`toggle ${wrap ? "toggle-on" : ""}`}
          aria-pressed={wrap}
          disabled={mode === "preview"}
          onClick={() => setWrap((v) => !v)}
        >
          Wrap
        </button>
        <button
          className={`toggle ${numbers ? "toggle-on" : ""}`}
          aria-pressed={numbers}
          disabled={mode === "preview"}
          onClick={() => setNumbers((v) => !v)}
        >
          Line numbers
        </button>
        <span className="header-spacer" />
        <div className="find-box">
          <span className="search-icon">
            <Search size={12} />
          </span>
          <input
            ref={findRef}
            className="input find-input mono"
            placeholder="Find in body"
            aria-label="Find in body"
            value={query}
            onChange={(e) => {
              setMode((m) => (m === "preview" ? "pretty" : m));
              setQuery(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && matches > 0)
                setCurrent((c) => (c + (e.shiftKey ? -1 : 1) + matches) % matches);
              if (e.key === "Escape") setQuery("");
            }}
          />
          {query !== "" && (
            <span className="find-count">
              {matches === 0 ? "0" : `${current + 1}/${matches}`}
            </span>
          )}
        </div>
        <button
          className="btn-ghost btn-icon"
          aria-label="Copy body"
          title="Copy body"
          onClick={() => {
            void navigator.clipboard.writeText(text);
            toast("success", "Response body copied");
          }}
        >
          <Copy size={13} />
        </button>
      </div>
      {binary && (
        <div className="notice" style={{ margin: "8px 12px 0" }}>
          <Warn size={13} />
          <span className="notice-text">
            Body is not valid UTF-8 — shown lossily.
          </span>
        </div>
      )}
      {mode === "preview" ? (
        <iframe
          className="response-preview"
          title="Response preview"
          sandbox=""
          srcDoc={text}
        />
      ) : (
        <div className="response-scroll">
          <JsonView
            text={text}
            highlight={mode === "pretty" && !binary}
            lineNumbers={numbers}
            wrap={wrap}
            query={query}
            current={current}
            actions={actions}
            onAction={onAction}
          />
        </div>
      )}
    </>
  );
}

interface ResponsePaneProps {
  response: ResponseState;
  findSignal: number;
}

export function ResponsePane({ response, findSignal }: ResponsePaneProps) {
  const [tab, setTab] = useState("body");
  const [findOpen, setFindOpen] = useState(false);
  const draft = useActiveRequest();
  const updateActive = useCollection((s) => s.updateActive);

  const onLeafAction = (action: LeafAction, leaf: JsonLeaf) => {
    const source = `body.${leaf.path}`;
    if (action === "copy") {
      void navigator.clipboard.writeText(source);
      toast("success", `Copied ${source}`);
      return;
    }
    if (!draft) return;
    const script = draft.testScript;
    const read = `pm.response.json()${accessor(leaf.path)}`;
    if (action === "capture") {
      if (script.includes(`, ${read});`)) {
        toast("info", `${source} is already captured`);
        return;
      }
      const taken = [...script.matchAll(/pm\.environment\.set\("([^"]+)"/g)].map((m) => m[1]!);
      updateActive({ testScript: appendScript(script, captureLine(leaf, taken)) });
      toast(
        "success",
        looksSecret(leaf)
          ? `${source} is now set as a variable by the response script — it looks like a credential, so keep it out of a shared environment file.`
          : `${source} is now set as a variable by the response script.`,
      );
      return;
    }
    if (script.includes(`pm.expect(${read})`)) {
      toast("info", `${leaf.path} is already asserted on`);
      return;
    }
    updateActive({ testScript: appendScript(script, assertLine(leaf)) });
    toast(
      "success",
      looksSecret(leaf)
        ? `Asserting ${leaf.path} is present — its value looks like a credential, so it stays out of the request file.`
        : `Added a test on ${leaf.path} to the response script.`,
    );
  };

  const leafActions: LeafAction[] = draft
    ? ["copy", "capture", "assert"]
    : ["copy"];

  useEffect(() => {
    if (findSignal === 0) return;
    setTab("body");
    setFindOpen(true);
  }, [findSignal]);

  if (response.phase === "idle")
    return (
      <section className="response">
        <Placeholder
          icon={<Inbox size={30} />}
          title="No response yet"
          line="Hit Send (⌘⏎) and the response lands here — body, headers and test results."
        />
      </section>
    );

  if (response.phase === "loading")
    return (
      <section className="response">
        <div className="skeleton-stack">
          <div className="skeleton" style={{ width: "34%" }} />
          <div className="skeleton" style={{ width: "78%" }} />
          <div className="skeleton" style={{ width: "62%" }} />
          <div className="skeleton" style={{ width: "70%" }} />
          <div className="skeleton" style={{ width: "45%" }} />
        </div>
      </section>
    );

  if (response.phase === "error")
    return (
      <section className="response">
        <div className="response-head">
          <span className="chip tone-error">Failed</span>
        </div>
        <div className="response-scroll">
          <pre className="response-error-body">{response.message}</pre>
        </div>
      </section>
    );

  const grpc = response.phase === "grpc";
  const run = response.run ?? EMPTY_RUN;
  const headers = grpc ? [] : (response.data as { headers: [string, string][] }).headers;
  const body = response.data.body;
  const binary = grpc ? false : (response.data as { binary: boolean }).binary;
  const tally = tallyTests(run.tests, run.scriptTests, run.runError);

  const items: TabItem[] = [
    { id: "body", label: "Body" },
    { id: "headers", label: "Headers", count: headers.length },
    {
      id: "tests",
      label: "Test Results",
      badge: tally
        ? {
            text: `${tally.passed}/${tally.total}`,
            tone: tally.ok ? "success" : "danger",
          }
        : undefined,
    },
    { id: "console", label: "Console", count: run.logs.length },
  ];

  return (
    <section className="response">
      <div className="response-head">
        <span className="response-title">Response</span>
        {grpc ? (
          <span className="chip tone-success">OK</span>
        ) : (
          <span
            className={`chip status-chip tone-${statusTone((response.data as { status: number }).status)}`}
          >
            {(response.data as { status: number }).status}{" "}
            {(response.data as { statusText: string }).statusText}
          </span>
        )}
        {binary && <span className="chip tone-muted">binary</span>}
        <span className="response-meta">
          <span>
            <span className="response-meta-label">time </span>
            {formatDuration(response.data.durationMs)}
          </span>
          {!grpc && (
            <span>
              <span className="response-meta-label">size </span>
              {formatBytes((response.data as { sizeBytes: number }).sizeBytes)}
            </span>
          )}
        </span>
      </div>
      <SecretNotices run={run} />
      <Tabs items={items} active={tab} onSelect={setTab} />
      {tab === "body" && (
        <BodyView
          body={body}
          binary={binary}
          findOpen={findOpen}
          onFindHandled={() => setFindOpen(false)}
          actions={leafActions}
          onAction={onLeafAction}
        />
      )}
      {tab === "headers" && (
        <div className="response-scroll">
          <HeadersView headers={headers} />
        </div>
      )}
      {tab === "tests" && (
        <div className="response-scroll">
          <TestResultsView run={run} />
        </div>
      )}
      {tab === "console" && (
        <div className="response-scroll">
          <ConsoleView logs={run.logs} />
        </div>
      )}
    </section>
  );
}
