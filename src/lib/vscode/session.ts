import type { GrpcResponse, ResponseData, SavedRequest, TestResult } from "../api";
import type { ResponseState, RunResult } from "../../store/session";
import { invoke } from "./invoke";

export interface CliTestOutcome {
  id: string;
  name: string;
  kind: "assertion" | "script";
  passed: boolean;
  detail: string | null;
}

export interface CliCapture {
  from: string;
  into: string;
  value: string;
  scope: string;
}

export interface CliOutcome {
  path: string;
  name: string;
  method: string;
  url: string;
  response: ResponseData | null;
  grpc: GrpcResponse | null;
  tests: CliTestOutcome[];
  captures: CliCapture[];
  logs: string[];
  passed: boolean;
  durationMs: number;
  error: string | null;
  errorCode: string | null;
}

function toResult(test: CliTestOutcome): TestResult {
  return { name: test.name, passed: test.passed, detail: test.detail };
}

export function toResponseState(outcome: CliOutcome): ResponseState {
  const logs = outcome.logs ?? [];
  if (outcome.error) return { phase: "error", message: outcome.error, logs };

  const captured: Record<string, string> = {};
  for (const capture of outcome.captures ?? []) captured[capture.into] = capture.value;
  const run: RunResult = {
    tests: (outcome.tests ?? []).filter((t) => t.kind !== "script").map(toResult),
    scriptTests: (outcome.tests ?? []).filter((t) => t.kind === "script").map(toResult),
    logs,
    captured,
    unboundSecrets: [],
    secretVarSets: [],
    runError: null,
  };

  if (outcome.grpc) return { phase: "grpc", data: outcome.grpc, run };
  if (outcome.response) return { phase: "http", data: outcome.response, run };
  return { phase: "error", message: `${outcome.name || outcome.path} produced no response`, logs };
}

export async function runRequest(request: SavedRequest, env: string | null): Promise<ResponseState> {
  const outcome = await invoke<CliOutcome>("run_request_full", { request, env });
  return toResponseState(outcome);
}
