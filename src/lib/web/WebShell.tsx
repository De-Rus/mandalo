import { useEffect, useRef, useState } from "react";
import App from "../../App";
import { dismiss, snapshot, subscribe, type Report } from "./bus";
import { DOWNLOAD_URL } from "./config";
import { activeVfs, reset, supportsFolders } from "./mounts";
import { PROTO_DIR, store } from "./protos";

function ProtoImport() {
  const input = useRef<HTMLInputElement>(null);
  const [note, setNote] = useState<string | null>(null);

  const onFiles = async (files: FileList | null) => {
    if (!files || files.length === 0) return;
    try {
      const vfs = await activeVfs();
      const saved: string[] = [];
      for (const file of Array.from(files))
        saved.push(await store(vfs, file.name, await file.text()));
      setNote(`Saved ${saved.join(", ")}`);
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <input
        ref={input}
        type="file"
        accept=".proto"
        multiple
        hidden
        onChange={(e) => {
          void onFiles(e.target.files);
          e.target.value = "";
        }}
      />
      <button
        className="web-ribbon-link"
        title={`gRPC requests are compiled in the browser from .proto files kept in the workspace's ${PROTO_DIR}/ folder`}
        onClick={() => input.current?.click()}
      >
        Proto files
      </button>
      {note && <span className="web-ribbon-note">{note}</span>}
    </>
  );
}

function Ribbon({ onReset }: { onReset: () => void }) {
  const folders = supportsFolders();
  return (
    <div className="web-ribbon">
      <span className="web-ribbon-tag">Web</span>
      <span className="web-ribbon-text">
        {folders
          ? "Mándalo in the browser. Open a folder to keep your collections as TOML in your own repo."
          : "Mándalo in the browser. Collections are stored in this browser; open a folder in a Chromium browser to keep them in your repo."}
      </span>
      <div className="web-ribbon-spacer" />
      <ProtoImport />
      <button className="web-ribbon-link" onClick={onReset}>
        Reset sample workspace
      </button>
      <a className="web-ribbon-cta" href={DOWNLOAD_URL}>
        Get the desktop app
      </a>
    </div>
  );
}

function Unreachable({ report }: { report: Report & { kind: "unreachable" } }) {
  return (
    <>
      <div className="web-card-head">
        <h2>
          The browser would not send this to <code>{report.host}</code>
        </h2>
        <button className="web-card-close" onClick={dismiss} aria-label="Dismiss">
          ✕
        </button>
      </div>
      {report.cors ? (
        <>
          <p>
            A page can only call another origin when that server opts in with
            CORS headers. When it doesn't, the browser blocks the response and —
            by design — refuses to tell scripts why. So this is one of exactly
            two things:
          </p>
          <ol className="web-card-list">
            <li>
              <strong>{report.host} did not send CORS headers.</strong>{" "}
              {report.preflight
                ? "This request needed a preflight (OPTIONS) because of its method or headers, and the server did not answer it."
                : "The server answered, but without Access-Control-Allow-Origin, so the browser discarded the response."}
            </li>
            <li>
              <strong>The host was genuinely unreachable</strong> — wrong URL,
              DNS failure, server down, or TLS refused.
            </li>
          </ol>
        </>
      ) : (
        <>
          <p>
            This call did not cross an origin, so CORS is not what stopped it.
            The browser rejected the request before it went anywhere:
          </p>
          <ol className="web-card-list">
            <li>
              <strong>The URL is not one a page can fetch</strong> — a missing
              or unsupported scheme, or an address the browser cannot parse.
            </li>
            <li>
              <strong>The host was genuinely unreachable</strong> — server down,
              TLS refused, or no network at all.
            </li>
          </ol>
        </>
      )}
      <p className="web-card-note">
        Either way the browser made the call, not Mándalo. The desktop app sends
        from a Rust HTTP client, where CORS does not exist: any host you can
        reach, including <code>localhost</code> and anything behind your VPN.
        We will not proxy your traffic through our servers to work around this —
        your requests and your tokens stay yours.
      </p>
      <div className="web-card-actions">
        <a className="web-card-cta" href={DOWNLOAD_URL}>
          Get the desktop app
        </a>
        <span className="web-card-hint">{report.detail}</span>
      </div>
    </>
  );
}

function Sent({ report }: { report: Report & { kind: "sent" } }) {
  return (
    <>
      <div className="web-card-head">
        <h2>
          {report.headerCount} response header
          {report.headerCount === 1 ? "" : "s"} readable
        </h2>
        <button className="web-card-close" onClick={dismiss} aria-label="Dismiss">
          ✕
        </button>
      </div>
      <p>
        {report.exposed
          ? `${report.host} sent Access-Control-Expose-Headers, so you can see more than the CORS defaults.`
          : `Browsers only let a page read the seven CORS-safelisted response headers unless the server sends Access-Control-Expose-Headers. ${report.host} did not, so the Headers tab shows what the browser exposed — not everything the server sent.`}
      </p>
      <p className="web-card-note">
        The status, body, timing and size are exactly what came back. The
        desktop app shows every header.
      </p>
    </>
  );
}

export function WebShell() {
  const [report, setReport] = useState<Report | null>(snapshot);
  const headerNoteShown = useRef(false);

  useEffect(
    () =>
      subscribe((next) => {
        if (next?.kind === "sent") {
          if (headerNoteShown.current) {
            setReport(null);
            return;
          }
          headerNoteShown.current = true;
        }
        setReport(next);
      }),
    [],
  );

  const onReset = () => {
    void reset().then(() => location.reload());
  };

  return (
    <div className="web-root">
      <Ribbon onReset={onReset} />
      <div className="web-app">
        <App />
      </div>
      {report && (
        <aside
          className={`web-card ${report.kind === "unreachable" ? "web-card-blocked" : ""}`}
          role="status"
        >
          {report.kind === "unreachable" ? (
            <Unreachable report={report} />
          ) : (
            <Sent report={report} />
          )}
        </aside>
      )}
    </div>
  );
}
