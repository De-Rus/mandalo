import { useEffect, useState } from "react";
import {
  checkForUpdate,
  installAndRelaunch,
  updateSummary,
  updaterAvailable,
  type Update,
} from "../lib/updater";
import { errorMessage } from "../lib/api";
import { toast } from "../store/toast";
import { ConfirmModal } from "./ConfirmModal";

/** Silent launch check; prompts only when a newer release is published. */
export function UpdatePrompt() {
  const [pending, setPending] = useState<Update | null>(null);

  useEffect(() => {
    if (!updaterAvailable()) return;
    let cancelled = false;
    void checkForUpdate()
      .then((update) => {
        if (!cancelled && update) setPending(update);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  if (pending === null) return null;

  return (
    <ConfirmModal
      title="Update available"
      message={updateSummary(pending)}
      confirmLabel="Install & restart"
      tone="primary"
      onConfirm={() => {
        const update = pending;
        setPending(null);
        toast("info", "Downloading update…");
        void installAndRelaunch(update).catch((e) =>
          toast("error", errorMessage(e)),
        );
      }}
      onClose={() => setPending(null)}
    />
  );
}
