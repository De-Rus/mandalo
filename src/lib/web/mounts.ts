import type { WorkspaceInfo } from "../api";
import { DEFAULT_ENV_NAME } from "./config";
import { HANDLES, META, del, entries, get, put } from "./idb";
import { seedFiles } from "./seed";
import { BrowserVfs, FolderVfs, type Vfs } from "./vfs";
import { ensureWorkspace, workspaceName } from "./workspace";

export const BROWSER_PATH = "browser://Browser storage";
const SEEDED_KEY = "seeded";
const ACTIVE_KEY = "activeWorkspace";

export function folderPath(key: string, name: string): string {
  return `folder://${key}/${name}`;
}

export function displayName(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

export function supportsFolders(): boolean {
  return typeof (globalThis as { showDirectoryPicker?: unknown })
    .showDirectoryPicker === "function";
}

const mounted = new Map<string, Vfs>();

function browserVfs(): Vfs {
  const existing = mounted.get(BROWSER_PATH);
  if (existing) return existing;
  const created = new BrowserVfs("browser");
  mounted.set(BROWSER_PATH, created);
  return created;
}

async function folderHandles(): Promise<[string, FileSystemDirectoryHandle][]> {
  return entries<FileSystemDirectoryHandle>(HANDLES);
}

export async function seedIfEmpty(): Promise<void> {
  if (await get<boolean>(META, SEEDED_KEY)) return;
  const vfs = browserVfs();
  for (const [path, text] of seedFiles()) await vfs.write(path, text);
  await put(META, SEEDED_KEY, true);
}

export async function list(): Promise<{
  items: WorkspaceInfo[];
  active: string;
}> {
  const items: WorkspaceInfo[] = [
    {
      id: "browser",
      path: BROWSER_PATH,
      name: (await workspaceName(browserVfs())) ?? "Browser storage",
    },
  ];
  for (const [key, handle] of await folderHandles()) {
    const path = folderPath(key, handle.name);
    mounted.set(path, new FolderVfs(key, handle));
    items.push({ id: `folder:${key}`, path, name: handle.name });
  }
  const stored = await get<string>(META, ACTIVE_KEY);
  const active = items.some((w) => w.id === stored) ? (stored as string) : "browser";
  return { items, active };
}

export async function activeVfs(): Promise<Vfs> {
  const { items, active } = await list();
  const found = items.find((w) => w.id === active);
  if (!found) throw new Error(`unknown workspace: ${active}`);
  return vfsFor(found.path);
}

export async function setActive(id: string): Promise<WorkspaceInfo> {
  const { items } = await list();
  const found = items.find((w) => w.id === id);
  if (!found) throw new Error(`unknown workspace: ${id}`);
  await put(META, ACTIVE_KEY, id);
  return found;
}

export async function forget(id: string): Promise<void> {
  if (id === "browser")
    throw new Error("Browser storage is always available and cannot be removed");
  await del(HANDLES, id.replace(/^folder:/, ""));
  const stored = await get<string>(META, ACTIVE_KEY);
  if (stored === id) await put(META, ACTIVE_KEY, "browser");
}

export function vfsFor(path: string): Vfs {
  const found = mounted.get(path);
  if (!found)
    throw new Error(
      `Workspace "${displayName(path)}" is not open. Reopen the folder to grant access again.`,
    );
  return found;
}

async function grant(handle: FileSystemDirectoryHandle): Promise<boolean> {
  const options = { mode: "readwrite" } as FileSystemHandlePermissionDescriptor;
  if ((await handle.queryPermission(options)) === "granted") return true;
  return (await handle.requestPermission(options)) === "granted";
}

/**
 * The picker must run inside the click that opened it, so the folder is mounted
 * by the dialog shim and claimed a moment later by open_workspace.
 */
const pending = new Map<string, WorkspaceInfo>();

export function claim(path: string): WorkspaceInfo | null {
  const found = pending.get(path) ?? null;
  pending.delete(path);
  return found;
}

export async function openFolder(): Promise<WorkspaceInfo> {
  if (!supportsFolders())
    throw new Error(
      "This browser cannot open a local folder. Chrome, Edge and other Chromium browsers can; Firefox and Safari have not shipped the File System Access API.",
    );
  const picker = (
    globalThis as unknown as {
      showDirectoryPicker: (o: {
        mode: string;
        id: string;
      }) => Promise<FileSystemDirectoryHandle>;
    }
  ).showDirectoryPicker;
  const handle = await picker({ mode: "readwrite", id: "mandalo-workspace" });
  if (!(await grant(handle)))
    throw new Error("Mándalo needs read and write access to that folder");

  const key = `${Date.now().toString(36)}-${handle.name}`;
  await put(HANDLES, key, handle);
  const vfs = new FolderVfs(key, handle);
  const path = folderPath(key, handle.name);
  mounted.set(path, vfs);
  await ensureWorkspace(vfs, handle.name);
  await put(META, ACTIVE_KEY, `folder:${key}`);
  const info = { id: `folder:${key}`, path, name: handle.name };
  pending.set(path, info);
  return info;
}

export async function reopen(id: string): Promise<boolean> {
  const key = id.replace(/^folder:/, "");
  const handle = await get<FileSystemDirectoryHandle>(HANDLES, key);
  if (!handle) return false;
  if (!(await grant(handle))) return false;
  mounted.set(folderPath(key, handle.name), new FolderVfs(key, handle));
  return true;
}

export async function reset(): Promise<void> {
  const vfs = browserVfs();
  await vfs.removeDir("");
  await del(META, SEEDED_KEY);
  localStorage.removeItem("mandalo.collection.active");
  localStorage.removeItem("mandalo.tabs.v1");
  localStorage.setItem("mandalo.env.selected", DEFAULT_ENV_NAME);
  await seedIfEmpty();
}
