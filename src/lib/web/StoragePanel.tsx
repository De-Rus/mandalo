import { useCallback, useEffect, useState } from "react";
import { exportWorkspace, lastExportAt } from "./export";
import { activeVfs, openFolder, supportsFolders } from "./mounts";
import {
  formatBytes,
  requestPersistence,
  storageState,
  type StorageState,
} from "./storage";

export const COPY = {
  folder: {
    label: "Saved to your folder",
    body: "Your collections are real .http files in a folder you chose. Commit them to git and you always have a way back — nothing here depends on this browser.",
  },
  persisted: {
    label: "Stored in this browser · persistent",
    body: "The browser has agreed not to evict your collections to reclaim disk space. Clearing site data for this site still deletes them, so keep a copy elsewhere.",
  },
  "best-effort": {
    label: "Stored in this browser · can be evicted",
    body: "Unrequested storage is best-effort: the browser may delete your collections when the disk runs low, and “Clear site data” removes them without warning.",
  },
  denied: {
    label: "Stored in this browser · eviction not blocked",
    body: "The browser refused to mark this storage persistent, so your collections can still be evicted when space runs low. Opening a folder is the durable option — real files, in your own repo.",
  },
  unavailable: {
    label: "Stored in this browser · durability unknown",
    body: "This browser will not say whether your collections are safe from eviction, which usually means they are not. Treat browser storage as a scratchpad and keep a copy elsewhere.",
  },
} as const;

export const NO_FOLDERS =
  "Firefox and Safari have not shipped the File System Access API, so Mándalo cannot open a folder here. Download a copy regularly, or use the desktop app to work on real files.";

export const NEVER_EXPORTED =
  "This workspace has never been exported. If this browser loses its storage, it is gone. Download a copy, or move it into a folder you control.";

export const NEAR_QUOTA =
  "This origin is nearly out of storage. Saves will start failing rather than silently truncating — export a copy or move to a folder now.";

type Mode = keyof typeof COPY;

function modeOf(state: StorageState, denied: boolean): Mode {
  if (state.durability === "folder") return "folder";
  if (state.durability === "persisted") return "persisted";
  if (state.durability === "unavailable") return "unavailable";
  return denied ? "denied" : "best-effort";
}

export function StoragePanel() {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<StorageState | null>(null);
  const [denied, setDenied] = useState(false);
  const [exported, setExported] = useState<number | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const read = useCallback(async () => {
    try {
      const vfs = await activeVfs();
      setState(await storageState(vfs.kind));
      setExported(await lastExportAt());
    } catch {
      setState({
        durability: "unavailable",
        usage: null,
        quota: null,
        ratio: null,
        nearQuota: false,
        canPersist: false,
      });
    }
  }, []);

  useEffect(() => {
    void read();
  }, [read]);

  if (!state) return null;
  const mode = modeOf(state, denied);
  const copy = COPY[mode];

  const onPersist = async () => {
    const granted = await requestPersistence();
    setDenied(!granted);
    await read();
  };

  const onExport = async () => {
    setNote("Preparing…");
    try {
      const vfs = await activeVfs();
      const count = await exportWorkspace(vfs);
      setNote(`Downloaded ${count} file${count === 1 ? "" : "s"}`);
      await read();
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    }
  };

  const onOpenFolder = async () => {
    try {
      await openFolder();
      location.reload();
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    }
  };

  const nudge =
    mode !== "folder" && exported === null ? NEVER_EXPORTED : null;

  return (
    <div className="web-storage">
      <button
        className={`web-storage-chip web-storage-${mode}`}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {copy.label}
        {state.nearQuota && " · nearly full"}
      </button>

      {open && (
        <div className="web-storage-panel" role="dialog" aria-label="Where your work is stored">
          <p className="web-storage-body">{copy.body}</p>

          {state.usage !== null && state.quota !== null && (
            <p className="web-storage-quota">
              Using {formatBytes(state.usage)} of {formatBytes(state.quota)}
              {state.ratio !== null && ` (${Math.round(state.ratio * 100)}%)`}
            </p>
          )}
          {state.nearQuota && <p className="web-storage-warn">{NEAR_QUOTA}</p>}
          {nudge && <p className="web-storage-warn">{nudge}</p>}

          <div className="web-storage-actions">
            {supportsFolders() ? (
              <button className="web-storage-primary" onClick={() => void onOpenFolder()}>
                Open folder…
              </button>
            ) : (
              <p className="web-storage-body">{NO_FOLDERS}</p>
            )}
            <button className="web-storage-action" onClick={() => void onExport()}>
              Download a copy
            </button>
            {mode === "best-effort" && state.canPersist && (
              <button className="web-storage-action" onClick={() => void onPersist()}>
                Ask the browser to keep it
              </button>
            )}
          </div>
          {note && <p className="web-storage-note">{note}</p>}
        </div>
      )}
    </div>
  );
}
