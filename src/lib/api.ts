import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Outgoing,
  SavedMessage,
  StreamEvent,
  StreamKind,
  StreamSpec,
  StreamStatus,
  Subscription,
} from "./stream";

export type Kind = "http" | "graphql" | "grpc" | StreamKind;

export type Auth =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apikey"; key: string; value: string; placement: "header" | "query" }
  /** A collection-wide default the request did not ask for. The editor shows the
   * auth it wraps; dropping the wrapper on save would delete the `# @auth
   * inherited` line and the credential under it. */
  | { type: "inherited"; auth: Auth };

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

/** Which layer answered for a variable's value. */
export type VarSource = "file" | "local" | "environment";

/**
 * What the UI may know about a variable. `shared` says where the value lives —
 * the committed file, or only this machine; `secret` says how it is treated.
 * `value` is the **committed** value and is null for anything not shared, so a
 * secret's value never crosses this boundary and a local one never does either.
 */
export interface VarInfo {
  shared: boolean;
  secret: boolean;
  value: string | null;
  hosts?: string[];
  set: boolean;
  source?: VarSource;
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

export interface IncludedRequest {
  path: string;
  name: string;
}

export interface IncludedCollection {
  slug: string;
  name: string;
  requests: IncludedRequest[];
}

export interface ExportIncluded {
  collections: IncludedCollection[];
  environments: string[];
  requestCount: number;
}

export interface ExportExcluded {
  secretValues: number;
  localValues: number;
  withheldNames: string[];
  collections: string[];
  requests: number;
  environments: string[];
}

/**
 * What an export *would* write. `token` pins the plan to the workspace as it was
 * read; `run_export` refuses a token the workspace has moved past, so a review
 * can never approve a bundle other than the one on screen.
 */
export interface ExportPlan {
  included: ExportIncluded;
  excluded: ExportExcluded;
  findings: Finding[];
  bytes: number;
  blocked: boolean;
  token: string;
  format: string;
  warnings: string[];
}

export interface ExportSelection {
  collections?: { slug: string; folders?: string[]; requests?: string[] }[];
  environments?: string[];
}

export interface ExportReceipt {
  path: string;
  bytes: number;
  requests: number;
  collections: number;
  environments: number;
  forced: boolean;
}

/** A document fetched for import, and what came back over the wire. */
export interface FetchedDocument {
  url: string;
  contentType: string | null;
  bytes: number;
  text: string;
  status: number;
}

/** Where a read-only workspace came from. `null` for one the user owns. */
export interface RemoteOrigin {
  label: string;
  url: string;
  commit: string | null;
  fetchedAt: number;
}

export interface RemoteScript {
  collection: string;
  request: string;
  hook: string;
  lines: number;
}

export interface RemoteEnvironment {
  name: string;
  declared: string[];
  sharedValues: number;
  awaitingValues: number;
}

/**
 * Everything a stranger's collection turns out to be, worked out without running
 * any of it. `token` pins the review to the exact bytes that were fetched, so
 * adopting cannot land something other than what was on screen.
 */
export interface RemoteReview {
  origin: RemoteOrigin;
  files: number;
  bytes: number;
  collections: number;
  requests: number;
  environments: RemoteEnvironment[];
  hosts: string[];
  templatedHosts: string[];
  scripts: RemoteScript[];
  findings: Finding[];
  skipped: string[];
  token: string;
}

export interface GitHygiene {
  gitignoreWritten: boolean;
  hookInstalled: boolean;
}

export interface GitIdentity {
  name: string;
  email: string;
  isFallback: boolean;
}

export interface SyncStatus {
  isRepo: boolean;
  branch: string | null;
  detached: boolean;
  remoteUrl: string | null;
  staged: number;
  unstaged: number;
  untracked: number;
  ahead: number;
  behind: number;
  conflicted: string[];
  dirtyFiles: string[];
  dirtyTotal: number;
  identity: GitIdentity | null;
}

export type FileChange =
  | "new"
  | "modified"
  | "deleted"
  | "renamed"
  | "typeChange"
  | "conflicted";

export type SyncAction =
  | "nothing"
  | "commit"
  | "push"
  | "commitAndPush"
  | "pull"
  | "branchAndPush";

export interface PlannedFile {
  path: string;
  change: FileChange;
  included: boolean;
}

export interface SyncSelection {
  only?: string[] | null;
  except?: string[];
}

export interface SyncPlan {
  action: SyncAction;
  files: PlannedFile[];
  included: number;
  excluded: number;
  remote: string | null;
  branch: string | null;
  targetBranch: string | null;
  ahead: number;
  behind: number;
  conflicted: string[];
  findings: Finding[];
  blocked: boolean;
  identity: GitIdentity;
  token: string;
  shareDir?: string | null;
  conflictItems?: ConflictItem[];
}

export type SyncOutcome =
  | { kind: "nothingToDo" }
  | { kind: "committed"; sha: string; identity: GitIdentity }
  | { kind: "pushed"; sha: string; ahead: number; identity: GitIdentity }
  | { kind: "pulled"; sha: string; behind: number }
  | { kind: "conflicted"; files: string[]; items?: ConflictItem[] }
  | { kind: "rejected"; reason: string };

export interface ConflictSidePreview {
  exists: boolean;
  kind?: string | null;
  method?: string | null;
  name?: string | null;
  detail?: string | null;
  /** Full file text for the line diff UI. */
  text?: string | null;
}

export interface ConflictItem {
  path: string;
  ours: ConflictSidePreview;
  theirs: ConflictSidePreview;
}

export type ConflictChoice = "ours" | "theirs";

export interface ConflictDecision {
  path: string;
  choice: ConflictChoice;
  /** Explicit merged file body (request-by-request picks). */
  content?: string | null;
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

/**
 * The protocol options a realtime request carries in its file. The shape mirrors
 * `StreamSpec` minus the parts a run supplies — url, headers, auth and vars are
 * the request's own fields, exactly as they are for HTTP.
 */
export interface SavedStream {
  subprotocols?: string[];
  autoReconnect?: boolean;
  pingIntervalMs?: number;
  lastEventId?: string | null;
  clientId?: string | null;
  username?: string | null;
  password?: string | null;
  cleanSession?: boolean;
  keepAliveSecs?: number;
  subscriptions?: Subscription[];
  protocolVersion?: "3.1.1" | "5";
  maxMessageBytes?: number;
  maxBufferedEvents?: number;
  maxReconnectAttempts?: number;
  idleTimeoutMs?: number;
  messages?: SavedMessage[];
}

export interface FormRowPayload {
  key: string;
  value?: string;
  enabled?: boolean;
  description?: string | null;
}

export interface FormDataRowPayload {
  key: string;
  value?: string;
  files?: string[];
  contentType?: string | null;
  enabled?: boolean;
  description?: string | null;
}

export type RawLanguage = "json" | "text" | "xml" | "html" | "javascript";

export type BodyPayload =
  | { mode: "raw"; language?: RawLanguage; text: string }
  | { mode: "urlencoded"; rows: FormRowPayload[] }
  | { mode: "formdata"; rows: FormDataRowPayload[] }
  | { mode: "binary"; file: string; contentType?: string | null }
  | { mode: "graphql"; query: string; variables: string };

export interface SavedRequest {
  id: string;
  name: string;
  kind: Kind;
  method: string;
  url: string;
  description?: string | null;
  body?: string | BodyPayload | null;
  /** A `< ./file` body. Only an engine with a filesystem can send one. */
  bodyFile?: string | null;
  headers: [string, string][];
  auth: Auth;
  graphql?: { query: string; variables: string } | null;
  grpc?: SavedGrpc | null;
  stream?: SavedStream | null;
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
  /** Present when opening migrates legacy root-level request files. */
  migrated?: string[];
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

/**
 * Copies a `.proto` into the workspace and answers with the workspace-relative
 * path a request should store. This is the only way a browser tab can supply a
 * proto at all: a page has no filesystem path to give.
 */
export function importProto(
  workspace: string,
  fileName: string,
  contents: string,
): Promise<string> {
  return invoke("import_proto", { workspace, fileName, contents });
}

export function listGrpcMethods(
  workspace: string,
  protoPaths: string[],
): Promise<GrpcMethodInfo[]> {
  return invoke("list_grpc_methods", { workspace, protoPaths });
}

export function describeMessage(
  workspace: string,
  protoPaths: string[],
  typeName: string,
): Promise<MessageShape> {
  return invoke("describe_message", { workspace, protoPaths, typeName });
}

export function sendGrpc(workspace: string, spec: GrpcSpec): Promise<GrpcResponse> {
  return invoke("send_grpc", { workspace, spec });
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

export function gitStatus(workspace: string): Promise<SyncStatus> {
  return invoke("git_status", { workspace });
}

export function gitInit(
  workspace: string,
  remoteUrl?: string | null,
): Promise<void> {
  return invoke("git_init", { workspace, remoteUrl: remoteUrl ?? null });
}

export function gitClone(
  url: string,
  dest: string,
  token?: string | null,
): Promise<void> {
  return invoke("git_clone", { url, dest, token: token ?? null });
}

export function planSync(
  workspace: string,
  selection?: SyncSelection | null,
  branch?: string | null,
): Promise<SyncPlan> {
  return invoke("plan_sync", {
    workspace,
    selection: selection ?? null,
    branch: branch ?? null,
  });
}

export function runSync(
  workspace: string,
  token: string,
  message: string,
  selection?: SyncSelection | null,
  force = false,
): Promise<SyncOutcome> {
  return invoke("run_sync", {
    workspace,
    selection: selection ?? null,
    token,
    message,
    gitToken: null,
    force,
  });
}

export function conflictPreviews(
  workspace: string,
  files: string[],
): Promise<ConflictItem[]> {
  return invoke("conflict_previews", { workspace, files });
}

export function applyConflictChoices(
  workspace: string,
  decisions: ConflictDecision[],
): Promise<void> {
  return invoke("apply_conflict_choices", { workspace, decisions });
}

export interface GithubStatus {
  connected: boolean;
  login: string | null;
}

export interface GithubLoginStart {
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
  handle: string;
}

export interface GithubPoll {
  pending: boolean;
  user: { login: string; name?: string | null; avatarUrl?: string | null } | null;
}

export interface GithubRepo {
  name: string;
  fullName: string;
  private: boolean;
  cloneUrl: string;
  htmlUrl: string;
  defaultBranch: string;
}

export function githubStatus(): Promise<GithubStatus> {
  return invoke("github_status", {});
}

export function githubStartLogin(publicOnly = false): Promise<GithubLoginStart> {
  return invoke("github_start_login", { publicOnly, clientId: null });
}

export function githubPollLogin(handle: string): Promise<GithubPoll> {
  return invoke("github_poll_login", { handle });
}

export function githubLogout(): Promise<void> {
  return invoke("github_logout", {});
}

export function githubListRepos(): Promise<GithubRepo[]> {
  return invoke("github_list_repos", {});
}

export function githubCreateRepo(
  name: string,
  privateRepo = true,
): Promise<GithubRepo> {
  return invoke("github_create_repo", { name, private: privateRepo });
}

export function toCurl(
  workspace: string,
  request: SavedRequest,
  env?: string | null,
): Promise<string> {
  return invoke("to_curl", { workspace, request, env: env ?? null });
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

/**
 * Puts the requests of one folder in the order given. `ordered` has to name
 * every request the folder holds — a partial list is refused rather than
 * guessed at.
 */
export function reorderRequests(
  workspace: string,
  collection: string,
  folder: string,
  ordered: string[],
): Promise<void> {
  return invoke("reorder_requests", { workspace, collection, folder, ordered });
}

export function importPostman(
  workspace: string,
  json: string,
): Promise<ImportReport> {
  return invoke("import_postman", { workspace, json });
}

/** The whole document as text — JSON or YAML, 3.1 / 3.0 / Swagger 2.0. */
export function importOpenapi(
  workspace: string,
  source: string,
): Promise<ImportReport> {
  return invoke("import_openapi", { workspace, source });
}

export type ShareFormat = "native" | "postman";

export interface ShareConfig {
  format: ShareFormat;
  dir?: string | null;
}

export function planExport(
  workspace: string,
  selection?: ExportSelection,
  format?: ShareFormat | null,
): Promise<ExportPlan> {
  return invoke("plan_export", {
    workspace,
    selection: selection ?? null,
    format: format ?? null,
  });
}

export function runExport(
  workspace: string,
  token: string,
  path: string,
  force: boolean,
  selection?: ExportSelection,
  format?: ShareFormat | null,
): Promise<ExportReceipt> {
  return invoke("run_export", {
    workspace,
    selection: selection ?? null,
    token,
    path,
    force,
    format: format ?? null,
  });
}

export function workspaceShare(
  workspace: string,
): Promise<ShareConfig | null> {
  return invoke("workspace_share", { workspace });
}

export function setWorkspaceShare(
  workspace: string,
  format: ShareFormat | null,
  dir?: string | null,
): Promise<unknown> {
  return invoke("set_workspace_share", {
    workspace,
    format,
    dir: dir ?? null,
  });
}

/** Fetched Rust-side so an import URL passes the same egress guard as any request. */
export function fetchTextForImport(url: string): Promise<FetchedDocument> {
  return invoke("fetch_text_for_import", { url });
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

/**
 * Opens a realtime connection. The events arrive on a per-connection channel,
 * never a global event bus, so one stream can never receive another's traffic
 * and dropping the receiver is what tells the engine to hang up.
 */
export async function streamOpen(
  spec: StreamSpec,
  onEvent: (event: StreamEvent) => void,
): Promise<string> {
  const channel = new Channel<StreamEvent>();
  channel.onmessage = onEvent;
  const { streamId } = await invoke<{ streamId: string }>("stream_open", {
    spec,
    channel,
  });
  return streamId;
}

export function streamSend(streamId: string, payload: Outgoing): Promise<void> {
  return invoke("stream_send", { streamId, payload });
}

export function streamClose(streamId: string): Promise<void> {
  return invoke("stream_close", { streamId });
}

export function streamStatus(streamId: string): Promise<StreamStatus> {
  return invoke("stream_status", { streamId });
}

export function streamList(): Promise<StreamStatus[]> {
  return invoke("stream_list", {});
}

export function addSampleCollection(workspace: string): Promise<string> {
  return invoke("add_sample_collection", { workspace });
}

export function seedWorkspace(
  workspace: string,
): Promise<ImportReport | null> {
  return invoke("seed_workspace", { workspace });
}

export function reviewRemote(source: string): Promise<RemoteReview> {
  return invoke("review_remote", { source });
}

export function adoptRemote(token: string, dest: string): Promise<WorkspaceInfo> {
  return invoke("adopt_remote", { token, dest });
}

export function remoteOrigin(workspace: string): Promise<RemoteOrigin | null> {
  return invoke("remote_origin", { workspace });
}

export function saveWorkspaceCopy(
  workspace: string,
  dest: string,
  name: string,
): Promise<WorkspaceInfo> {
  return invoke("save_workspace_copy", { workspace, dest, name });
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
