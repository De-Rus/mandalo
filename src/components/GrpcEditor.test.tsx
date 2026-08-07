import { EditorView } from "@codemirror/view";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GrpcMethodInfo, MessageShape } from "../lib/api";
import { newDraft, type GrpcDraft } from "../lib/draft";
import { GrpcEditor } from "./GrpcEditor";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const METHODS: GrpcMethodInfo[] = [
  {
    service: "mock.v1.Mock",
    method: "Say",
    input: "mock.v1.EchoRequest",
    output: "mock.v1.EchoResponse",
    clientStreaming: false,
    serverStreaming: false,
  },
  {
    service: "mock.v1.Mock",
    method: "GetUser",
    input: "mock.v1.GetUserRequest",
    output: "mock.v1.GetUserResponse",
    clientStreaming: false,
    serverStreaming: false,
  },
];

const SHAPES: Record<string, MessageShape> = {
  "mock.v1.EchoRequest": {
    name: "mock.v1.EchoRequest",
    fields: [
      { name: "text", type: "string", repeated: false, message: null, enumValues: [] },
      { name: "count", type: "number", repeated: false, message: null, enumValues: [] },
    ],
  },
  "mock.v1.GetUserRequest": {
    name: "mock.v1.GetUserRequest",
    fields: [
      { name: "id", type: "string", repeated: false, message: null, enumValues: [] },
      { name: "tags", type: "string", repeated: true, message: null, enumValues: [] },
    ],
  },
};

function Harness({ start }: { start: Partial<GrpcDraft> }) {
  const [grpc, setGrpc] = useState<GrpcDraft>({
    ...newDraft("g", "grpc").grpc,
    protoPaths: "protos/mock.proto",
    ...start,
  });
  return (
    <>
      <GrpcEditor tab="Proto" grpc={grpc} onChange={setGrpc} />
      <GrpcEditor tab="Message" grpc={grpc} onChange={setGrpc} />
    </>
  );
}

async function loadAndPick(start: Partial<GrpcDraft>, value: string) {
  const view = render(<Harness start={start} />);
  fireEvent.click(screen.getByText("Load methods"));
  await screen.findByRole("option", { name: /GetUser/ });
  fireEvent.change(screen.getByRole("combobox"), { target: { value } });
  return view.container;
}

function message(container: HTMLElement): string {
  const content = container.querySelector(".grpc-message .cm-content");
  if (content === null) throw new Error("the gRPC message editor is not mounted");
  return EditorView.findFromDOM(content as HTMLElement)!.state.doc.toString();
}

describe("gRPC method selection", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      if (command === "list_grpc_methods") return Promise.resolve(METHODS);
      if (command === "describe_message") {
        const shape = SHAPES[args.typeName as string];
        return shape
          ? Promise.resolve(shape)
          : Promise.reject(new Error(`unknown type ${String(args.typeName)}`));
      }
      return Promise.resolve(undefined);
    });
  });

  afterEach(cleanup);

  it("fills an empty message with the new method's skeleton", async () => {
    const container = await loadAndPick({ message: "" }, "mock.v1.Mock/GetUser");

    await waitFor(() =>
      expect(JSON.parse(message(container))).toEqual({ id: "", tags: [] }),
    );
  });

  it("replaces the previous method's untouched skeleton", async () => {
    const container = await loadAndPick(
      {
        service: "mock.v1.Mock",
        method: "Say",
        message: '{\n  "text": "",\n  "count": 0\n}',
      },
      "mock.v1.Mock/GetUser",
    );

    await waitFor(() =>
      expect(JSON.parse(message(container))).toEqual({ id: "", tags: [] }),
    );
  });

  it("never overwrites a message the user wrote, and offers the example instead", async () => {
    const container = await loadAndPick(
      {
        service: "mock.v1.Mock",
        method: "Say",
        message: '{"text": "hola", "count": 21}',
      },
      "mock.v1.Mock/GetUser",
    );

    const offer = await screen.findAllByText(
      "Insert example message for GetUser",
    );
    expect(message(container)).toBe('{"text": "hola", "count": 21}');

    fireEvent.click(offer[0]);
    await waitFor(() =>
      expect(JSON.parse(message(container))).toEqual({ id: "", tags: [] }),
    );
  });

  it("leaves the message alone when the backend cannot describe the type", async () => {
    invoke.mockImplementation((command: string) =>
      command === "list_grpc_methods"
        ? Promise.resolve(METHODS)
        : Promise.reject(new Error("describe_message is not available")),
    );
    const container = await loadAndPick({ message: "" }, "mock.v1.Mock/GetUser");

    await waitFor(() => expect(screen.getByRole("combobox")).toBeTruthy());
    expect(message(container)).toBe("");
    expect(screen.queryByText(/Insert example message/)).toBeNull();
  });
});
