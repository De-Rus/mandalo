import { useState } from "react";
import { retryPendingSave, useCollection } from "../../store/collection";
import { looksLapsed, reconnectActive } from "./mounts";

function SaveFailure({ message }: { message: string }) {
  const [busy, setBusy] = useState(false);
  const lapsed = looksLapsed(message);

  const onRetry = async () => {
    setBusy(true);
    try {
      if (lapsed && !(await reconnectActive())) return;
      await retryPendingSave();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="web-notice web-notice-danger" role="alert">
      <div className="web-notice-body">
        <strong>Your last change was not saved.</strong>
        <span>{message}</span>
      </div>
      <button className="web-notice-action" disabled={busy} onClick={() => void onRetry()}>
        {lapsed ? "Reconnect folder" : "Try again"}
      </button>
    </div>
  );
}

function Conflict({ id }: { id: string }) {
  const takeTheirs = useCollection((s) => s.takeTheirs);
  const keepMine = useCollection((s) => s.keepMine);

  return (
    <div className="web-notice web-notice-warn" role="alert">
      <div className="web-notice-body">
        <strong>This request changed in another tab.</strong>
        <span>
          You have unsaved edits here, so nothing was replaced. Load the other tab's
          version, or keep yours and overwrite it on the next save.
        </span>
      </div>
      <button className="web-notice-action" onClick={() => void takeTheirs(id)}>
        Load theirs
      </button>
      <button className="web-notice-action" onClick={() => keepMine(id)}>
        Keep mine
      </button>
    </div>
  );
}

function Vanished() {
  return (
    <div className="web-notice web-notice-warn" role="alert">
      <div className="web-notice-body">
        <strong>This request was deleted in another tab.</strong>
        <span>
          What you see here is your copy. Saving it will write the request back.
        </span>
      </div>
    </div>
  );
}

export function Notices() {
  const saveError = useCollection((s) => s.saveError);
  const activeId = useCollection((s) => s.activeId);
  const conflicts = useCollection((s) => s.conflicts);
  const vanished = useCollection((s) => s.vanished);

  const conflicted = activeId !== null && conflicts.includes(activeId);
  const deleted = activeId !== null && vanished.includes(activeId);
  if (!saveError && !conflicted && !deleted) return null;

  return (
    <div className="web-notices">
      {saveError && <SaveFailure message={saveError} />}
      {conflicted && activeId && <Conflict id={activeId} />}
      {deleted && <Vanished />}
    </div>
  );
}
