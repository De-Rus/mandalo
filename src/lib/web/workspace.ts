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
  encodeCollectionManifest,
  encodeEnvDoc,
  encodeWorkspaceManifest,
} from "./toml";
import type { EnvDoc, VarDef } from "./toml";
import { blockNames, parseFile, removeBlock, replaceBlock } from "../format/edit";
import { withFileVars } from "../format/httpFormat";
import type { RequestModel } from "../format/model";
import { extensionForKind, renderRequest } from "../format/render";
import {
  indexIn,
  requestFilePath,
  requestFragmentOf,
  engineOnlyReason,
  engineOnlyRequestKind,
  textFileKind,
  type TextFileKind,
} from "../format/textFormat";
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

function toSaved(model: RequestModel): SavedRequest {
  return {
    id: model.id,
    name: model.name,
    kind: model.kind as SavedRequest["kind"],
    method: model.method,
    url: model.url,
    description: model.description ?? null,
    body: model.body ?? null,
    bodyFile: model.bodyFile ?? null,
    headers: model.headers,
    auth: model.auth as SavedRequest["auth"],
    graphql: model.graphql ?? null,
    grpc: model.grpc ?? null,
    scripts: { pre: model.scripts.pre ?? null, post: model.scripts.post ?? null },
    tests: [],
    captures: [],
  };
}

/** The web engine applies the auth a request effectively sends, which is what the
 * Rust runner's `Auth::effective()` does with an inherited wrapper. */
function effectiveAuth(auth: SavedRequest["auth"] | undefined): RequestModel["auth"] {
  if (!auth) return { type: "none" };
  return auth.type === "inherited" ? effectiveAuth(auth.auth) : auth;
}

function toModel(request: SavedRequest): RequestModel {
  const model: RequestModel = {
    schemaVersion: SCHEMA_VERSION,
    id: request.id,
    name: request.name,
    kind: request.kind,
    method: request.method,
    url: request.url,
    headers: request.headers ?? [],
    auth: effectiveAuth(request.auth),
    scripts: {},
    tests: request.tests ?? [],
    captures: request.captures ?? [],
  };
  if (request.description !== undefined && request.description !== null)
    model.description = request.description;
  if (request.bodyFile !== undefined && request.bodyFile !== null)
    model.bodyFile = request.bodyFile;
  else if (typeof request.body === "string") model.body = request.body;
  else if (request.body !== undefined && request.body !== null) {
    if (request.body.mode !== "raw")
      throw new Error(
        `"${request.name}" has a ${request.body.mode} body, and a web page cannot send one. Open this workspace in the Mándalo desktop app or run it with the CLI.`,
      );
    model.body = request.body.text;
  }
  if (request.graphql) model.graphql = request.graphql;
  if (request.grpc) model.grpc = request.grpc;
  if (request.scripts?.pre) model.scripts.pre = request.scripts.pre;
  if (request.scripts?.post) model.scripts.post = request.scripts.post;
  return model;
}

const TOML_REQUEST =
  "requests are .http and .grpc files now — this TOML request has to be converted";

interface RequestFile {
  file: string;
  kind: TextFileKind;
  source: string;
}

