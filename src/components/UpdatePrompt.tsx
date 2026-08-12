import { useCallback, useEffect, useState } from "react";
import {
  checkForUpdate,
  installAndRelaunch,
  updateSummary,
  updaterAvailable,
  type Update,
} from "../lib/updater";
import { errorMessage } from "../lib/api";
import { currentHost } from "../lib/host";
import { toast } from "../store/toast";
import { ConfirmModal } from "./ConfirmModal";

/**
 * Launch check; prompts only when a newer release is published.
 *
 * A failure here is reported rather than swallowed. Staying quiet made a broken
 * check look exactly like being up to date, which is the worst possible pair of
 * states to conflate: the one path by which every fix reaches a user can stop
 * working and nobody, including whoever is debugging it, has anything to go on.
 */
export function UpdatePrompt() {
  const [pending, setPending] = useState<Update | null>(null);

  /**
   * `announce` separates the two callers: the launch check stays quiet about
   * being current, while someone who just clicked the menu is owed an answer
   * either way.
   */
  const run = useCallback(async (announce: boolean) => {
    try {
      const update = await checkForUpdate();
      if (update) setPending(update);
      else if (announce) toast("success", "You’re on the latest version");
    } catch (e) {
      toast("error", `Could not check for updates: ${errorMessage(e)}`);
    }
  }, []);

  useEffect(() => {
    if (!updaterAvailable()) return;
    void run(false);
  }, [run]);

  // The macOS app menu is where people look for this, so it drives the same flow.
  useEffect(() => {
    if (currentHost() !== "desktop") return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen("menu://check-updates", () => void run(true)))
      .then(
        (stop) => {
          if (cancelled) stop();
          else unlisten = stop;
        },
        () => undefined,
      );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [run]);

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
