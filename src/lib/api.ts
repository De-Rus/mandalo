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

export type ProtoFieldType =
  | "string"
  | "number"
  | "bool"
  | "bytes"
  | "enum"
  | "message";

export interface ProtoField {
  name: string;
  type: ProtoFieldType;
  repeated: boolean;
  message: MessageShape | null;
  enumValues: string[];
}

/** The field tree of one proto message, deep enough to write an example by. */
export interface MessageShape {
  name: string;
  fields: ProtoField[];
}

export interface GrpcSpec {
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

/** The plain-value shape `save_environment` accepts. Never carries a secret. */
export interface Environment {
  name: string;
  vars: Record<string, string>;
}

/** What the UI may know about a variable. A secret's `value` is always null. */
export interface VarInfo {
  secret: boolean;
  value: string | null;
  hosts?: string[];
  set: boolean;
}

export interface EnvironmentView {
  name: string;
  vars: Record<string, VarInfo>;
}

export interface EnvironmentViewList {
  items: EnvironmentView[];
  skipped: string[];
}

export interface Finding {
  path: string;
  line: number;
  rule: string;
  excerpt: string;
}

export interface ExportBundle {
  json: string;
  findings: Finding[];
}

export interface GitHygiene {
  gitignoreWritten: boolean;
  hookInstalled: boolean;
}

export interface Scripts {
  pre: string | null;
  post: string | null;
}

export type StatusOp = "eq" | "ne" | "lt" | "gt";
export type JsonOp =
  | "eq"
  | "ne"
  | "exists"
  | "absent"
  | "contains"
  | "matches"
  | "lt"
  | "gt"
  | "len";
export type HeaderOp = "exists" | "absent" | "eq" | "contains" | "matches";
export type DurationOp = "lt" | "gt";

export type TestAssertion =
  | { kind: "status"; op: StatusOp; value: number }
  | { kind: "json"; path: string; op: JsonOp; value?: unknown }
  | { kind: "header"; name: string; op: HeaderOp; value?: string | null }
  | { kind: "duration"; op: DurationOp; value: number };

export interface TestResult {
  name: string;
  passed: boolean;
  detail: string | null;
}

export type CaptureScope = "run" | "session" | "persist";

export interface Capture {
  from: string;
  into: string;
  scope: CaptureScope;
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
  description?: string | null;
  body?: string | null;
  headers: [string, string][];
  auth: Auth;
  graphql?: { query: string; variables: string } | null;
  grpc?: SavedGrpc | null;
  scripts?: Scripts;
  tests?: TestAssertion[];
  captures?: Capture[];
}

export interface WorkspaceInfo {
  id: string;
  path: string;
  name: string;
}

export interface WorkspaceList {
  items: WorkspaceInfo[];
  active: string;
}

export interface WorkspaceOpen {
  workspace: WorkspaceInfo;
  migrated: string[];
}

export interface CollectionInfo {
  id: string;
  slug: string;
  name: string;
}

export interface CollectionList {
  items: CollectionInfo[];
  skipped: string[];
}

export interface RequestSummary {
  id: string;
  name: string;
  kind: Kind;
  method: string;
  path: string;
}

export interface FolderNode {
  name: string;
  path: string;
  folders: FolderNode[];
  requests: RequestSummary[];
}

export interface CollectionNode {
  id: string;
  slug: string;
  name: string;
  folders: FolderNode[];
  requests: RequestSummary[];
}

export interface Tree {
  collections: CollectionNode[];
  skipped: string[];
}

export interface SavedPath {
  path: string;
}

export interface ImportReport {
  imported: number;
  collections: number;
  environments: number;
  skipped: string[];
  warnings: string[];
  summary: string;
}

export interface ScriptRequest {
  method: string;
  url: string;
  headers: [string, string][];
  body: string | null;
}

export interface ScriptResponse {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: string;
  durationMs: number;
}

export interface ScriptContext {
  vars: Record<string, string>;
  requestName: string;
  request: ScriptRequest;
  response?: ScriptResponse | null;
}

export interface ScriptTest {
  name: string;
  passed: boolean;
  error: string | null;
}

export interface ScriptOutcome {
  varSets: Record<string, string>;
  varUnsets: string[];
  tests: ScriptTest[];
  logs: string[];
  requestPatch: ScriptRequest | null;
}

export interface CaptureOutcome {
  from: string;
  into: string;
  value: string;
  scope: CaptureScope;
}

/** A secret this request used that no host owns yet, and the host it went to. */
export interface UnboundSecret {
  name: string;
  env: string;
  host: string;
}

/**
 * Everything one run produced. `varSets` never carries a value the environment
 * declares secret — those arrive as names in `secretVarSets`, because a secret
 * value has no business crossing the IPC boundary or landing in a file.
 */
export interface StepResult {
  requestName: string;
  path: string;
  method: string;
  url: string;
  response: ResponseData | null;
  grpc: GrpcResponse | null;
  tests: TestResult[];
  scriptTests: ScriptTest[];
  logs: string[];
  captured: Record<string, string>;
  captures: CaptureOutcome[];
  unboundSecrets: UnboundSecret[];
  varSets: Record<string, string>;
  varUnsets: string[];
  secretVarSets: string[];
  passed: boolean;
  durationMs: number;
  error: string | null;
  errorCode: string | null;
}

export function sendRequest(spec: RequestSpec): Promise<ResponseData> {
  return invoke("send_request", { spec });
}

export function listGrpcMethods(protoPaths: string[]): Promise<GrpcMethodInfo[]> {
  return invoke("list_grpc_methods", { protoPaths });
}

/**
 * PENDING BACKEND: no `describe_message` command exists yet, in either build.
 * The editor treats a rejection as "no example available" and never guesses a
 * shape from the proto text, so this is the one place that has to change once
 * the command lands.
 */
export function describeMessage(
  protoPaths: string[],
  typeName: string,
): Promise<MessageShape> {
  return invoke("describe_message", { protoPaths, typeName });
}

export function sendGrpc(spec: GrpcSpec): Promise<GrpcResponse> {
  return invoke("send_grpc", { spec });
}

export function executeScript(
  source: string,
  context: ScriptContext,
): Promise<ScriptOutcome> {
  return invoke("execute_script", { source, context });
}

/**
 * Runs what the editor has on screen — scripts, secrets, declarative tests and
 * captures — through the same runner `mandalo run` uses. The request travels as
 * a payload, so an unsaved edit runs exactly as it reads.
 */
export function runRequestDraft(
  workspace: string,
  request: SavedRequest,
  env: string | null,
): Promise<StepResult> {
  return invoke("run_request_draft", { workspace, request, env });
}

export function listWorkspaces(): Promise<WorkspaceList> {
  return invoke("list_workspaces", {});
}

export function createWorkspace(
  path: string,
  name: string,
): Promise<WorkspaceInfo> {
  return invoke("create_workspace", { path, name });
}

export function openWorkspace(path: string): Promise<WorkspaceOpen> {
  return invoke("open_workspace", { path });
}

export function setActiveWorkspace(id: string): Promise<WorkspaceInfo> {
  return invoke("set_active_workspace", { id });
}

export function removeWorkspace(id: string): Promise<void> {
  return invoke("remove_workspace", { id });
}

export function defaultWorkspaceDir(): Promise<string> {
  return invoke("default_workspace_dir", {});
}

export function listEnvironments(
  workspace: string,
): Promise<EnvironmentViewList> {
  return invoke("list_environments", { workspace });
}

export function setSecret(
  workspace: string,
  env: string,
  key: string,
  value: string,
): Promise<void> {
  return invoke("set_secret", { workspace, env, key, value });
}

export function clearSecret(
  workspace: string,
  env: string,
  key: string,
): Promise<void> {
  return invoke("clear_secret", { workspace, env, key });
}

export function secretStatus(
  workspace: string,
  env: string,
): Promise<Record<string, boolean>> {
  return invoke("secret_status", { workspace, env });
}

export function bindSecretHost(
  workspace: string,
  env: string,
  key: string,
  host: string,
): Promise<string[]> {
  return invoke("bind_secret_host", { workspace, env, key, host });
}

export function deleteVar(
  workspace: string,
  env: string,
  key: string,
): Promise<void> {
  return invoke("delete_var", { workspace, env, key });
}

export function ensureGitHygiene(workspace: string): Promise<GitHygiene> {
  return invoke("ensure_git_hygiene", { workspace });
}

export function installPrecommitHook(workspace: string): Promise<void> {
  return invoke("install_precommit_hook", { workspace });
}

export function scanWorkspace(workspace: string): Promise<Finding[]> {
  return invoke("scan_workspace", { workspace });
}

export function saveEnvironment(
  workspace: string,
  env: Environment,
): Promise<string> {
  return invoke("save_environment", { workspace, env });
}

export function deleteEnvironment(
  workspace: string,
  name: string,
): Promise<void> {
  return invoke("delete_environment", { workspace, name });
}

export function listCollections(workspace: string): Promise<CollectionList> {
  return invoke("list_collections", { workspace });
}

export function createCollection(
  workspace: string,
  name: string,
): Promise<CollectionInfo> {
  return invoke("create_collection", { workspace, name });
}

export function renameCollection(
  workspace: string,
  slug: string,
  name: string,
): Promise<CollectionInfo> {
  return invoke("rename_collection", { workspace, slug, name });
}

export function deleteCollection(
  workspace: string,
  slug: string,
): Promise<void> {
  return invoke("delete_collection", { workspace, slug });
}

export function listTree(workspace: string): Promise<Tree> {
  return invoke("list_tree", { workspace });
}

export function saveRequest(
  workspace: string,
  collection: string,
  path: string | null,
  folder: string | null,
  request: SavedRequest,
): Promise<SavedPath> {
  return invoke("save_request", {
    workspace,
    collection,
    path,
    folder,
    request,
  });
}

export function loadRequest(
  workspace: string,
  collection: string,
  path: string,
): Promise<SavedRequest> {
  return invoke("load_request", { workspace, collection, path });
}

export function deleteRequest(
  workspace: string,
  collection: string,
  path: string,
): Promise<void> {
  return invoke("delete_request", { workspace, collection, path });
}

export function createFolder(
  workspace: string,
  collection: string,
  path: string,
): Promise<void> {
  return invoke("create_folder", { workspace, collection, path });
}

export function deleteFolder(
  workspace: string,
  collection: string,
  path: string,
): Promise<void> {
  return invoke("delete_folder", { workspace, collection, path });
}

export function renameFolder(
  workspace: string,
  collection: string,
  path: string,
  name: string,
): Promise<SavedPath> {
  return invoke("rename_folder", { workspace, collection, path, name });
}

export function moveRequest(
  workspace: string,
  collection: string,
  from: string,
  toFolder: string,
): Promise<SavedPath> {
  return invoke("move_request", { workspace, collection, from, toFolder });
}

export function importPostman(
  workspace: string,
  json: string,
): Promise<ImportReport> {
  return invoke("import_postman", { workspace, json });
}

export function exportBundle(workspace: string): Promise<ExportBundle> {
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

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
