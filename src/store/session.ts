import { create } from "zustand";
import {
  errorMessage,
  runRequestDraft,
  type GrpcResponse,
  type ResponseData,
  type ScriptTest,
  type StepResult,
  type TestResult,
  type UnboundSecret,
} from "../lib/api";
import { toSaved } from "../lib/collection";
import type { RequestDraft } from "../lib/draft";
import { useEnv } from "./env";

export interface RunResult {
  tests: TestResult[];
  scriptTests: TestResult[];
  logs: string[];
  captured: Record<string, string>;
  unboundSecrets: UnboundSecret[];
  secretVarSets: string[];
  runError: string | null;
}

export type ResponseState =
  | { phase: "idle" }
  | { phase: "loading" }
  | { phase: "http"; data: ResponseData; run: RunResult }
  | { phase: "grpc"; data: GrpcResponse; run: RunResult }
  | { phase: "error"; message: string; logs: string[] };

interface SessionState {
  responses: Record<string, ResponseState>;
  send: (draft: RequestDraft) => Promise<void>;
  evictResponse: (id: string) => void;
}

export const EMPTY_RUN: RunResult = {
  tests: [],
  scriptTests: [],
  logs: [],
  captured: {},
  unboundSecrets: [],
  secretVarSets: [],
  runError: null,
};

function toTestResult(test: ScriptTest): TestResult {
  return { name: test.name, passed: test.passed, detail: test.error };
}

function runOf(step: StepResult): RunResult {
  return {
    tests: step.tests ?? [],
    scriptTests: (step.scriptTests ?? []).map(toTestResult),
    logs: step.logs ?? [],
    captured: step.captured ?? {},
    unboundSecrets: step.unboundSecrets ?? [],
    secretVarSets: step.secretVarSets ?? [],
    runError: step.error ?? null,
  };
}

export const useSession = create<SessionState>((set, get) => ({
  responses: {},
  send: async (draft) => {
    if (get().responses[draft.id]?.phase === "loading") return;
    const put = (state: ResponseState) =>
      set((s) => ({ responses: { ...s.responses, [draft.id]: state } }));
    put({ phase: "loading" });

    const { workspace, selected } = useEnv.getState();
    if (!workspace) {
      put({ phase: "error", message: "Workspace not loaded yet", logs: [] });
      return;
    }

    try {
      const step = await runRequestDraft(workspace, toSaved(draft), selected);
      const run = runOf(step);
      if (step.error && !step.grpc && !step.response) {
        put({ phase: "error", message: step.error, logs: run.logs });
        return;
      }
      if (step.grpc) {
        put({ phase: "grpc", data: step.grpc, run });
      } else if (step.response) {
        put({ phase: "http", data: step.response, run });
      } else {
        put({
          phase: "error",
          message: `${draft.name} produced no response`,
          logs: run.logs,
        });
        return;
      }
      await useEnv.getState().applyVarWrites(step.varSets ?? {}, step.varUnsets ?? []);
    } catch (e) {
      put({ phase: "error", message: errorMessage(e), logs: [] });
    }
  },
  evictResponse: (id) =>
    set((s) => {
      if (!(id in s.responses)) return s;
      const responses = { ...s.responses };
      delete responses[id];
      return { responses };
    }),
}));

const IDLE: ResponseState = { phase: "idle" };

export function useResponse(id: string | null): ResponseState {
  return useSession((s) => (id ? (s.responses[id] ?? IDLE) : IDLE));
}
