import type { RemoteOrigin, WorkspaceInfo } from "../api";
import { HANDLES, META, del, entries, get, put } from "./idb";
import { materialize, readOrigin, stampOrigin, type RemoteFetch } from "./remote";
import { seedFiles } from "./seed";
import { BrowserVfs, FolderVfs, type Vfs } from "./vfs";
import { ensureWorkspace, workspaceName } from "./workspace";

export const BROWSER_PATH = "browser://Browser storage";
const SEEDED_KEY = "seeded";
const ACTIVE_KEY = "activeWorkspace";
const REMOTES_KEY = "remoteWorkspaces";

interface RemoteMount {
  id: string;
  name: string;
}

async function remoteMounts(): Promise<RemoteMount[]> {
  return (await get<RemoteMount[]>(META, REMOTES_KEY)) ?? [];
}

export function remotePath(id: string, name: string): string {
  return `remote://${id}/${name}`;
}

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
  for (const mount of await remoteMounts()) {
    const path = remotePath(mount.id, mount.name);
    mounted.set(path, new BrowserVfs(mount.id));
    items.push({ id: mount.id, path, name: mount.name });
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
  const remotes = await remoteMounts();
  if (remotes.some((m) => m.id === id)) {
    const vfs = new BrowserVfs(id);
    await vfs.removeDir("");
    await put(
      META,
      REMOTES_KEY,
      remotes.filter((m) => m.id !== id),
    );
    mounted.delete(remotePath(id, ""));
    for (const key of [...mounted.keys()])
      if (key.startsWith(`remote://${id}/`)) mounted.delete(key);
    const stored = await get<string>(META, ACTIVE_KEY);
    if (stored === id) await put(META, ACTIVE_KEY, "browser");
    return;
  }
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

/**
 * Permission to a folder lapses when the browser restarts, and re-granting it needs a
 * user gesture — so a write cannot quietly re-prompt. It fails with this instead, and
 * the shell turns it into a Reconnect button the user can actually click.
 */
export const LAPSED_MARKER = "no longer has permission to write to";

export class FolderAccessLapsed extends Error {
  constructor(readonly workspaceId: string, readonly folder: string) {
    super(
      `Mándalo ${LAPSED_MARKER} “${folder}”. The browser drops folder access when it restarts. Reconnect the folder to keep saving — nothing in it has been changed.`,
    );
    this.name = "FolderAccessLapsed";
  }
}

export function looksLapsed(message: string | null): boolean {
  return message !== null && message.includes(LAPSED_MARKER);
}

export async function reconnectActive(): Promise<boolean> {
  const { active } = await list();
  return active === "browser" ? true : reopen(active);
}

export async function ensureGranted(
  handle: FileSystemDirectoryHandle,
  workspaceId: string,
): Promise<void> {
  const state = await handle.queryPermission({
    mode: "readwrite",
  } as FileSystemHandlePermissionDescriptor);
  if (state !== "granted") throw new FolderAccessLapsed(workspaceId, handle.name);
}

export async function assertWritable(path: string): Promise<void> {
  const vfs = vfsFor(path);
  const origin = await readOrigin(vfs);
  if (origin !== null)
    throw new ReadOnlyWorkspace(
      `This workspace is a read-only copy of ${origin.label}. Save a copy of it to make changes.`,
    );
  if (vfs.kind !== "folder") return;
  const handle = await get<FileSystemDirectoryHandle>(HANDLES, vfs.id);
  if (handle) await ensureGranted(handle, `folder:${vfs.id}`);
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

const SAMPLE_SLUG = "mock";

/**
 * Copies the shipped sample collection into a workspace, under a free slug, without
 * touching anything already there — a user who deleted it, or who is working in their
 * own folder, can always get a collection back to send from. Supporting files the
 * sample's requests point at (`protos/`, `files/`) come with it; an environment is
 * written only when the workspace has none by that name.
 */
export async function addSampleCollection(vfs: Vfs): Promise<string> {
  await ensureWorkspace(vfs, "Mándalo");
  const taken = new Set((await vfs.list("collections")).map((entry) => entry.name));
  let slug = SAMPLE_SLUG;
  for (let n = 2; taken.has(slug); n += 1) slug = `${SAMPLE_SLUG}-${n}`;

  const prefix = `collections/${SAMPLE_SLUG}/`;
  let copied = 0;
  for (const [path, text] of seedFiles()) {
    if (path.startsWith(prefix)) {
      await vfs.write(`collections/${slug}/${path.slice(prefix.length)}`, text);
      copied += 1;
      continue;
    }
    if (path.startsWith("collections/") || path === "mandalo.toml") continue;
    if ((await vfs.read(path)) === null) await vfs.write(path, text);
  }
  if (copied === 0)
    throw new Error(
      "the sample collection is missing from this build: examples/mock-workspace shipped no requests",
    );
  return slug;
}


/**
 * A workspace that arrived from a link is somebody else's. Editing it in place
 * would quietly diverge from the thing the link points at, and the user never
 * asked to own it — so every write is refused until they say they want it.
 */
export class ReadOnlyWorkspace extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ReadOnlyWorkspace";
  }
}

