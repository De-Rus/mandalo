export const REQUEST_KINDS = ["http", "graphql", "grpc"] as const;
export type RequestKind = (typeof REQUEST_KINDS)[number];

export const CAPTURE_SCOPES = ["run", "session", "persist"] as const;
export type CaptureScope = (typeof CAPTURE_SCOPES)[number];

export const AUTH_TYPES = ["none", "bearer", "basic", "apikey"] as const;
export type AuthType = (typeof AUTH_TYPES)[number];

export type Auth =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apikey"; key: string; value: string; placement: string };

export interface Scripts {
  pre?: string;
  post?: string;
}

export interface GraphqlBody {
  query: string;
  variables: string;
}

export interface GrpcRequest {
  protoPaths: string[];
  service: string;
  method: string;
  message: string;
  metadata: [string, string][];
}

export type TestAssertion =
  | { kind: "status"; op: string; value: number }
  | { kind: "json"; path: string; op: string; value?: unknown }
  | { kind: "header"; name: string; op: string; value?: string }
  | { kind: "duration"; op: string; value: number }
  | { kind: string; [key: string]: unknown };

export interface Capture {
  from: string;
  into: string;
  scope: string;
}

export interface RequestModel {
  schemaVersion: number;
  id: string;
  name: string;
  kind: string;
  method: string;
  url: string;
  description?: string;
  body?: string;
  headers: [string, string][];
  auth: Auth;
  graphql?: GraphqlBody;
  grpc?: GrpcRequest;
  scripts: Scripts;
  tests: TestAssertion[];
  captures: Capture[];
}

export interface CollectionManifest {
  schemaVersion: number;
  id: string;
  name: string;
}

export interface WorkspaceManifest {
  schemaVersion: number;
  id: string;
  name: string;
}

export interface EnvironmentModel {
  name: string;
  vars: Record<string, string>;
}

export interface RequestNode {
  id: string;
  name: string;
  kind: string;
  method: string;
  /** `auth/login.http#0` — the file inside the collection plus the block index. */
  relPath: string;
  fsPath: string;
  index: number;
  line: number;
}

export interface FolderNode {
  name: string;
  relPath: string;
  folders: FolderNode[];
  requests: RequestNode[];
}

export interface CollectionNode {
  id: string;
  slug: string;
  name: string;
  dirPath: string;
  manifestPath: string;
  folders: FolderNode[];
  requests: RequestNode[];
}

export interface WorkspaceNode {
  id: string;
  name: string;
  rootPath: string;
  manifestPath: string;
  collections: CollectionNode[];
  environments: EnvironmentModel[];
  skipped: string[];
}

export function isRequestKind(value: string): value is RequestKind {
  return (REQUEST_KINDS as readonly string[]).includes(value);
}

export function isCaptureScope(value: string): value is CaptureScope {
  return (CAPTURE_SCOPES as readonly string[]).includes(value);
}
