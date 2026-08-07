import { open } from "@tauri-apps/plugin-dialog";
import { currentHost } from "./host";

function isCancel(error: unknown): boolean {
  if (error === null || typeof error !== "object") return false;
  const name = "name" in error ? String(error.name) : "";
  const message = "message" in error ? String(error.message) : "";
  return (
    name === "AbortError" ||
    message.includes("abort") ||
    message.includes("cancel")
  );
}

function asPath(picked: string | string[] | null): string | null {
  if (picked === null) return null;
  if (Array.isArray(picked)) return picked[0] ?? null;
  return typeof picked === "string" ? picked : null;
}

/**
 * Open a directory picker that works from inside a dropdown on every host.
 *
 * Desktop: a still-open HTML menu can sit above the native dialog or eat the
 * click, so the menu is closed and we yield one tick before asking Tauri.
 * Browser: `showDirectoryPicker` needs the user gesture, so the picker starts
 * first and the menu closes after.
 */
export async function pickDirectory(options: {
  title?: string;
  closeMenu?: () => void;
}): Promise<string | null> {
  const desktop = currentHost() === "desktop";
  if (desktop) {
    options.closeMenu?.();
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  }
  try {
    const picked = await open({
      directory: true,
      multiple: false,
      title: options.title,
    });
    if (!desktop) options.closeMenu?.();
    return asPath(picked);
  } catch (error) {
    if (!desktop) options.closeMenu?.();
    if (isCancel(error)) return null;
    throw error;
  }
}