async function readRequestFile(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<RequestFile> {
  const file = requestFilePath(rel);
  const kind = textFileKind(file);
  if (kind === undefined)
    throw new Error(
      `${file} is not a request file — Mándalo stores HTTP and GraphQL in .http and gRPC in .grpc`,
    );
  const source = await vfs.read(requestPath(slug, file));
  if (source === null) throw new Error(`unknown request: ${rel}`);
  return { file, kind, source };
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
    if (entry.name.startsWith(".")) continue;
    const path = rel === "" ? entry.name : `${rel}/${entry.name}`;
    const kind = textFileKind(entry.name);
    if (kind === undefined) {
      if (entry.name.endsWith(".toml") && !(rel === "" && entry.name === MANIFEST))
        skipped.push(`${COLLECTIONS}/${slug}/${path}: ${TOML_REQUEST}`);
      const engineOnly = engineOnlyRequestKind(entry.name);
      if (engineOnly !== undefined)
        skipped.push(`${COLLECTIONS}/${slug}/${path}: ${engineOnlyReason(engineOnly)}`);
      continue;
    }
    try {
      const raw = await vfs.read(`${collectionDir(slug)}/${path}`);
      if (raw === null) continue;
      for (const block of parseFile(path, raw, kind).blocks)
        requests.push({
          id: block.model.id,
          name: block.name,
          kind: block.model.kind as RequestSummary["kind"],
          method: block.model.method,
          path: `${path}#${block.index}`,
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

function isReservedCollectionSlug(slug: string): boolean {
  return slug === COLLECTIONS || slug === ENVIRONMENTS;
}

export async function createCollection(
  vfs: Vfs,
  name: string,
): Promise<CollectionInfo> {
  const trimmed = name.trim();
  if (trimmed === "") throw new Error("a collection needs a name");
  const base = slugify(trimmed);
  if (isReservedCollectionSlug(base))
    throw new Error(`collection name "${base}" is reserved`);
  const existing = (await vfs.list(COLLECTIONS)).map((e) => e.name);
  let slug = base;
  for (let n = 2; existing.includes(slug); n += 1) slug = `${base}-${n}`;
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

/** The request the runner sends: the file's own `@vars` already applied. */
export async function loadRequest(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<SavedRequest> {
  const found = await readRequestFile(vfs, slug, rel);
  const file = parseFile(found.file, found.source, found.kind);
  const index = indexIn(blockNames(file), found.file, requestFragmentOf(rel));
  const resolved = withFileVars(file)[index];
  if (resolved === undefined) throw new Error(`unknown request: ${rel}`);
  return toSaved(resolved.model);
}

async function freeFile(
  vfs: Vfs,
  slug: string,
  dir: string,
  base: string,
  extension: string,
): Promise<string> {
  for (let n = 1; ; n += 1) {
    const name = n === 1 ? `${base}.${extension}` : `${base}-${n}.${extension}`;
    const candidate = dir === "" ? name : `${dir}/${name}`;
    if ((await vfs.read(requestPath(slug, candidate))) === null) return candidate;
  }
}

/**
 * Saves one request. An existing one is rewritten inside the file that holds it, so
 * every other block keeps its bytes; a new one gets its own file. The returned path
 * always carries the block index, exactly as the Rust twin's does.
 */
export async function saveRequest(
  vfs: Vfs,
  slug: string,
  previous: string | null,
  folder: string | null,
  request: SavedRequest,
): Promise<string> {
  const extension = extensionForKind(request.kind);
  const model = toModel(request);

  if (previous !== null) {
    const found = await readRequestFile(vfs, slug, previous);
    if (found.kind !== extension)
      throw new Error(
        `${found.file} holds ${found.kind} requests, so a ${request.kind} request cannot be saved into it`,
      );
    const file = parseFile(found.file, found.source, found.kind);
    const index = indexIn(blockNames(file), found.file, requestFragmentOf(previous));
    await vfs.write(requestPath(slug, found.file), replaceBlock(file, index, model));
    return `${found.file}#${index}`;
  }

  const dir = (folder ?? "").replace(/^\/+|\/+$/g, "");
  assertRelative(dir);
  const target = await freeFile(vfs, slug, dir, slugify(request.name), extension);
  await vfs.write(requestPath(slug, target), renderRequest(model));
  return `${target}#0`;
}

export async function deleteRequest(
  vfs: Vfs,
  slug: string,
  rel: string,
): Promise<void> {
  const found = await readRequestFile(vfs, slug, rel);
  const fragment = requestFragmentOf(rel);
  const file = parseFile(found.file, found.source, found.kind);
  if (fragment === undefined || file.blocks.length === 1) {
    await vfs.remove(requestPath(slug, found.file));
    return;
  }
  const index = indexIn(blockNames(file), found.file, fragment);
  await vfs.write(requestPath(slug, found.file), removeBlock(file, index));
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

/**
 * Cuts one request out of its file and lands it in its own file in the target folder.
 * The block is moved exactly as written — resolving the file's `@vars` first would
 * bake them into literals the move was never asked to make.
 */
export async function moveRequest(
  vfs: Vfs,
  slug: string,
  from: string,
  toFolder: string,
): Promise<string> {
  const found = await readRequestFile(vfs, slug, from);
  const file = parseFile(found.file, found.source, found.kind);
  const index = indexIn(blockNames(file), found.file, requestFragmentOf(from));
  const block = file.blocks[index]!;
  const dir = toFolder.replace(/^\/+|\/+$/g, "");
  assertRelative(dir);
  const target = await freeFile(vfs, slug, dir, slugify(block.name), found.kind);
  await vfs.write(requestPath(slug, target), renderRequest(block.model, file.newline));
  await deleteRequest(vfs, slug, from);
  return `${target}#0`;
}

async function readEnvDoc(vfs: Vfs, name: string): Promise<EnvDoc | null> {
  const raw = await vfs.read(`${ENVIRONMENTS}/${name}.toml`);
  return raw === null ? null : decodeEnvDoc(name, raw);
}

/**
 * A page cannot read this machine's secrets file, so anything not shared has no
 * value here and `set` is false — saying "not set" rather than pretending a
 * value exists is what keeps the browser from reporting a run as sendable when
 * it is not.
 */
function viewOf(doc: EnvDoc): EnvironmentView {
  const vars: EnvironmentView["vars"] = {};
  for (const [key, def] of Object.entries(doc.vars)) {
    vars[key] = def.shared
      ? { shared: true, secret: false, value: def.value, set: true, source: "file" }
      : {
          shared: false,
          secret: def.secret,
          value: null,
          hosts: def.hosts,
          set: false,
        };
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
    if (def.shared) continue;
    if (key in env.vars)
      throw new Error(
        `${env.name}.${key} is declared ${def.secret ? "secret" : "shared = false"}; its value never goes in the environment file — write it on the machine that holds it`,
      );
    vars[key] = def;
  }
  for (const [key, value] of Object.entries(env.vars))
    vars[key] = { shared: true, secret: false, value };
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
