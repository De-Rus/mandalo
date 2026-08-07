import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { currentHost } from "./host";

export type { Update };

export function updaterAvailable(): boolean {
  return currentHost() === "desktop";
}

/** Returns a pending update, or null when already current / not on desktop. */
export async function checkForUpdate(): Promise<Update | null> {
  if (!updaterAvailable()) return null;
  return check();
}

export async function installAndRelaunch(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}

export function updateSummary(update: Update): string {
  const notes = (update.body ?? "").trim();
  if (notes === "") return `Version ${update.version} is ready to install.`;
  return `Version ${update.version} is ready to install.\n\n${notes}`;
}
