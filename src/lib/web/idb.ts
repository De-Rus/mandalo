const DB_NAME = "mandalo";
const DB_VERSION = 2;
export const FILES = "files";
export const HANDLES = "handles";
export const META = "meta";
export const VERSIONS = "versions";

let opening: Promise<IDBDatabase> | null = null;

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("IndexedDB request failed"));
  });
}

export function db(): Promise<IDBDatabase> {
  if (opening) return opening;
  opening = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const database = req.result;
      if (!database.objectStoreNames.contains(FILES))
        database.createObjectStore(FILES);
      if (!database.objectStoreNames.contains(HANDLES))
        database.createObjectStore(HANDLES);
      if (!database.objectStoreNames.contains(META)) database.createObjectStore(META);
      if (!database.objectStoreNames.contains(VERSIONS))
        database.createObjectStore(VERSIONS);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () =>
      reject(
        req.error ??
          new Error(
            "This browser refused to open IndexedDB. Private-browsing modes sometimes do this.",
          ),
      );
  });
  return opening;
}

async function tx(store: string, mode: IDBTransactionMode): Promise<IDBObjectStore> {
  const database = await db();
  return database.transaction(store, mode).objectStore(store);
}

export async function get<T>(store: string, key: string): Promise<T | undefined> {
  return request<T | undefined>((await tx(store, "readonly")).get(key));
}

export async function put(store: string, key: string, value: unknown): Promise<void> {
  await request((await tx(store, "readwrite")).put(value, key));
}

export async function del(store: string, key: string): Promise<void> {
  await request((await tx(store, "readwrite")).delete(key));
}

export async function keys(store: string): Promise<string[]> {
  const all = await request((await tx(store, "readonly")).getAllKeys());
  return all.map(String);
}

export interface Version {
  text: string;
  at: number;
}

export interface AtomicWrite {
  fileKey: string;
  text: string;
  dirKeys: string[];
  dirMarker: string;
  versionKey: string;
  ringLimit: number;
  ringMaxBytes: number;
}

/**
 * One transaction for the whole logical save: the directory markers, the previous
 * revision pushed onto the ring and the new text either all land or none do, so a
 * tab closed mid-save can never leave a half-written file behind.
 */
export async function writeAtomically(write: AtomicWrite): Promise<void> {
  return writeInto(await db(), write);
}

export function writeInto(database: IDBDatabase, write: AtomicWrite): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const t = database.transaction([FILES, VERSIONS], "readwrite");
    const files = t.objectStore(FILES);
    const versions = t.objectStore(VERSIONS);

    for (const key of write.dirKeys) files.put(write.dirMarker, key);

    const previous = files.get(write.fileKey);
    previous.onsuccess = () => {
      const old = previous.result as string | undefined;
      if (old !== undefined && old !== write.dirMarker && old !== write.text) {
        const ring = versions.get(write.versionKey);
        ring.onsuccess = () => {
          const kept = ((ring.result as Version[] | undefined) ?? []).filter(
            (v) => v.text.length <= write.ringMaxBytes,
          );
          if (old.length <= write.ringMaxBytes)
            versions.put(
              [{ text: old, at: Date.now() }, ...kept].slice(0, write.ringLimit),
              write.versionKey,
            );
        };
      }
      files.put(write.text, write.fileKey);
    };

    t.oncomplete = () => resolve();
    t.onerror = () =>
      reject(t.error ?? new Error("This browser refused to write to IndexedDB"));
    t.onabort = () =>
      reject(t.error ?? new Error("The save was rolled back before it completed"));
  });
}

export async function entries<T>(store: string): Promise<[string, T][]> {
  const handle = await tx(store, "readonly");
  const [ks, vs] = await Promise.all([
    request(handle.getAllKeys()),
    request<T[]>(handle.getAll()),
  ]);
  return ks.map((k, i) => [String(k), vs[i]]);
}
