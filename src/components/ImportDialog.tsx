import { useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  defaultWorkspaceDir,
  errorMessage,
  type ImportReport,
} from "../lib/api";
import { currentHost } from "../lib/host";
import {
  detectImportKind,
  IMPORT_KINDS,
  IMPORT_LABELS,
  type ImportKind,
} from "../lib/importKind";
import {
  documentFromFile,
  documentFromPath,
  documentFromText,
  documentFromUrl,
  type LoadedDocument,
} from "../lib/importSource";
import { formatBytes } from "../lib/format";
import { importAs } from "../lib/transfer";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { useModalGuard } from "../store/ui";
import { Close, Doc, Import, Warn } from "./Icons";

const FILE_FILTER = [
  { name: "Collection or specification", extensions: ["json", "yaml", "yml"] },
];

type Source = "url" | "file" | "text";

const SOURCE_LABELS: Record<Source, string> = {
  url: "From URL",
  file: "From file",
  text: "Paste",
};

function Report({
  report,
  kind,
  onClose,
}: {
  report: ImportReport;
  kind: ImportKind;
  onClose: () => void;
}) {
  return (
    <>
      <p>{report.summary}</p>
      <p className="report-counts">
        {report.imported} requests · {report.collections} collections ·{" "}
        {report.environments} environments · read as {IMPORT_LABELS[kind]}
      </p>
      {report.skipped.length > 0 && (
        <div className="report-section">
          <span className="field-label">Skipped</span>
          <ul className="report-list">
            {report.skipped.map((line, i) => (
              <li key={i}>{line}</li>
            ))}
          </ul>
        </div>
      )}
      {report.warnings.length > 0 && (
        <div className="report-section">
          <span className="field-label">Warnings</span>
          <ul className="report-list report-warnings">
            {report.warnings.map((line, i) => (
              <li key={i}>{line}</li>
            ))}
          </ul>
        </div>
      )}
      <div className="modal-actions">
        <button className="btn btn-primary" onClick={onClose}>
          Done
        </button>
      </div>
    </>
  );
}

