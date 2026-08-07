export type Durability = "folder" | "persisted" | "best-effort" | "unavailable";

export interface StorageState {
  durability: Durability;
  usage: number | null;
  quota: number | null;
  ratio: number | null;
  nearQuota: boolean;
  canPersist: boolean;
}

const NEAR_QUOTA = 0.8;

function manager(): StorageManager | null {
  if (typeof navigator === "undefined") return null;
  return (navigator as Navigator & { storage?: StorageManager }).storage ?? null;
}

export function canPersist(): boolean {
  return typeof manager()?.persist === "function";
}

export async function persisted(): Promise<boolean> {
  const storage = manager();
  if (typeof storage?.persisted !== "function") return false;
  try {
    return await storage.persisted();
  } catch {
    return false;
  }
}

/**
 * Chromium grants this silently on an engaged origin and Firefox prompts, so it must
 * be called from something the user just did — never on first paint, where it is a
 * prompt for a decision they have no context for yet.
 */
export async function requestPersistence(): Promise<boolean> {
  const storage = manager();
  if (typeof storage?.persist !== "function") return false;
  try {
    return await storage.persist();
  } catch {
    return false;
  }
}

let asked = false;

/** Saving real work is the engagement signal; first paint is not. */
export async function persistOnFirstSave(): Promise<void> {
  if (asked || !canPersist()) return;
  asked = true;
  if (await persisted()) return;
  await requestPersistence();
}

async function estimate(): Promise<{ usage: number | null; quota: number | null }> {
  const storage = manager();
  if (typeof storage?.estimate !== "function") return { usage: null, quota: null };
  try {
    const { usage, quota } = await storage.estimate();
    return { usage: usage ?? null, quota: quota ?? null };
  } catch {
    return { usage: null, quota: null };
  }
}

export async function storageState(kind: "browser" | "folder"): Promise<StorageState> {
  const { usage, quota } = await estimate();
  const ratio = usage !== null && quota !== null && quota > 0 ? usage / quota : null;
  const durability: Durability =
    kind === "folder"
      ? "folder"
      : !canPersist()
        ? "unavailable"
        : (await persisted())
          ? "persisted"
          : "best-effort";
  return {
    durability,
    usage,
    quota,
    ratio,
    nearQuota: ratio !== null && ratio >= NEAR_QUOTA,
    canPersist: canPersist(),
  };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
