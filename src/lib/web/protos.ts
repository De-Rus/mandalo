import type { Vfs } from "./vfs";

export const PROTO_DIR = "protos";

export interface ProtoFile {
  path: string;
  contents: string;
}

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * A saved request stores the proto as a filesystem path, which a page cannot open.
 * So the browser build looks the same file up inside the workspace it already has
 * mounted — verbatim first, then by name under `protos/`.
 */
export function candidates(path: string): string[] {
  const clean = path.replace(/\\/g, "/").replace(/^\.\//, "");
  const bare = clean.replace(/^\/+/, "");
  const name = basename(clean);
  const tried = [clean, bare, `${PROTO_DIR}/${name}`, name];
  return [...new Set(tried.filter((p) => p !== "" && !p.startsWith("/")))];
}

async function listProtos(vfs: Vfs, dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await vfs.list(dir)) {
    const child = dir === "" ? entry.name : `${dir}/${entry.name}`;
    if (entry.dir) out.push(...(await listProtos(vfs, child)));
    else if (entry.name.endsWith(".proto")) out.push(child);
  }
  return out;
}

async function resolveOne(vfs: Vfs, path: string): Promise<ProtoFile> {
  for (const candidate of candidates(path)) {
    const contents = await vfs.read(candidate);
    if (contents !== null) return { path: candidate, contents };
  }
  throw new Error(
    `Could not find the proto file "${path}" in this workspace. A browser cannot read a path off your disk, so Mándalo looked for ${candidates(path)
      .map((c) => `"${c}"`)
      .join(", ")} inside the workspace instead. Add the .proto to the workspace's ${PROTO_DIR}/ folder — the “Proto files” button in the web ribbon copies one in — or run this request in the desktop app.`,
  );
}

/**
 * Everything under `protos/` rides along so that an `import` inside a listed file
 * resolves: the compiler has no filesystem to fall back to.
 */
export async function collect(
  vfs: Vfs,
  protoPaths: string[],
): Promise<ProtoFile[]> {
  if (protoPaths.length === 0)
    throw new Error("no proto files given");

  const files = new Map<string, ProtoFile>();
  for (const path of protoPaths) {
    const found = await resolveOne(vfs, path);
    files.set(found.path, found);
  }
  for (const path of await listProtos(vfs, PROTO_DIR)) {
    if (files.has(path)) continue;
    const contents = await vfs.read(path);
    if (contents !== null) files.set(path, { path, contents });
  }
  return [...files.values()];
}

export async function store(
  vfs: Vfs,
  name: string,
  contents: string,
): Promise<string> {
  if (!name.endsWith(".proto"))
    throw new Error(`"${name}" is not a .proto file`);
  const path = `${PROTO_DIR}/${basename(name)}`;
  await vfs.write(path, contents);
  return path;
}