/**
 * Writes a reviewed remote collection into browser storage of its own, stamped
 * with where it came from. Nothing in it has run and nothing in it can be
 * written; it is a copy to read and send from.
 */
export async function openRemote(fetched: RemoteFetch): Promise<WorkspaceInfo> {
  const id = `remote-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const vfs = new BrowserVfs(id);
  await materialize(fetched, vfs);
  await stampOrigin(vfs, fetched.origin);
  const name = fetched.origin.label;
  await put(META, REMOTES_KEY, [...(await remoteMounts()), { id, name }]);
  const path = remotePath(id, name);
  mounted.set(path, vfs);
  await put(META, ACTIVE_KEY, id);
  return { id, path, name };
}

export async function originOf(path: string): Promise<RemoteOrigin | null> {
  return readOrigin(vfsFor(path));
}

/**
 * Turns a read-only copy into an ordinary workspace: same files, new identity,
 * no origin stamp. The original is left exactly as it was.
 */
export async function saveCopy(path: string, name: string): Promise<WorkspaceInfo> {
  const from = vfsFor(path);
  if ((await readOrigin(from)) === null)
    throw new Error("This is already a workspace you own — there is nothing to copy out of.");
  const target = browserVfs();
  const taken = new Set((await target.list("collections")).map((entry) => entry.name));
  let copied = 0;
  for (const [source, destination] of await pairsUnder(from, "collections", taken)) {
    await target.write(destination, (await from.read(source)) as string);
    copied += 1;
  }
  for (const dir of ["environments", "protos", "files"]) {
    for (const file of await filesUnder(from, dir)) {
      if ((await target.read(file)) === null)
        await target.write(file, (await from.read(file)) as string);
    }
  }
  if (copied === 0)
    throw new Error(`${name} carries no requests, so there is nothing to copy.`);
  await put(META, ACTIVE_KEY, "browser");
  return {
    id: "browser",
    path: BROWSER_PATH,
    name: (await workspaceName(target)) ?? "Browser storage",
  };
}

async function filesUnder(vfs: Vfs, dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await vfs.list(dir)) {
    const path = `${dir}/${entry.name}`;
    if (entry.dir) out.push(...(await filesUnder(vfs, path)));
    else out.push(path);
  }
  return out;
}

/**
 * Every request file of the remote workspace, paired with where it lands in the
 * target — under a free slug, so a copy never lands on a collection the user
 * already has.
 */
async function pairsUnder(
  from: Vfs,
  root: string,
  taken: Set<string>,
): Promise<[string, string][]> {
  const out: [string, string][] = [];
  for (const entry of await from.list(root)) {
    if (!entry.dir) continue;
    let slug = entry.name;
    for (let n = 2; taken.has(slug); n += 1) slug = `${entry.name}-${n}`;
    taken.add(slug);
    for (const file of await filesUnder(from, `${root}/${entry.name}`))
      out.push([file, `${root}/${slug}/${file.slice(root.length + entry.name.length + 2)}`]);
  }
  return out;
}
