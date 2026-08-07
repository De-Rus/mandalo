import { useState } from "react";
import { emptyFormDataRow, type FormDataRowDraft } from "../lib/draft";
import { useModalGuard } from "../store/ui";
import { Close } from "./Icons";

function relativize(path: string, root: string | null): string | null {
  if (root === null) return null;
  const base = root.endsWith("/") || root.endsWith("\\") ? root : `${root}/`;
  if (path.startsWith(base)) return path.slice(base.length);
  const backslashed = base.replace(/\//g, "\\");
  if (path.startsWith(backslashed)) return path.slice(backslashed.length).replace(/\\/g, "/");
  return null;
}

function fileLabel(path: string): string {
  const parts = path.split(/[/\\]/).filter((part) => part !== "");
  return parts[parts.length - 1] ?? path;
}

interface FormDataEditorProps {
  rows: FormDataRowDraft[];
  onChange: (rows: FormDataRowDraft[]) => void;
  workspaceRoot: string | null;
}

interface FilesModalProps {
  fieldKey: string;
  files: string[];
  onRemove: (fileIndex: number) => void;
  onClear: () => void;
  onAdd: () => void;
  onClose: () => void;
}

function FilesModal({ fieldKey, files, onRemove, onClear, onAdd, onClose }: FilesModalProps) {
  useModalGuard();
  const title = fieldKey.trim() === "" ? "Files" : fieldKey;
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal fd-files-modal"
        role="dialog"
        aria-label={`${title} files`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{title}</h2>
          <button type="button" className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {files.length === 0 ? (
            <p className="empty-line">No files yet. Add one or more workspace files.</p>
          ) : (
            <ul className="fd-files-modal-list">
              {files.map((path, fileIndex) => (
                <li className="fd-files-modal-row" key={`${path}-${fileIndex}`}>
                  <span className="fd-files-modal-path mono" title={path}>
                    {path}
                  </span>
                  <button
                    type="button"
                    className="btn-ghost btn-icon"
                    aria-label={`Remove ${path}`}
                    title="Remove"
                    onClick={() => onRemove(fileIndex)}
                  >
                    <Close size={12} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="modal-actions">
            {files.length > 0 && (
              <button type="button" className="btn" onClick={onClear}>
                Remove all
              </button>
            )}
            <span className="header-spacer" />
            <button type="button" className="btn" onClick={onClose}>
              Done
            </button>
            <button type="button" className="btn btn-primary" onClick={onAdd}>
              Add files…
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function FormDataEditor({ rows, onChange, workspaceRoot }: FormDataEditorProps) {
  const [pickError, setPickError] = useState<string | null>(null);
  const [manageIndex, setManageIndex] = useState<number | null>(null);

  const edit = (index: number, patch: Partial<FormDataRowDraft>) => {
    const next = rows.map((row, i) => (i === index ? { ...row, ...patch } : row));
    const last = next[next.length - 1]!;
    if (last.key !== "" || last.value !== "" || last.files.length > 0)
      next.push(emptyFormDataRow());
    onChange(next);
  };

  const remove = (index: number) => {
    const next = rows.filter((_, i) => i !== index);
    onChange(next.length > 0 ? next : [emptyFormDataRow()]);
  };

  const addFiles = (index: number, paths: string[]) => {
    if (paths.length === 0) return;
    const row = rows[index]!;
    const seen = new Set(row.files);
    const extra = paths.filter((path) => {
      if (seen.has(path)) return false;
      seen.add(path);
      return true;
    });
    if (extra.length > 0) edit(index, { files: [...row.files, ...extra] });
  };

  const removeFile = (index: number, fileIndex: number) => {
    const row = rows[index]!;
    edit(index, { files: row.files.filter((_, i) => i !== fileIndex) });
  };

  const browse = async (index: number) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: true,
        title: "Pick files inside the workspace",
        defaultPath: workspaceRoot ?? undefined,
      });
      if (picked === null) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      const inside: string[] = [];
      const outside: string[] = [];
      for (const path of paths) {
        const rel = relativize(path, workspaceRoot);
        if (rel === null) outside.push(path);
        else inside.push(rel);
      }
      setPickError(
        outside.length === 0
          ? null
          : `Files must live inside the workspace (${workspaceRoot ?? "?"}) so the collection stays shareable — copy them in first: ${outside.join(", ")}`,
      );
      addFiles(index, inside);
    } catch {
      setPickError(
        "File picker is unavailable here — open this request in the desktop app to attach workspace files.",
      );
    }
  };

  const managed = manageIndex !== null ? rows[manageIndex] : null;

  return (
    <div className="kv fd">
      <div className="kv-head fd-head">
        <span />
        <span>Key</span>
        <span>Type</span>
        <span>Value</span>
        <span />
      </div>
      {rows.map((row, i) => (
        <div className={`kv-row fd-row ${row.enabled ? "" : "kv-row-off"}`} key={row.id}>
          <span className="kv-check-cell">
            <input
              type="checkbox"
              className="checkbox"
              aria-label="Enable field"
              checked={row.enabled}
              onChange={(e) => edit(i, { enabled: e.target.checked })}
            />
          </span>
          <input
            className="kv-input"
            aria-label="Field name"
            placeholder="field"
            spellCheck={false}
            value={row.key}
            onChange={(e) => edit(i, { key: e.target.value })}
          />
          <select
            className="select fd-kind"
            aria-label="Field type"
            value={row.kind}
            onChange={(e) => edit(i, { kind: e.target.value as FormDataRowDraft["kind"] })}
          >
            <option value="text">Text</option>
            <option value="file">File</option>
          </select>
          {row.kind === "text" ? (
            <input
              className="kv-input"
              aria-label="Field value"
              placeholder="value"
              spellCheck={false}
              value={row.value}
              onChange={(e) => edit(i, { value: e.target.value })}
            />
          ) : (
            <span className="fd-file-cell">
              <button
                type="button"
                className={`fd-file-summary ${row.files.length === 0 ? "fd-file-summary-empty" : ""}`}
                aria-label="Field files"
                title={
                  row.files.length === 0
                    ? "No files yet"
                    : `${row.files.length} file${row.files.length === 1 ? "" : "s"} — click to manage`
                }
                onClick={() => setManageIndex(i)}
              >
                {row.files.length === 0 ? (
                  <span className="fd-file-empty">No files</span>
                ) : (
                  <span className="fd-file-list">
                    {row.files.slice(0, 2).map((path, fileIndex) => (
                      <span className="fd-file-chip" key={`${path}-${fileIndex}`}>
                        <span className="fd-file-path" title={path}>
                          {fileLabel(path)}
                        </span>
                      </span>
                    ))}
                    {row.files.length > 2 && (
                      <span className="fd-file-more">+{row.files.length - 2}</span>
                    )}
                  </span>
                )}
              </button>
              <button
                type="button"
                className="btn btn-sm fd-file-browse"
                onClick={() => void browse(i)}
              >
                Add files…
              </button>
              <input
                className="kv-input fd-ct"
                aria-label="Part content type"
                placeholder="auto"
                spellCheck={false}
                title={
                  row.contentType.trim() === ""
                    ? "Content type — leave empty to sniff from the file extension"
                    : row.contentType
                }
                value={row.contentType}
                onChange={(e) => edit(i, { contentType: e.target.value })}
              />
            </span>
          )}
          <button
            className="btn-ghost btn-icon kv-del"
            aria-label="Delete field"
            title="Delete field"
            onClick={() => remove(i)}
          >
            <Close size={12} />
          </button>
        </div>
      ))}
      {pickError !== null && <div className="fd-error">{pickError}</div>}
      <p className="fd-hint">
        Sent as multipart/form-data. Several files under one key post as separate parts sharing
        that name. Click the files to manage them; paths stay workspace-relative.
      </p>
      {managed !== undefined && managed !== null && manageIndex !== null && (
        <FilesModal
          fieldKey={managed.key}
          files={managed.files}
          onRemove={(fileIndex) => removeFile(manageIndex, fileIndex)}
          onClear={() => edit(manageIndex, { files: [] })}
          onAdd={() => void browse(manageIndex)}
          onClose={() => setManageIndex(null)}
        />
      )}
    </div>
  );
}
