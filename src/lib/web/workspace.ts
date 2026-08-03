import type {
  CollectionInfo,
  CollectionList,
  CollectionNode,
  Environment,
  EnvironmentView,
  EnvironmentViewList,
  FolderNode,
  RequestSummary,
  SavedRequest,
  Tree,
} from "../api";
import {
  decodeEnvDoc,
  decodeManifest,
  decodeRequest,
  encodeCollectionManifest,
  encodeEnvDoc,
  encodeRequest,
  encodeWorkspaceManifest,
} from "./toml";
import type { EnvDoc, VarDef } from "./toml";
import type { Vfs } from "./vfs";

export const SCHEMA_VERSION = 1;

const COLLECTIONS = "collections";
const ENVIRONMENTS = "environments";
const MANIFEST = "collection.toml";
const WORKSPACE_MANIFEST = "mandalo.toml";

export function slugify(name: string): string {
  let out = "";
  for (const ch of name) {
    if (/[a-zA-Z0-9]/.test(ch)) out += ch.toLowerCase();
    else if (!out.endsWith("-")) out += "-";
  }
  const trimmed = out.replace(/^-+|-+$/g, "");
  return trimmed === "" ? "untitled" : trimmed;
}

export function validateEnvName(name: string): void {
  if (name === "" || !/^[A-Za-z0-9_-]+$/.test(name))
    throw new Error(`invalid environment name: "${name}"`);
}

function parentOf(path: string): string {
  return path.split("/").slice(0, -1).join("/");
}

function fileName(path: string): string {
  return path.split("/").pop() ?? path;
}

function assertComponent(part: string): void {
  if (
    part === "" ||
    part === "." ||
    part === ".." ||
    part.startsWith(".") ||
    part.includes("\\") ||
    part.includes("\0")
  )
    throw new Error(`invalid path component: "${part}"`);
}

export function assertRelative(rel: string): string[] {
  const trimmed = rel.replace(/^\/+|\/+$/g, "");
  if (trimmed === "") return [];
  const parts = trimmed.split("/");
  for (const part of parts) assertComponent(part);
  return parts;
}

function collectionDir(slug: string): string {
  if (!/^[a-z0-9-]+$/.test(slug))
    throw new Error(`invalid collection slug: "${slug}"`);
  return `${COLLECTIONS}/${slug}`;
}

function requestPath(slug: string, rel: string): string {
  const parts = assertRelative(rel);
  if (parts.length === 0) throw new Error("a request needs a path");
  return `${collectionDir(slug)}/${parts.join("/")}`;
}

function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export async function ensureWorkspace(vfs: Vfs, name: string): Promise<void> {
  // `id` is not #[serde(default)] on the Rust side: a mandalo.toml without it
  // fails to parse in the desktop app.
  if ((await vfs.read(WORKSPACE_MANIFEST)) === null)
    await vfs.write(
      WORKSPACE_MANIFEST,
      encodeWorkspaceManifest({
        schemaVersion: SCHEMA_VERSION,
        id: newId(),
        name,
      }),
    );
  await vfs.mkdirp(COLLECTIONS);
  await vfs.mkdirp(ENVIRONMENTS);
}

export async function workspaceName(vfs: Vfs): Promise<string | null> {
  const raw = await vfs.read(WORKSPACE_MANIFEST);
  if (raw === null) return null;
  try {
    return decodeManifest(raw).name;
  } catch {
    return null;
  }
}

async function readCollection(
  vfs: Vfs,
  slug: string,
): Promise<CollectionInfo | null> {
  const raw = await vfs.read(`${collectionDir(slug)}/${MANIFEST}`);
  if (raw === null) return null;
  const manifest = decodeManifest(raw);
  if (manifest.schemaVersion !== SCHEMA_VERSION)
    throw new Error(
      `collections/${slug}/${MANIFEST}: unsupported schema_version ${manifest.schemaVersion} (expected ${SCHEMA_VERSION})`,
    );
  return { id: manifest.id ?? slug, slug, name: manifest.name };
}

