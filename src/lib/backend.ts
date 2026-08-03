import * as api from "./api";
import type {
  Environment,
  EnvironmentViewList,
  SavedPath,
  SavedRequest,
  Tree,
  WorkspaceList,
} from "./api";
import {
  mockEnvironments,
  mockRequest,
  mockTree,
  mockWorkspaces,
} from "./mock";

export function mockEnabled(): boolean {
  return import.meta.env.DEV && localStorage.getItem("mandalo.devMock") === "1";
}

export function basename(path: string): string {
  const parts = path.split("/").filter((p) => p !== "");
  return parts[parts.length - 1] ?? path;
}

export function listWorkspaces(): Promise<WorkspaceList> {
  if (mockEnabled()) return Promise.resolve(mockWorkspaces());
  return api.listWorkspaces();
}

export function listTree(workspace: string): Promise<Tree> {
  if (mockEnabled()) return Promise.resolve(mockTree());
  return api.listTree(workspace);
}

export function loadRequest(
  workspace: string,
  collection: string,
  path: string,
): Promise<SavedRequest> {
  if (mockEnabled()) return Promise.resolve(mockRequest(path));
  return api.loadRequest(workspace, collection, path);
}

export function saveRequest(
  workspace: string,
  collection: string,
  path: string | null,
  folder: string | null,
  request: SavedRequest,
): Promise<SavedPath> {
  if (mockEnabled())
    return Promise.resolve({ path: path ?? `${request.id}.toml` });
  return api.saveRequest(workspace, collection, path, folder, request);
}

export function deleteRequest(
  workspace: string,
  collection: string,
  path: string,
): Promise<void> {
  if (mockEnabled()) return Promise.resolve();
  return api.deleteRequest(workspace, collection, path);
}

export function listEnvironments(
  workspace: string,
): Promise<EnvironmentViewList> {
  if (mockEnabled())
    return Promise.resolve({ items: mockEnvironments(), skipped: [] });
  return api.listEnvironments(workspace);
}

export function saveEnvironment(
  workspace: string,
  env: Environment,
): Promise<string> {
  if (mockEnabled()) return Promise.resolve(env.name);
  return api.saveEnvironment(workspace, env);
}
