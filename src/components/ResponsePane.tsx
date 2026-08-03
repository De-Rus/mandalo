import { useEffect, useMemo, useRef, useState } from "react";
import { errorMessage, type TestResult } from "../lib/api";
import {
  bodyText,
  formatBytes,
  formatDuration,
  statusTone,
} from "../lib/format";
import { useEnv } from "../store/env";
import { EMPTY_RUN, type ResponseState, type RunResult } from "../store/session";
import { toast } from "../store/toast";
import { Check, Copy, Inbox, Search, Warn } from "./Icons";
import { countMatches, JsonView } from "./JsonView";
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
  if (all.length === 0)
    return (
      <Placeholder
        icon={<Check size={26} />}
        line="No tests ran. Write pm.test(…) in the Tests tab of the request."
      />
    );
  const passed = all.filter((t) => t.passed).length;
  const failed = all.length - passed;
  return (
    <>
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

interface BodyViewProps {
  body: string;
  binary: boolean;
  findOpen: boolean;
  onFindHandled: () => void;
}

function BodyView({ body, binary, findOpen, onFindHandled }: BodyViewProps) {
  const [pretty, setPretty] = useState(true);
  const [wrap, setWrap] = useState(false);
  const [numbers, setNumbers] = useState(true);
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const findRef = useRef<HTMLInputElement>(null);

  const text = useMemo(
    () => (pretty ? bodyText(body, binary) : body),
    [body, binary, pretty],
  );
  const matches = useMemo(() => countMatches(text, query), [text, query]);

  useEffect(() => {
    if (!findOpen) return;
    findRef.current?.focus();
    findRef.current?.select();
    onFindHandled();
  }, [findOpen, onFindHandled]);

  useEffect(() => setCurrent(0), [query]);

  return (
    <>
      <div className="response-body-toolbar">
        <div className="segmented" role="group" aria-label="Body view">
          <button
            className={`segment ${pretty ? "segment-active" : ""}`}
            onClick={() => setPretty(true)}
          >
            Pretty
          </button>
          <button
            className={`segment ${pretty ? "" : "segment-active"}`}
            onClick={() => setPretty(false)}
          >
            Raw
          </button>
        </div>
        <button
          className={`toggle ${wrap ? "toggle-on" : ""}`}
          aria-pressed={wrap}
          onClick={() => setWrap((v) => !v)}
        >
          Wrap
        </button>
        <button
          className={`toggle ${numbers ? "toggle-on" : ""}`}
          aria-pressed={numbers}
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
            onChange={(e) => setQuery(e.target.value)}
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
      <div className="response-scroll">
        <JsonView
          text={text}
          highlight={pretty && !binary}
          lineNumbers={numbers}
          wrap={wrap}
          query={query}
          current={current}
        />
      </div>
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
  const failed = testsOf(run).filter((t) => !t.passed).length;

  const items: TabItem[] = [
    { id: "body", label: "Body" },
    { id: "headers", label: "Headers", count: headers.length },
    { id: "tests", label: "Test Results", count: failed, dot: failed > 0 },
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
            className={`chip tone-${statusTone((response.data as { status: number }).status)}`}
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