export async function listCollections(vfs: Vfs): Promise<CollectionList> {
  const items: CollectionInfo[] = [];
  const skipped: string[] = [];
  for (const entry of await vfs.list(COLLECTIONS)) {
    if (!entry.dir) continue;
    try {
      const found = await readCollection(vfs, entry.name);
      if (found) items.push(found);
    } catch (e) {
      skipped.push(`${COLLECTIONS}/${entry.name}: ${message(e)}`);
    }
  }
  return { items, skipped };
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

async function readFolder(
  vfs: Vfs,
  slug: string,
  rel: string,
  skipped: string[],
): Promise<{ folders: FolderNode[]; requests: RequestSummary[] }> {
  const dir = rel === "" ? collectionDir(slug) : `${collectionDir(slug)}/${rel}`;
  const folders: FolderNode[] = [];
  const requests: RequestSummary[] = [];
  for (const entry of await vfs.list(dir)) {
    if (entry.dir) {
      const child = rel === "" ? entry.name : `${rel}/${entry.name}`;
      const inner = await readFolder(vfs, slug, child, skipped);
      folders.push({
        name: entry.name,
        path: child,
        folders: inner.folders,
        requests: inner.requests,
      });
      continue;
    }
    if (!entry.name.endsWith(".toml")) continue;
    if (rel === "" && entry.name === MANIFEST) continue;
    const path = rel === "" ? entry.name : `${rel}/${entry.name}`;
    try {
      const raw = await vfs.read(`${collectionDir(slug)}/${path}`);
      if (raw === null) continue;
      const request = decodeRequest(raw);
      requests.push({
        id: request.id,
        name: request.name,
        kind: request.kind,
        method: request.method,
        path,
      });
    } catch (e) {
      skipped.push(`${COLLECTIONS}/${slug}/${path}: ${message(e)}`);
    }
  }
  return { folders, requests };
}

export async function listTree(vfs: Vfs): Promise<Tree> {
  const { items, skipped } = await listCollections(vfs);
  const collections: CollectionNode[] = [];
  for (const info of items) {
    const { folders, requests } = await readFolder(vfs, info.slug, "", skipped);
    collections.push({ ...info, folders, requests });
  }
  return { collections, skipped };
}

export async function createCollection(
  vfs: Vfs,
  name: string,
): Promise<CollectionInfo> {
  const trimmed = name.trim();
  if (trimmed === "") throw new Error("a collection needs a name");
  const existing = (await vfs.list(COLLECTIONS)).map((e) => e.name);
  let slug = slugify(trimmed);
  for (let n = 2; existing.includes(slug); n += 1) slug = `${slugify(trimmed)}-${n}`;
  const id = slug;
  await vfs.mkdirp(collectionDir(slug));
  await vfs.write(
    `${collectionDir(slug)}/${MANIFEST}`,
    encodeCollectionManifest({ schemaVersion: SCHEMA_VERSION, id, name: trimmed }),
  );
  return { id, slug, name: trimmed };
}

export async function renameCollection(
  vfs: Vfs,
  slug: string,
  name: string,
): Promise<CollectionInfo> {
  const current = await readCollection(vfs, slug);
  if (!current) throw new Error(`unknown collection: ${slug}`);
  const trimmed = name.trim();
  if (trimmed === "") throw new Error("a collection needs a name");
  await vfs.write(
    `${collectionDir(slug)}/${MANIFEST}`,
    encodeCollectionManifest({
      schemaVersion: SCHEMA_VERSION,
      id: current.id,
      name: trimmed,
    }),
  );
  return { ...current, name: trimmed };
}

export async function deleteCollection(vfs: Vfs, slug: string): Promise<void> {
  await vfs.removeDir(collectionDir(slug));
}

export async function loadRequest(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<SavedRequest> {
  const raw = await vfs.read(requestPath(slug, rel));
  if (raw === null) throw new Error(`request not found: ${rel}`);
  return decodeRequest(raw);
}

async function freePath(
  vfs: Vfs,
  slug: string,
  desired: string,
  keep: string | null,
): Promise<string> {
  const stem = desired.replace(/\.toml$/, "");
  for (let n = 1; ; n += 1) {
    const candidate = n === 1 ? desired : `${stem}-${n}.toml`;
    if (candidate === keep) return candidate;
    if ((await vfs.read(requestPath(slug, candidate))) === null) return candidate;
  }
}

export async function saveRequest(
  vfs: Vfs,
  slug: string,
  previous: string | null,
  folder: string | null,
  request: SavedRequest,
): Promise<string> {
  const dir = (folder ?? (previous === null ? "" : parentOf(previous))).replace(
    /^\/+|\/+$/g,
    "",
  );
  assertRelative(dir);
  const base = `${slugify(request.name)}.toml`;
  const desired = dir === "" ? base : `${dir}/${base}`;
  const target =
    previous !== null && fileName(previous) === base && parentOf(previous) === dir
      ? previous
      : await freePath(vfs, slug, desired, previous);
  await vfs.write(requestPath(slug, target), encodeRequest(request));
  if (previous !== null && previous !== target)
    await vfs.remove(requestPath(slug, previous));
  return target;
}

export async function deleteRequest(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<void> {
  await vfs.remove(requestPath(slug, rel));
}

export async function createFolder(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<void> {
  const parts = assertRelative(rel);
  if (parts.length === 0) throw new Error("a folder needs a name");
  await vfs.mkdirp(`${collectionDir(slug)}/${parts.join("/")}`);
}

export async function deleteFolder(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<void> {
  const parts = assertRelative(rel);
  if (parts.length === 0) throw new Error("a folder needs a name");
  await vfs.removeDir(`${collectionDir(slug)}/${parts.join("/")}`);
}

async function copyTree(
  vfs: Vfs,
  slug: string,
  from: string,
  to: string,
): Promise<void> {
  await vfs.mkdirp(`${collectionDir(slug)}/${to}`);
  for (const entry of await vfs.list(`${collectionDir(slug)}/${from}`)) {
    const child = `${from}/${entry.name}`;
    const target = `${to}/${entry.name}`;
    if (entry.dir) {
      await copyTree(vfs, slug, child, target);
      continue;
    }
    const raw = await vfs.read(`${collectionDir(slug)}/${child}`);
    if (raw !== null) await vfs.write(`${collectionDir(slug)}/${target}`, raw);
  }
}

export async function renameFolder(
  vfs: Vfs,
  slug: string,
  rel: string,
  name: string,
): Promise<string> {
  const parts = assertRelative(rel);
  if (parts.length === 0) throw new Error("a folder needs a name");
  const parent = parts.slice(0, -1).join("/");
  const wanted = slugify(name);
  const to = parent === "" ? wanted : `${parent}/${wanted}`;
  if (to === rel) return rel;
  const clash = (await vfs.list(`${collectionDir(slug)}/${parent}`)).some(
    (e) => e.dir && e.name === wanted,
  );
  if (clash) throw new Error(`a folder named "${wanted}" already exists here`);
  await copyTree(vfs, slug, rel, to);
  await vfs.removeDir(`${collectionDir(slug)}/${rel}`);
  return to;
}

export async function moveRequest(
  vfs: Vfs,
  slug: string,
  from: string,
  toFolder: string,
): Promise<string> {
  const request = await loadRequest(vfs, slug, from);
  return saveRequest(vfs, slug, from, toFolder, request);
}

async function readEnvDoc(vfs: Vfs, name: string): Promise<EnvDoc | null> {
  const raw = await vfs.read(`${ENVIRONMENTS}/${name}.toml`);
  return raw === null ? null : decodeEnvDoc(name, raw);
}

/**
 * The browser has no keychain: a secret is a declaration the desktop app can
 * fill, so `set` is always false here rather than pretending a value exists.
 */
function viewOf(doc: EnvDoc): EnvironmentView {
  const vars: EnvironmentView["vars"] = {};
  for (const [key, def] of Object.entries(doc.vars)) {
    vars[key] = def.secret
      ? { secret: true, value: null, hosts: def.hosts, set: false }
      : { secret: false, value: def.value, set: true };
  }
  return { name: doc.name, vars };
}

export async function listEnvironments(
  vfs: Vfs,
): Promise<EnvironmentViewList> {
  const items: EnvironmentView[] = [];
  const skipped: string[] = [];
  for (const entry of await vfs.list(ENVIRONMENTS)) {
    if (entry.dir || !entry.name.endsWith(".toml")) continue;
    const name = entry.name.slice(0, -5);
    try {
      const raw = await vfs.read(`${ENVIRONMENTS}/${entry.name}`);
      if (raw !== null) items.push(viewOf(decodeEnvDoc(name, raw)));
    } catch (e) {
      skipped.push(`${ENVIRONMENTS}/${entry.name}: ${message(e)}`);
    }
  }
  return { items, skipped };
}

export async function saveEnvironment(
  vfs: Vfs,
  env: Environment,
): Promise<string> {
  validateEnvName(env.name);
  const existing = await readEnvDoc(vfs, env.name);
  const vars: Record<string, VarDef> = {};
  for (const [key, def] of Object.entries(existing?.vars ?? {})) {
    if (!def.secret) continue;
    if (key in env.vars)
      throw new Error(
        `${env.name}.${key} is declared secret; its value never goes in the environment file — write it with Set value`,
      );
    vars[key] = def;
  }
  for (const [key, value] of Object.entries(env.vars))
    vars[key] = { secret: false, value };
  await vfs.write(
    `${ENVIRONMENTS}/${env.name}.toml`,
    encodeEnvDoc({ name: env.name, vars }),
  );
  return env.name;
}

export async function bindSecretHost(
  vfs: Vfs,
  name: string,
  key: string,
  host: string,
): Promise<string[]> {
  validateEnvName(name);
  const normalized = host.trim().toLowerCase();
  if (normalized === "") throw new Error("host must not be empty");
  const doc = await readEnvDoc(vfs, name);
  if (!doc) throw new Error(`unknown environment: ${name}`);
  const def = doc.vars[key];
  if (!def || !def.secret)
    throw new Error(`${name}.${key} is not declared secret`);
  if (!def.hosts.includes(normalized)) def.hosts = [...def.hosts, normalized].sort();
  await vfs.write(`${ENVIRONMENTS}/${name}.toml`, encodeEnvDoc(doc));
  return def.hosts;
}

export async function deleteVar(
  vfs: Vfs,
  name: string,
  key: string,
): Promise<void> {
  validateEnvName(name);
  const doc = await readEnvDoc(vfs, name);
  if (!doc) throw new Error(`unknown environment: ${name}`);
  if (!(key in doc.vars)) throw new Error(`unknown variable: ${name}.${key}`);
  delete doc.vars[key];
  await vfs.write(`${ENVIRONMENTS}/${name}.toml`, encodeEnvDoc(doc));
}

export async function secretStatus(
  vfs: Vfs,
  name: string,
): Promise<Record<string, boolean>> {
  const doc = await readEnvDoc(vfs, name);
  if (!doc) throw new Error(`unknown environment: ${name}`);
  const status: Record<string, boolean> = {};
  for (const [key, def] of Object.entries(doc.vars))
    if (def.secret) status[key] = false;
  return status;
}

export async function deleteEnvironment(vfs: Vfs, name: string): Promise<void> {
  validateEnvName(name);
  await vfs.remove(`${ENVIRONMENTS}/${name}.toml`);
}
