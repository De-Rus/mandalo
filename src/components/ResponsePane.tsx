import { useState } from "react";
import {
  bodyText,
  formatBytes,
  formatDuration,
  statusTone,
} from "../lib/format";
import type { ResponseState } from "../store/session";
import { Tabs } from "./Tabs";

function BodyView({ body, binary = false }: { body: string; binary?: boolean }) {
  return (
    <>
      {binary && (
        <p className="empty-line binary-note">
          Body is not valid UTF-8 — shown lossily.
        </p>
      )}
      <pre className="response-body mono">{bodyText(body, binary)}</pre>
    </>
  );
}

function HeadersView({ headers }: { headers: [string, string][] }) {
  if (headers.length === 0)
    return <p className="empty-line">No headers in the response.</p>;
  return (
    <table className="headers-table">
      <tbody>
        {headers.map(([k, v], i) => (
          <tr key={i}>
            <td className="mono header-key">{k}</td>
            <td className="mono">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function ResponsePane({ response }: { response: ResponseState }) {
  const [tab, setTab] = useState("Body");

  if (response.phase === "idle") {
    return (
      <section className="response-pane">
        <p className="empty-line pane-empty">
          Send a request and the response will show up here.
        </p>
      </section>
    );
  }

  if (response.phase === "loading") {
    return (
      <section className="response-pane">
        <div className="pane-empty loading-row">
          <span className="spinner" />
          <span>Sending request…</span>
        </div>
      </section>
    );
  }

  if (response.phase === "error") {
    return (
      <section className="response-pane">
        <div className="response-status">
          <span className="status-chip tone-error">Error</span>
        </div>
        <pre className="response-body mono response-error">{response.message}</pre>
      </section>
    );
  }

  if (response.phase === "grpc") {
    return (
      <section className="response-pane">
        <div className="response-status">
          <span className="status-chip tone-success">OK</span>
          <span className="status-meta">{formatDuration(response.data.durationMs)}</span>
        </div>
        <BodyView body={response.data.body} />
      </section>
    );
  }

  const { data } = response;
  return (
    <section className="response-pane">
      <div className="response-status">
        <span className={`status-chip tone-${statusTone(data.status)}`}>
          {data.status} {data.statusText}
        </span>
        <span className="status-meta">{formatDuration(data.durationMs)}</span>
        <span className="status-meta">{formatBytes(data.sizeBytes)}</span>
        {data.binary && <span className="status-chip tone-muted">binary</span>}
      </div>
      <Tabs tabs={["Body", "Headers"]} active={tab} onSelect={setTab} />
      <div className="response-content">
        {tab === "Body" ? (
          <BodyView body={data.body} binary={data.binary} />
        ) : (
          <HeadersView headers={data.headers} />
        )}
      </div>
    </section>
  );
}
