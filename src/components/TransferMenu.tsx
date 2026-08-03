import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  defaultWorkspaceDir,
  errorMessage,
  exportBundle,
  readTextFileForImport,
  writeTextFileForExport,
  type ImportReport,
} from "../lib/api";
import { importFromText } from "../lib/transfer";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { useModalGuard } from "../store/ui";

const JSON_FILTER = [{ name: "JSON", extensions: ["json"] }];

function ImportReportModal({
  report,
  onClose,
}: {
  report: ImportReport;
  onClose: () => void;
}) {
  useModalGuard();
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Import complete</h2>
          <button className="btn-ghost" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>
        <div className="modal-body">
          <p>{report.summary}</p>
          <p className="report-counts">
            {report.imported} requests · {report.environments} environments
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
        </div>
      </div>
    </div>
  );
}

export function TransferMenu() {
  const [menuOpen, setMenuOpen] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node))
        setMenuOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 2500);
    return () => clearTimeout(timer);
  }, [notice]);

  const workspaceDir = () =>
    useCollection.getState().workspace ?? defaultWorkspaceDir();

  const doImport = async () => {
    setMenuOpen(false);
    setError(null);
    try {
      const file = await open({ multiple: false, filters: JSON_FILTER });
      if (typeof file !== "string") return;
      const json = await readTextFileForImport(file);
      const workspace = await workspaceDir();
      const result = await importFromText(workspace, json);
      await useCollection.getState().reload();
      await useEnv.getState().init();
      setReport(result);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const doExport = async () => {
    setMenuOpen(false);
    setError(null);
    try {
      const workspace = await workspaceDir();
      const json = await exportBundle(workspace);
      const path = await save({
        defaultPath: "mandalo-bundle.json",
        filters: JSON_FILTER,
      });
      if (!path) return;
      await writeTextFileForExport(path, json);
      setNotice("Bundle saved");
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className="transfer" ref={wrapRef}>
      <button
        className="btn-ghost"
        title="Import / Export"
        aria-label="Import / Export"
        onClick={() => setMenuOpen((v) => !v)}
      >
        ⋯
      </button>
      {menuOpen && (
        <div className="menu">
          <button className="menu-item" onClick={() => void doImport()}>
            Import…
          </button>
          <button className="menu-item" onClick={() => void doExport()}>
            Export bundle…
          </button>
        </div>
      )}
      {notice && <div className="menu-notice">{notice}</div>}
      {error && (
        <div className="menu-notice menu-notice-error" title={error}>
          {error}
        </div>
      )}
      {report && (
        <ImportReportModal report={report} onClose={() => setReport(null)} />
      )}
    </div>
  );
}
