import { useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  defaultWorkspaceDir,
  errorMessage,
  exportBundle,
  readTextFileForImport,
  writeTextFileForExport,
  type ExportBundle,
  type ImportReport,
} from "../lib/api";
import { importFromText } from "../lib/transfer";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { Dropdown, MenuItem } from "./Dropdown";
import { Close, Dots, Export, Import, Warn } from "./Icons";

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
          <button
            className="btn-ghost btn-icon"
            onClick={onClose}
            aria-label="Close"
          >
            <Close size={13} />
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

function ExportReviewModal({
  bundle,
  onCancel,
  onConfirm,
}: {
  bundle: ExportBundle;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  useModalGuard();
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Review before exporting</h2>
          <button
            className="btn-ghost btn-icon"
            onClick={onCancel}
            aria-label="Close"
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          <div className="notice notice-error">
            <Warn size={13} />
            <span className="notice-text">
              {bundle.findings.length} value(s) in this workspace look like
              credentials. Anything you export leaves this machine — check each
              line before you write the file.
            </span>
          </div>
          <ul className="report-list report-findings">
            {bundle.findings.map((finding, i) => (
              <li key={i}>
                <span className="finding-rule">{finding.rule}</span>
                <span className="finding-path mono">
                  {finding.path}:{finding.line}
                </span>
                <span className="finding-excerpt mono">{finding.excerpt}</span>
              </li>
            ))}
          </ul>
          <div className="modal-actions">
            <button className="btn-ghost" onClick={onCancel}>
              Cancel
            </button>
            <button className="btn btn-primary" onClick={onConfirm}>
              Export anyway
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function TransferMenu() {
  const [report, setReport] = useState<ImportReport | null>(null);
  const [pending, setPending] = useState<ExportBundle | null>(null);

  const workspaceDir = () =>
    useCollection.getState().workspace ?? defaultWorkspaceDir();

  const doImport = async () => {
    try {
      const file = await open({ multiple: false, filters: JSON_FILTER });
      if (typeof file !== "string") return;
      const json = await readTextFileForImport(file);
      const workspace = await workspaceDir();
      const result = await importFromText(workspace, json);
      await useCollection.getState().refreshTree();
      await useEnv.getState().init(workspace);
      setReport(result);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  const writeBundle = async (json: string) => {
    const path = await save({
      defaultPath: "mandalo-bundle.json",
      filters: JSON_FILTER,
    });
    if (!path) return;
    await writeTextFileForExport(path, json);
    toast("success", "Bundle saved");
  };

  const doExport = async () => {
    try {
      const workspace = await workspaceDir();
      const bundle = await exportBundle(workspace);
      if (bundle.findings.length > 0) {
        setPending(bundle);
        return;
      }
      await writeBundle(bundle.json);
    } catch (e) {
      toast("error", errorMessage(e));
    }
  };

  return (
    <>
      <Dropdown
        align="right"
        trigger={({ open: isOpen, toggle }) => (
          <button
            className={`btn-ghost btn-icon ${isOpen ? "menu-item-active" : ""}`}
            title="Import / Export"
            aria-label="Import / Export"
            onClick={toggle}
          >
            <Dots size={14} />
          </button>
        )}
      >
        {(close) => (
          <>
            <MenuItem
              icon={<Import size={13} />}
              onClick={() => {
                close();
                void doImport();
              }}
            >
              Import…
            </MenuItem>
            <MenuItem
              icon={<Export size={13} />}
              onClick={() => {
                close();
                void doExport();
              }}
            >
              Export bundle…
            </MenuItem>
          </>
        )}
      </Dropdown>
      {report && (
        <ImportReportModal report={report} onClose={() => setReport(null)} />
      )}
      {pending && (
        <ExportReviewModal
          bundle={pending}
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const json = pending.json;
            setPending(null);
            void writeBundle(json).catch((e) => toast("error", errorMessage(e)));
          }}
        />
      )}
    </>
  );
}
