/** Folder name a `git clone` would create when no dest is given. */
export function repoFolderName(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "");
  const last = trimmed.split(/[/:]/).filter(Boolean).pop() ?? "workspace";
  const name = last.replace(/\.git$/i, "");
  return name === "" ? "workspace" : name;
}
