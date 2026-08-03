import { promises as fs } from "node:fs";
import * as path from "node:path";
import type {
  CollectionNode,
  EnvironmentModel,
  FolderNode,
  RequestNode,
  WorkspaceNode,
} from "./model";
import { parseCollectionManifest, parseEnvironment, parseWorkspaceManifest } from "./parse";
import {
  isTextRequestPath,
  requestFilePath,
  requestPathAt,
  scanTextRequests,
  textFileKind,
} from "./textFormat";

const MANIFEST = "mandalo.toml";
const COLLECTION_MANIFEST = "collection.toml";

async function readDir(dir: string): Promise<import("node:fs").Dirent[]> {
  try {
    return await fs.readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

function toPosix(value: string): string {
  return value.split(path.sep).join("/");
}

async function scanFolder(
  dir: string,
  relPath: string,
  skipped: string[],
): Promise<{ folders: FolderNode[]; requests: RequestNode[] }> {
  const folders: FolderNode[] = [];
  const requests: RequestNode[] = [];
  for (const entry of await readDir(dir)) {
    if (entry.name.startsWith(".")) continue;
    const fsPath = path.join(dir, entry.name);
    const childRel = relPath === "" ? entry.name : `${relPath}/${entry.name}`;
    if (entry.isDirectory()) {
      const child = await scanFolder(fsPath, childRel, skipped);
      folders.push({ name: entry.name, relPath: childRel, ...child });
      continue;
    }
    const fileKind = textFileKind(entry.name);
    if (fileKind === undefined) continue;
    try {
      for (const block of scanTextRequests(await fs.readFile(fsPath, "utf8"), fileKind)) {
        requests.push({
          id: `${childRel}-${block.index}`,
          name: block.name,
          kind: block.kind,
          method: block.method,
          relPath: requestPathAt(childRel, block.index),
          fsPath,
          index: block.index,
          line: block.lineNumber,
        });
      }
    } catch (error) {
      skipped.push(`${toPosix(fsPath)}: ${(error as Error).message}`);
    }
  }
  folders.sort((a, b) => a.name.localeCompare(b.name));
  requests.sort((a, b) => {
    const left = requestFilePath(a.relPath);
    const right = requestFilePath(b.relPath);
    return left === right ? a.index - b.index : left.localeCompare(right);
  });
  return { folders, requests };
}

async function scanCollections(root: string, skipped: string[]): Promise<CollectionNode[]> {
  const dir = path.join(root, "collections");
  const collections: CollectionNode[] = [];
  for (const entry of await readDir(dir)) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
    const dirPath = path.join(dir, entry.name);
    const manifestPath = path.join(dirPath, COLLECTION_MANIFEST);
    let manifest;
    try {
      manifest = parseCollectionManifest(await fs.readFile(manifestPath, "utf8"));
    } catch (error) {
      skipped.push(`${toPosix(manifestPath)}: ${(error as Error).message}`);
      continue;
    }
    const tree = await scanFolder(dirPath, "", skipped);
    collections.push({
      id: manifest.id,
      slug: entry.name,
      name: manifest.name,
      dirPath,
      manifestPath,
      ...tree,
    });
  }
  collections.sort((a, b) => a.name.localeCompare(b.name));
  return collections;
}

export async function scanEnvironments(root: string, skipped: string[] = []): Promise<EnvironmentModel[]> {
  const dir = path.join(root, "environments");
  const items: EnvironmentModel[] = [];
  for (const entry of await readDir(dir)) {
    if (!entry.isFile() || !entry.name.endsWith(".toml")) continue;
    const fsPath = path.join(dir, entry.name);
    try {
      items.push(parseEnvironment(await fs.readFile(fsPath, "utf8"), entry.name.replace(/\.toml$/, "")));
    } catch (error) {
      skipped.push(`${toPosix(fsPath)}: ${(error as Error).message}`);
    }
  }
  items.sort((a, b) => a.name.localeCompare(b.name));
  return items;
}

export async function scanWorkspace(root: string): Promise<WorkspaceNode> {
  const manifestPath = path.join(root, MANIFEST);
  const manifest = parseWorkspaceManifest(await fs.readFile(manifestPath, "utf8"));
  const skipped: string[] = [];
  return {
    id: manifest.id,
    name: manifest.name,
    rootPath: root,
    manifestPath,
    collections: await scanCollections(root, skipped),
    environments: await scanEnvironments(root, skipped),
    skipped,
  };
}

export function findCollectionFor(workspace: WorkspaceNode, fsPath: string): CollectionNode | undefined {
  return workspace.collections.find(
    (collection) => fsPath === collection.dirPath || fsPath.startsWith(collection.dirPath + path.sep),
  );
}

export function requestRelPath(collection: CollectionNode, fsPath: string): string {
  return toPosix(path.relative(collection.dirPath, fsPath));
}

/** A bare file URI carries no block index, so it addresses the first request in the file. */
export function defaultRequestRelPath(collection: CollectionNode, fsPath: string): string {
  const relPath = requestRelPath(collection, fsPath);
  return isTextRequestPath(relPath) ? requestPathAt(relPath, 0) : relPath;
}

export function flattenRequests(collection: CollectionNode): RequestNode[] {
  const out: RequestNode[] = [...collection.requests];
  const walk = (folders: FolderNode[]): void => {
    for (const folder of folders) {
      out.push(...folder.requests);
      walk(folder.folders);
    }
  };
  walk(collection.folders);
  return out;
}