export function ImportDialog({
  dropped,
  onClose,
}: {
  dropped: LoadedDocument | null;
  onClose: () => void;
}) {
  useModalGuard();
  const host = currentHost();
  const fileInput = useRef<HTMLInputElement>(null);

  const [source, setSource] = useState<Source>(dropped ? "file" : "url");
  const [url, setUrl] = useState("");
  const [pasted, setPasted] = useState("");
  const [doc, setDoc] = useState<LoadedDocument | null>(dropped);
  const [override, setOverride] = useState<ImportKind | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<{
    report: ImportReport;
    kind: ImportKind;
  } | null>(null);

  const detection = doc ? detectImportKind(doc.text) : null;
  const kind = override ?? detection?.kind ?? "postman";

  const load = async (note: string, run: () => Promise<LoadedDocument>) => {
    setBusy(note);
    setError(null);
    try {
      const loaded = await run();
      setDoc(loaded);
      setOverride(null);
    } catch (e) {
      setDoc(null);
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const onPick = async () => {
    if (host === "desktop") {
      const picked = await open({ multiple: false, filters: FILE_FILTER });
      if (typeof picked !== "string") return;
      await load(`Reading ${picked}…`, () => documentFromPath(picked));
      return;
    }
    fileInput.current?.click();
  };

  const doImport = async () => {
    if (!doc) return;
    setBusy(`Importing as ${IMPORT_LABELS[kind]}…`);
    setError(null);
    try {
      const workspace =
        useCollection.getState().workspace ?? (await defaultWorkspaceDir());
      const result = await importAs(workspace, kind, doc.text);
      await useCollection.getState().refreshTree();
      await useEnv.getState().init(workspace);
      setReport({ report: result, kind });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{report ? "Import complete" : "Import"}</h2>
          <button
            className="btn-ghost btn-icon"
            onClick={onClose}
            aria-label="Close"
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {report ? (
            <Report
              report={report.report}
              kind={report.kind}
              onClose={onClose}
            />
          ) : (
            <>
              {host === "browser" && (
                <div className="notice notice-wrap">
                  <Warn size={13} />
                  <span className="notice-text">
                    Importing needs the desktop app, which a web page cannot
                    load. Open this workspace in the desktop app to import.
                  </span>
                </div>
              )}
              <div className="segmented import-sources">
                {(Object.keys(SOURCE_LABELS) as Source[]).map((id) => (
                  <button
                    key={id}
                    className={`segment ${source === id ? "segment-active" : ""}`}
                    onClick={() => setSource(id)}
                  >
                    {SOURCE_LABELS[id]}
                  </button>
                ))}
              </div>

              {source === "url" && (
                <label className="field">
                  <span className="field-label">
                    OpenAPI or Postman document URL
                  </span>
                  <div className="import-url-row">
                    <input
                      className="input mono"
                      placeholder="https://example.com/openapi.json"
                      value={url}
                      onChange={(e) => setUrl(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key !== "Enter" || url.trim() === "") return;
                        e.preventDefault();
                        void load(`Fetching ${url.trim()}…`, () =>
                          documentFromUrl(url),
                        );
                      }}
                    />
                    <button
                      className="btn"
                      disabled={url.trim() === "" || busy !== null}
                      onClick={() =>
                        void load(`Fetching ${url.trim()}…`, () =>
                          documentFromUrl(url),
                        )
                      }
                    >
                      Fetch
                    </button>
                  </div>
                  <p className="import-hint">
                    {host === "browser"
                      ? "The page fetches this itself, so the server has to allow it with CORS headers. If it does not, download the file and drop it here."
                      : "Mándalo fetches the URL itself, so it goes through the same network checks as any request you send."}
                  </p>
                </label>
              )}

              {source === "file" && (
                <div className="import-drop">
                  <Doc size={22} />
                  <span className="import-drop-title">
                    Drop a file here, anywhere in the window
                  </span>
                  <span className="import-drop-hint">
                    Postman collection, OpenAPI specification (JSON or YAML), or
                    a Mándalo bundle.
                  </span>
                  <button className="btn" onClick={() => void onPick()}>
                    Choose file…
                  </button>
                  <input
                    ref={fileInput}
                    type="file"
                    accept=".json,.yaml,.yml"
                    hidden
                    onChange={(e) => {
                      const file = e.target.files?.item(0);
                      e.target.value = "";
                      if (!file) return;
                      void load(`Reading ${file.name}…`, () =>
                        documentFromFile(file),
                      );
                    }}
                  />
                </div>
              )}

              {source === "text" && (
                <label className="field">
                  <span className="field-label">Document</span>
                  <textarea
                    className="textarea mono import-paste"
                    placeholder="Paste a Postman collection, an OpenAPI specification, or a Mándalo bundle"
                    value={pasted}
                    onChange={(e) => setPasted(e.target.value)}
                  />
                  <button
                    className="btn import-read"
                    disabled={pasted.trim() === ""}
                    onClick={() =>
                      void load("Reading pasted document…", () =>
                        Promise.resolve(documentFromText(pasted)),
                      )
                    }
                  >
                    Read document
                  </button>
                </label>
              )}

              {busy && <p className="import-status">{busy}</p>}
              {error && (
                <div className="notice notice-error notice-wrap">
                  <Warn size={13} />
                  <span className="notice-text">{error}</span>
                </div>
              )}

              {doc && detection && (
                <div className="import-summary">
                  <div className="import-summary-head">
                    <span className="import-summary-name mono">{doc.name}</span>
                    <span className="import-summary-size">
                      {formatBytes(doc.bytes)}
                    </span>
                  </div>
                  <p className="import-hint">{detection.reason}</p>
                  <label className="field">
                    <span className="field-label">Import with</span>
                    <select
                      className="select"
                      value={kind}
                      onChange={(e) =>
                        setOverride(e.target.value as ImportKind)
                      }
                    >
                      {IMPORT_KINDS.map((id) => (
                        <option key={id} value={id}>
                          {IMPORT_LABELS[id]}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              )}

              <div className="modal-actions">
                <button className="btn-ghost" onClick={onClose}>
                  Cancel
                </button>
                <button
                  className="btn btn-primary"
                  disabled={!doc || busy !== null}
                  onClick={() => void doImport()}
                >
                  <Import size={13} />
                  Import
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
