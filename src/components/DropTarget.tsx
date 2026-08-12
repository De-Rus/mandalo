import { useEffect, useState } from "react";
import { errorMessage } from "../lib/api";
import {
  documentFromFile,
  type LoadedDocument,
} from "../lib/importSource";
import { toast } from "../store/toast";
import { useTransfer } from "../store/transfer";
import { Import } from "./Icons";

function carriesFiles(transfer: DataTransfer | null): boolean {
  return transfer !== null && Array.from(transfer.types).includes("Files");
}

/**
 * One drop path for every host.
 *
 * The desktop used to take a second one: with the window's `dragDropEnabled`
 * on, the OS handler swallowed HTML drag events and handed Rust a real path
 * instead. That also made dragging anything *inside* the page impossible, which
 * is what moving a request between folders needs, so the window now turns it off
 * and every host reads the dropped file straight from the DOM. Nothing is lost —
 * both routes produced the same document, and this one also enforces the import
 * size limit. A file picked through the dialog still arrives as a path; that is
 * `ImportDialog`, not this.
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

    return () => {
      window.removeEventListener("dragenter", onEnter);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("dragleave", onLeave);
      window.removeEventListener("drop", onDrop);
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
