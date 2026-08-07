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
  | { type: "apikey"; key: string; value: string; placement: string }
  /** `# @auth inherited`: a collection-wide default the request did not ask for. */
  | { type: "inherited"; auth: Auth };

/** `# @reconnect off` on a stream request; absent means the engine's default. */
export interface StreamOptions {
  autoReconnect?: boolean;
}

export interface Scripts {
  pre?: string;
  post?: string;
}

export interface GraphqlBody {
  query: string;
  variables: string;
}

/** One multipart part: a text field carries `value`, a file field carries `files`. */
export interface FormDataRowModel {
  key: string;
  value?: string;
  files?: string[];
  contentType?: string;
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
  /** A `< ./file` body: workspace-relative, and only the CLI can read it off disk. */
  bodyFile?: string;
  /** A multipart/form-data body. File parts are workspace-relative, CLI-only. */
  formdata?: FormDataRowModel[];
  stream?: StreamOptions;
  headers: [string, string][];
  auth: Auth;
  graphql?: GraphqlBody;
  grpc?: GrpcRequest;
  scripts: Scripts;
  tests: TestAssertion[];
  captures: Capture[];
}

export function isRequestKind(value: string): value is RequestKind {
  return (REQUEST_KINDS as readonly string[]).includes(value);
}

export function isCaptureScope(value: string): value is CaptureScope {
  return (CAPTURE_SCOPES as readonly string[]).includes(value);
}
