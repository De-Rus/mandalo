import { invoke } from "@tauri-apps/api/core";

export type Kind = "http" | "graphql" | "grpc";

export type Auth =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apikey"; key: string; value: string; placement: "header" | "query" };

export interface RequestSpec {
  kind: "http" | "graphql";
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
  auth: Auth;
  graphql: { query: string; variables: string } | null;
  vars: Record<string, string>;
}

export interface ResponseData {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: string;
  binary: boolean;
  durationMs: number;
  sizeBytes: number;
}

export interface GrpcMethodInfo {
  service: string;
  method: string;
  input: string;
  output: string;
  clientStreaming: boolean;
  serverStreaming: boolean;
}

export interface GrpcRequest {
  url: string;
  protoPaths: string[];
  service: string;
  method: string;
  message: string;
  metadata: [string, string][];
  vars: Record<string, string>;
}

export interface GrpcResponse {
  body: string;
  durationMs: number;
}

export interface Environment {
  name: string;
  vars: Record<string, string>;
}

export interface SavedGrpc {
  protoPaths: string[];
  service: string;
  method: string;
  message: string;
  metadata: [string, string][];
}

export interface SavedRequest {
  id: string;
  name: string;
  kind: Kind;
  method: string;
  url: string;
  headers: [string, string][];
  body?: string | null;
  auth: Auth;
  graphql?: { query: string; variables: string } | null;
  grpc?: SavedGrpc | null;
}

export interface RequestList {
  items: SavedRequest[];
  skipped: string[];
}

export interface EnvironmentList {
  items: Environment[];
  skipped: string[];
}

export interface ImportReport {
  imported: number;
  environments: number;
  skipped: string[];
  warnings: string[];
  summary: string;
}

export function sendRequest(spec: RequestSpec): Promise<ResponseData> {
  return invoke("send_request", { spec });
}

export function listGrpcMethods(protoPaths: string[]): Promise<GrpcMethodInfo[]> {
  return invoke("list_grpc_methods", { protoPaths });
}

export function sendGrpc(req: GrpcRequest): Promise<GrpcResponse> {
  return invoke("send_grpc", { spec: req });
}

export function listEnvironments(workspace: string): Promise<EnvironmentList> {
  return invoke("list_environments", { workspace });
}

export function saveEnvironment(workspace: string, env: Environment): Promise<void> {
  return invoke("save_environment", { workspace, env });
}

export function deleteEnvironment(workspace: string, name: string): Promise<void> {
  return invoke("delete_environment", { workspace, name });
}

export function listRequests(workspace: string): Promise<RequestList> {
  return invoke("list_requests", { workspace });
}

export function saveRequest(
  workspace: string,
  request: SavedRequest,
): Promise<string> {
  return invoke("save_request", { workspace, request });
}

export function deleteRequest(workspace: string, id: string): Promise<void> {
  return invoke("delete_request", { workspace, id });
}

export function importPostman(
  workspace: string,
  json: string,
): Promise<ImportReport> {
  return invoke("import_postman", { workspace, json });
}

export function exportBundle(workspace: string): Promise<string> {
  return invoke("export_bundle", { workspace });
}

export function importBundle(
  workspace: string,
  json: string,
): Promise<ImportReport> {
  return invoke("import_bundle", { workspace, json });
}

export function readTextFileForImport(path: string): Promise<string> {
  return invoke("read_text_file_for_import", { path });
}

export function writeTextFileForExport(
  path: string,
  contents: string,
): Promise<void> {
  return invoke("write_text_file_for_export", { path, contents });
}

export function defaultWorkspaceDir(): Promise<string> {
  return invoke("default_workspace_dir");
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
