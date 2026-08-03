import { openFolder } from "./mounts";

/**
 * The desktop app returns a path string here and then calls a workspace
 * command with it. In the browser the picker IS the mount, so opening the
 * folder happens inside open_workspace / create_workspace and this only has to
 * hand back a stable identifier for it.
 */
export async function open(options?: { directory?: boolean }): Promise<string> {
  if (!options?.directory)
    throw new Error(
      "Picking a file to import needs the desktop app. In the browser, use “Open folder…” to work on a real directory instead.",
    );
  return (await openFolder()).path;
}

export function save(): Promise<string | null> {
  return Promise.reject(
    new Error(
      "Saving a file to an arbitrary location needs the desktop app. In the browser, open a folder and Mándalo writes your collections straight into it.",
    ),
  );
}
