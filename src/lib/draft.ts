import type { Capture, Kind, TestAssertion } from "./api";

export interface KVRow {
  id: string;
  key: string;
  value: string;
  enabled: boolean;
}

export type AuthType = "none" | "bearer" | "basic" | "apikey";

export interface AuthDraft {
  type: AuthType;
  token: string;
  username: string;
  password: string;
  key: string;
  value: string;
  placement: "header" | "query";
}

export interface GrpcDraft {
  protoPaths: string;
  service: string;
  method: string;
  message: string;
  metadata: KVRow[];
}

export interface RequestDraft {
  id: string;
  name: string;
  kind: Kind;
  method: string;
  url: string;
  description: string;
  collection: string;
  path: string | null;
  params: KVRow[];
  headers: KVRow[];
  body: string;
  auth: AuthDraft;
  graphqlQuery: string;
  graphqlVariables: string;
  grpc: GrpcDraft;
  preScript: string;
  testScript: string;
  tests: TestAssertion[];
  captures: Capture[];
}

export function uid(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36);
}

export function emptyRow(): KVRow {
  return { id: uid(), key: "", value: "", enabled: true };
}

export function newDraft(
  name = "New Request",
  kind: Kind = "http",
  collection = "",
): RequestDraft {
  return {
    id: uid(),
    name,
    kind,
    method: kind === "http" ? "GET" : "POST",
    url: "",
    description: "",
    collection,
    path: null,
    params: [emptyRow()],
    headers: [emptyRow()],
    body: "",
    auth: {
      type: "none",
      token: "",
      username: "",
      password: "",
      key: "",
      value: "",
      placement: "header",
    },
    graphqlQuery: "",
    graphqlVariables: "",
    grpc: {
      protoPaths: "",
      service: "",
      method: "",
      message: "",
      metadata: [emptyRow()],
    },
    preScript: "",
    testScript: "",
    tests: [],
    captures: [],
  };
}
