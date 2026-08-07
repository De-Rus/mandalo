import { bridge } from "./bridge";

const DESKTOP_ONLY: Record<string, string> = {
  list_workspaces:
    "The VS Code editor works on the file you opened. Use the Mándalo sidebar to move around the workspace.",
  create_workspace: "Creating a workspace needs the desktop app or the `mandalo` CLI.",
  open_workspace: "Opening another workspace needs the desktop app — VS Code already has one open.",
  set_active_workspace: "VS Code decides the active workspace; switch folders instead.",
  remove_workspace: "Removing a workspace needs the desktop app.",
  import_postman: "Importing a Postman collection needs the desktop app or `mandalo import`.",
  import_bundle: "Importing a bundle needs the desktop app or `mandalo import`.",
  export_bundle: "Exporting a bundle needs the desktop app or `mandalo export`.",
  list_grpc_methods:
    "Reflecting a proto file needs the desktop app. In VS Code, type the service and method and press Send — the CLI compiles the protos.",
  stream_open:
    "Live connections — WebSocket, SSE and MQTT — need the desktop app or the browser build. The VS Code editor speaks one request at a time over postMessage and has nowhere to keep a socket open.",
  stream_send: "There is no live connection in the VS Code editor to send on.",
  stream_close: "There is no live connection in the VS Code editor to close.",
  stream_status: "There is no live connection in the VS Code editor.",
  stream_list: "There is no live connection in the VS Code editor.",
};

/** The shape `@tauri-apps/api/core` exports, so the seam stays a pure swap. */
export class Channel<T> {
  onmessage: ((message: T) => void) | null = null;
}

export function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const refusal = DESKTOP_ONLY[command];
  if (refusal) return Promise.reject(new Error(refusal));
  return bridge().call<T>(command, args);
}
