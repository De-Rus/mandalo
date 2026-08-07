import { useEffect, useState } from "react";
import { errorMessage } from "../lib/api";
import { currentHost } from "../lib/host";
import {
  documentFromFile,
  documentFromPath,
  type LoadedDocument,
} from "../lib/importSource";
import { toast } from "../store/toast";
import { useTransfer } from "../store/transfer";
import { Import } from "./Icons";

function carriesFiles(transfer: DataTransfer | null): boolean {
  return transfer !== null && Array.from(transfer.types).includes("Files");
}

/**
 * Two drop paths, because the desktop webview swallows HTML drag events and
 * reports the drop to Rust with a real path instead: the DOM path serves the
 * browser and the editor, `onDragDropEvent` serves the desktop app.
 */
export function DropTarget() {
  const openImport = useTransfer((s) => s.openImport);
  const [over, setOver] = useState(false);

  useEffect(() => {
    const accept = (load: () => Promise<LoadedDocument>) => {
      void load().then(
        (doc) => openImport(doc),
        (e) => toast("error", errorMessage(e)),
      );
    };

    let depth = 0;
    const onEnter = (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      depth += 1;
      setOver(true);
    };
    const onOver = (e: DragEvent) => {
      if (carriesFiles(e.dataTransfer)) e.preventDefault();
    };
    const onLeave = () => {
      depth = Math.max(0, depth - 1);
      if (depth === 0) setOver(false);
    };
    const onDrop = (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      depth = 0;
      setOver(false);
      const file = e.dataTransfer?.files.item(0);
      if (file) accept(() => documentFromFile(file));
    };

    window.addEventListener("dragenter", onEnter);
    window.addEventListener("dragover", onOver);
    window.addEventListener("dragleave", onLeave);
    window.addEventListener("drop", onDrop);

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    if (currentHost() === "desktop") {
      void import("@tauri-apps/api/webview")
        .then(({ getCurrentWebview }) =>
          getCurrentWebview().onDragDropEvent((event) => {
            const payload = event.payload;
            if (payload.type === "enter" || payload.type === "over") {
              setOver(true);
              return;
            }
            setOver(false);
            if (payload.type !== "drop") return;
            const path = payload.paths[0];
            if (path) accept(() => documentFromPath(path));
          }),
        )
        .then(
          (stop) => {
            if (cancelled) stop();
            else unlisten = stop;
          },
          () => undefined,
        );
    }

    return () => {
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDrop);
      cancelled = true;
      unlisten?.();
    };
  }, [openImport]);

  if (!over) return null;
  return (
    <div className="drop-overlay" role="status">
      <div className="drop-overlay-card">
        <Import size={22} />
        <span className="drop-overlay-title">Drop to import</span>
        <span className="drop-overlay-hint">
          Postman collection, OpenAPI specification, or a Mándalo bundle.
        </span>
      </div>
    </div>
  );
}
