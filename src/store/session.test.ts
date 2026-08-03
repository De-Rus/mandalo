import { beforeEach, describe, expect, it, vi } from "vitest";
import { newDraft } from "../lib/draft";
import { useSession } from "./session";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("session store send", () => {
  beforeEach(() => {
    invoke.mockReset();
    useSession.setState({ responses: {} });
  });

  it("stores the http response keyed by request id", async () => {
    const data = {
      status: 200,
      statusText: "OK",
      headers: [],
      body: "{}",
      durationMs: 12,
      sizeBytes: 2,
    };
    invoke.mockResolvedValueOnce(data);
    const draft = newDraft();
    draft.url = "https://x.dev";

    await useSession.getState().send(draft, { a: "1" });

    expect(invoke).toHaveBeenCalledWith("send_request", {
      spec: expect.objectContaining({ url: "https://x.dev", vars: { a: "1" } }),
    });
    expect(useSession.getState().responses[draft.id]).toEqual({
      phase: "http",
      data,
    });
  });

  it("routes grpc drafts through send_grpc", async () => {
    invoke.mockResolvedValueOnce({ body: "{}", durationMs: 3 });
    const draft = newDraft();
    draft.kind = "grpc";
    draft.url = "http://localhost:50051";
    draft.grpc.protoPaths = "/a.proto";

    await useSession.getState().send(draft, {});

    expect(invoke).toHaveBeenCalledWith(
      "send_grpc",
      expect.objectContaining({
        spec: expect.objectContaining({ protoPaths: ["/a.proto"] }),
      }),
    );
    expect(useSession.getState().responses[draft.id].phase).toBe("grpc");
  });

  it("captures invoke string errors as a readable error state", async () => {
    invoke.mockRejectedValueOnce("connection refused");
    const draft = newDraft();
    draft.url = "https://down.dev";

    await useSession.getState().send(draft, {});

    expect(useSession.getState().responses[draft.id]).toEqual({
      phase: "error",
      message: "connection refused",
    });
  });
});
