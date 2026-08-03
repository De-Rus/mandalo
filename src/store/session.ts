import { create } from "zustand";
import {
  errorMessage,
  sendGrpc,
  sendRequest,
  type GrpcResponse,
  type ResponseData,
} from "../lib/api";
import type { RequestDraft } from "../lib/draft";
import { buildGrpcRequest, buildRequestSpec } from "../lib/spec";

export type ResponseState =
  | { phase: "idle" }
  | { phase: "loading" }
  | { phase: "http"; data: ResponseData }
  | { phase: "grpc"; data: GrpcResponse }
  | { phase: "error"; message: string };

interface SessionState {
  responses: Record<string, ResponseState>;
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  send: (draft: RequestDraft, vars: Record<string, string>) => Promise<void>;
  evictResponse: (id: string) => void;
}

export const useSession = create<SessionState>((set, get) => ({
  responses: {},
  sidebarCollapsed: false,
  toggleSidebar: () =>
    set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  send: async (draft, vars) => {
    if (get().responses[draft.id]?.phase === "loading") return;
    const put = (state: ResponseState) =>
      set((s) => ({ responses: { ...s.responses, [draft.id]: state } }));
    put({ phase: "loading" });
    try {
      if (draft.kind === "grpc") {
        const data = await sendGrpc(buildGrpcRequest(draft, vars));
        put({ phase: "grpc", data });
      } else {
        const data = await sendRequest(buildRequestSpec(draft, vars));
        put({ phase: "http", data });
      }
    } catch (e) {
      put({ phase: "error", message: errorMessage(e) });
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

export function useResponse(id: string | null): ResponseState {
  return useSession((s) =>
    id ? (s.responses[id] ?? IDLE) : IDLE,
  );
}

const IDLE: ResponseState = { phase: "idle" };
