import { describe, expect, it, vi } from "vitest";
import { invoke } from "./invoke";
import { PROTOCOL_VERSION, type CallMessage } from "./protocol";

const sent: CallMessage[] = [];

vi.stubGlobal("acquireVsCodeApi", () => ({
  postMessage: (message: unknown) => void sent.push(message as CallMessage),
}));

describe("invoke", () => {
  it("forwards a supported command over the bridge", () => {
    void invoke("load_document", { a: 1 });
    expect(sent[sent.length - 1]).toMatchObject({
      v: PROTOCOL_VERSION,
      type: "call",
      command: "load_document",
      args: { a: 1 },
    });
  });

  it("refuses a desktop-only command with an explanation, without a round trip", async () => {
    const before = sent.length;
    await expect(invoke("import_postman")).rejects.toThrow(/desktop app/);
    await expect(invoke("list_grpc_methods")).rejects.toThrow(/desktop app/);
    await expect(invoke("list_workspaces")).rejects.toThrow(/sidebar/);
    expect(sent).toHaveLength(before);
  });
});
