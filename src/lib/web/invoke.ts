import type {
  CollectionInfo,
  CollectionList,
  Environment,
  EnvironmentViewList,
  GrpcMethodInfo,
  GrpcResponse,
  GrpcSpec,
  MessageShape,
  SavedPath,
  SavedRequest,
  ScriptContext,
  ScriptOutcome,
  StepResult,
  Tree,
  WorkspaceInfo,
  WorkspaceList,
  WorkspaceOpen,
} from "../api";
import {
  webDescribeMessage,
  webListGrpcMethods,
  webSendGrpc,
} from "./grpc";
import * as mounts from "./mounts";
import { webRunRequest } from "./run";
import { webExecuteScript } from "./script";
import { webSend } from "./send";
import * as ws from "./workspace";

const NO_KEYCHAIN =
  "Secret values live in the OS keychain, which a web page cannot reach. Open this workspace in the Mándalo desktop app to store or clear a secret — the browser can still declare one and bind it to a host.";

const NO_FILESYSTEM =
  "Reading and writing arbitrary files needs the desktop app. In the browser, use “Open folder…” to work directly on a real directory instead.";

type Args = Record<string, any>;

function vfsOf(args: Args) {
  return mounts.vfsFor(args.workspace as string);
}

const handlers: Record<string, (a: Args) => unknown> = {
  send_request: (a) => webSend(a.spec),

  list_grpc_methods: async (a): Promise<GrpcMethodInfo[]> =>
    webListGrpcMethods(await mounts.activeVfs(), a.protoPaths as string[]),

  describe_message: async (a): Promise<MessageShape> =>
    webDescribeMessage(
      await mounts.activeVfs(),
      a.protoPaths as string[],
      a.typeName as string,
    ),

  send_grpc: async (a): Promise<GrpcResponse> =>
    webSendGrpc(await mounts.activeVfs(), a.spec as GrpcSpec),

  execute_script: (a): Promise<ScriptOutcome> =>
    webExecuteScript(a.source as string, a.context as ScriptContext),

  run_request_draft: (a): Promise<StepResult> =>
    webRunRequest(
      vfsOf(a),
      a.request as SavedRequest,
      (a.env as string | null) ?? null,
    ),

  list_workspaces: async (): Promise<WorkspaceList> => {
    await mounts.seedIfEmpty();
    return mounts.list();
  },

  default_workspace_dir: (): string => mounts.BROWSER_PATH,

  create_workspace: async (a): Promise<WorkspaceInfo> =>
    mounts.claim(a.path as string) ?? (await mounts.openFolder()),

  open_workspace: async (a): Promise<WorkspaceOpen> => ({
    workspace: mounts.claim(a.path as string) ?? (await mounts.openFolder()),
    migrated: [],
  }),

  set_active_workspace: (a): Promise<WorkspaceInfo> =>
    mounts.setActive(a.id as string),

  remove_workspace: (a): Promise<void> => mounts.forget(a.id as string),

  list_environments: (a): Promise<EnvironmentViewList> =>
    ws.listEnvironments(vfsOf(a)),

  set_secret: () => {
    throw new Error(NO_KEYCHAIN);
  },

  clear_secret: () => {
    throw new Error(NO_KEYCHAIN);
  },

  secret_status: (a): Promise<Record<string, boolean>> =>
    ws.secretStatus(vfsOf(a), a.env as string),

  bind_secret_host: (a): Promise<string[]> =>
    ws.bindSecretHost(vfsOf(a), a.env as string, a.key as string, a.host as string),

  delete_var: (a): Promise<void> =>
    ws.deleteVar(vfsOf(a), a.env as string, a.key as string),

  save_environment: (a): Promise<string> =>
    ws.saveEnvironment(vfsOf(a), a.env as Environment),

  delete_environment: (a): Promise<void> =>
    ws.deleteEnvironment(vfsOf(a), a.name as string),

  list_collections: (a): Promise<CollectionList> => ws.listCollections(vfsOf(a)),

  create_collection: (a): Promise<CollectionInfo> =>
    ws.createCollection(vfsOf(a), a.name as string),

  rename_collection: (a): Promise<CollectionInfo> =>
    ws.renameCollection(vfsOf(a), a.slug as string, a.name as string),

  delete_collection: (a): Promise<void> =>
    ws.deleteCollection(vfsOf(a), a.slug as string),

  list_tree: (a): Promise<Tree> => ws.listTree(vfsOf(a)),

  save_request: async (a): Promise<SavedPath> => ({
    path: await ws.saveRequest(
      vfsOf(a),
      a.collection as string,
      (a.path as string | null) ?? null,
      (a.folder as string | null) ?? null,
      a.request as SavedRequest,
    ),
  }),

  load_request: (a): Promise<SavedRequest> =>
    ws.loadRequest(vfsOf(a), a.collection as string, a.path as string),

  delete_request: (a): Promise<void> =>
    ws.deleteRequest(vfsOf(a), a.collection as string, a.path as string),

  create_folder: (a): Promise<void> =>
    ws.createFolder(vfsOf(a), a.collection as string, a.path as string),

  delete_folder: (a): Promise<void> =>
    ws.deleteFolder(vfsOf(a), a.collection as string, a.path as string),

  rename_folder: async (a): Promise<SavedPath> => ({
    path: await ws.renameFolder(
      vfsOf(a),
      a.collection as string,
      a.path as string,
      a.name as string,
    ),
  }),

  move_request: async (a): Promise<SavedPath> => ({
    path: await ws.moveRequest(
      vfsOf(a),
      a.collection as string,
      a.from as string,
      a.toFolder as string,
    ),
  }),

  import_postman: () => {
    throw new Error(NO_FILESYSTEM);
  },
  import_bundle: () => {
    throw new Error(NO_FILESYSTEM);
  },
  export_bundle: () => {
    throw new Error(NO_FILESYSTEM);
  },
  read_text_file_for_import: () => {
    throw new Error(NO_FILESYSTEM);
  },
  write_text_file_for_export: () => {
    throw new Error(NO_FILESYSTEM);
  },
};

export function invoke<T>(command: string, args: Args = {}): Promise<T> {
  const handler = handlers[command];
  if (!handler)
    return Promise.reject(
      new Error(`"${command}" is not available in the browser build`),
    );
  try {
    return Promise.resolve(handler(args ?? {}) as T);
  } catch (e) {
    return Promise.reject(e instanceof Error ? e : new Error(String(e)));
  }
}
