import type { CollectionNode, FolderNode, RequestSummary } from "./api";
import { fuzzyScore } from "./fuzzy";

export function folderRequestCount(folder: FolderNode): number {
  return (
    folder.requests.length +
    folder.folders.reduce((n, f) => n + folderRequestCount(f), 0)
  );
}

export function collectionRequestCount(collection: CollectionNode): number {
  return (
    collection.requests.length +
    collection.folders.reduce((n, f) => n + folderRequestCount(f), 0)
  );
}

function filterFolder(folder: FolderNode, query: string): FolderNode | null {
  const folders = folder.folders
    .map((f) => filterFolder(f, query))
    .filter((f): f is FolderNode => f !== null);
  const requests = folder.requests.filter((r) => matches(r, query));
  if (folders.length === 0 && requests.length === 0) return null;
  return { ...folder, folders, requests };
}

function matches(request: RequestSummary, query: string): boolean {
  return (
    fuzzyScore(request.name, query) > 0 || fuzzyScore(request.method, query) > 0
  );
}

export function filterTree(
  collections: CollectionNode[],
  query: string,
): CollectionNode[] {
  const trimmed = query.trim();
  if (trimmed === "") return collections;
  const out: CollectionNode[] = [];
  for (const collection of collections) {
    const folders = collection.folders
      .map((f) => filterFolder(f, trimmed))
      .filter((f): f is FolderNode => f !== null);
    const requests = collection.requests.filter((r) => matches(r, trimmed));
    if (folders.length > 0 || requests.length > 0)
      out.push({ ...collection, folders, requests });
  }
  return out;
}

export interface FlatRequest extends RequestSummary {
  collection: string;
  folder: string | null;
}

function flattenFolder(
  folder: FolderNode,
  collection: string,
  prefix: string,
  out: FlatRequest[],
): void {
  const here = prefix === "" ? folder.name : `${prefix} / ${folder.name}`;
  for (const request of folder.requests)
    out.push({ ...request, collection, folder: here });
  for (const child of folder.folders)
    flattenFolder(child, collection, here, out);
}

export function flattenTree(collections: CollectionNode[]): FlatRequest[] {
  const out: FlatRequest[] = [];
  for (const collection of collections) {
    for (const request of collection.requests)
      out.push({ ...request, collection: collection.name, folder: null });
    for (const folder of collection.folders)
      flattenFolder(folder, collection.name, "", out);
  }
  return out;
}

export interface DraftLike {
  id: string;
  name: string;
  kind: RequestSummary["kind"];
  method: string;
}

function overrideFolder(
  folder: FolderNode,
  byId: Map<string, DraftLike>,
): FolderNode {
  return {
    ...folder,
    folders: folder.folders.map((f) => overrideFolder(f, byId)),
    requests: folder.requests.map((r) => overrideRequest(r, byId)),
  };
}

function overrideRequest(
  request: RequestSummary,
  byId: Map<string, DraftLike>,
): RequestSummary {
  const draft = byId.get(request.id);
  if (!draft) return request;
  return {
    ...request,
    name: draft.name,
    kind: draft.kind,
    method: draft.method,
  };
}

export function applyDraftOverrides(
  collections: CollectionNode[],
  drafts: DraftLike[],
): CollectionNode[] {
  if (drafts.length === 0) return collections;
  const byId = new Map(drafts.map((d) => [d.id, d]));
  return collections.map((collection) => ({
    ...collection,
    folders: collection.folders.map((f) => overrideFolder(f, byId)),
    requests: collection.requests.map((r) => overrideRequest(r, byId)),
  }));
}

export function locateRequests(
  collections: CollectionNode[],
): Map<string, { collection: string; path: string }> {
  const out = new Map<string, { collection: string; path: string }>();
  const walk = (folder: FolderNode, slug: string) => {
    for (const request of folder.requests)
      out.set(request.id, { collection: slug, path: request.path });
    for (const child of folder.folders) walk(child, slug);
  };
  for (const collection of collections) {
    for (const request of collection.requests)
      out.set(request.id, { collection: collection.slug, path: request.path });
    for (const folder of collection.folders) walk(folder, collection.slug);
  }
  return out;
}

/**
 * The requests one folder holds, in the order the tree shows them. `folder` is
 * "" for the collection root. Nested folders are not included: reordering acts
 * on one folder at a time, the same unit the manifest records.
 */
export function requestsIn(
  collection: CollectionNode,
  folder: string,
): RequestSummary[] {
  if (folder === "") return collection.requests;
  const find = (folders: FolderNode[]): FolderNode | null => {
    for (const node of folders) {
      if (node.path === folder) return node;
      const deeper = find(node.folders);
      if (deeper) return deeper;
    }
    return null;
  };
  return find(collection.folders)?.requests ?? [];
}

export function folderOf(path: string): string {
  const parts = path.split("/");
  return parts.slice(0, -1).join("/");
}
