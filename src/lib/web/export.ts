import { META, get, put } from "./idb";
import type { Vfs } from "./vfs";
import { zip, type ZipEntry } from "./zip";

const EXPORTED_KEY = "lastExportAt";

export async function collect(vfs: Vfs, dir = ""): Promise<ZipEntry[]> {
  const out: ZipEntry[] = [];
  for (const entry of await vfs.list(dir)) {
    const path = dir === "" ? entry.name : `${dir}/${entry.name}`;
    if (entry.dir) {
      out.push(...(await collect(vfs, path)));
      continue;
    }
    const text = await vfs.read(path);
    if (text !== null) out.push({ path, text });
  }
  return out;
}

export async function lastExportAt(): Promise<number | null> {
  return (await get<number>(META, EXPORTED_KEY)) ?? null;
}

export async function exportWorkspace(vfs: Vfs, name = "mandalo"): Promise<number> {
  const entries = await collect(vfs);
  if (entries.length === 0)
    throw new Error("There is nothing in this workspace to export yet");
  const blob = zip(entries);
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `${name}-${new Date().toISOString().slice(0, 10)}.zip`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
  await put(META, EXPORTED_KEY, Date.now());
  return entries.length;
}
