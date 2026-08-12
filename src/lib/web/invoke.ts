import type {
  CollectionInfo,
  CollectionList,
  Environment,
  EnvironmentViewList,
  GrpcMethodInfo,
  GrpcResponse,
  GrpcSpec,
  ImportReport,
  MessageShape,
  SavedPath,
  SavedRequest,
  ScriptContext,
  ScriptOutcome,
  StepResult,
  RemoteOrigin,
  RemoteReview,
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
import { withWriteLock } from "./locks";
import * as mounts from "./mounts";
import * as remote from "./remote";
import { webRunRequest } from "./run";
import { persistOnFirstSave } from "./storage";
import { publish, type WorkspaceChange } from "./sync";
import type { Vfs } from "./vfs";
import { webExecuteScript } from "./script";
import { webSend } from "./send";
import type { Outgoing, StreamEvent, StreamSpec } from "../stream";
import {
  webStreamClose,
  webStreamList,
  webStreamOpen,
  webStreamSend,
  webStreamStatus,
} from "./stream";
import * as ws from "./workspace";

const NO_KEYCHAIN =
  "Values that are not shared live in a file on your machine (~/.config/mandalo/secrets.toml), which a web page cannot reach. Open this workspace in the Mándalo desktop app to store or clear one — the browser can still declare it and bind it to a host.";

const NO_IMPORTER =
  "Importing a Postman, OpenAPI or bundle file needs the Mándalo desktop app. In the browser you can open a folder and keep .http files there directly.";

const NO_EXPORTER =
  "Building a bundle — and scanning it for credentials before anything is written — needs the Mándalo desktop app. Open this workspace there to export it.";

const NO_FILESYSTEM =
  "Reading and writing arbitrary files needs the desktop app. In the browser, use “Open folder…” to work directly on a real directory instead.";

const NO_GIT =
  "Git status and sync need the Mándalo desktop app — a browser tab has no working copy of your repository.";

const NO_CURL =
  "Generating a curl command with the same secrets Send would use needs the Mándalo desktop app.";

type Args = Record<string, any>;

/**
 * A remote collection that has been read but not opened. The review on screen
 * describes exactly these bytes, so adopting writes them rather than fetching
 * again — a second fetch could return something else.
 */
const PENDING = new Map<string, remote.RemoteFetch>();

function vfsOf(args: Args) {
  return mounts.vfsFor(args.workspace as string);
}

type Change = Omit<WorkspaceChange, "workspace">;

async function mutate<T>(
  args: Args,
  run: (vfs: Vfs) => Promise<T>,
  change: (value: T) => Change,
): Promise<T> {
  const workspace = args.workspace as string;
  await mounts.assertWritable(workspace);
  const vfs = mounts.vfsFor(workspace);
  const value = await withWriteLock(vfs.id, () => run(vfs));
  if (vfs.kind === "browser") void persistOnFirstSave();
  publish({ workspace, ...change(value) });
  return value;
}

const TREE: Change = { scope: "tree" };
const ENVIRONMENTS: Change = { scope: "environments" };

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

  add_sample_collection: (a): Promise<string> =>
    mutate(a, (vfs) => mounts.addSampleCollection(vfs), () => TREE),

  seed_workspace: async (): Promise<ImportReport | null> => null,

  review_remote: async (a): Promise<RemoteReview> => {
    const fetched = await remote.fetchRemote(remote.parseSource(a.source as string));
    const review = await remote.reviewFetch(fetched);
    PENDING.set(review.token, fetched);
    return review;
  },

  adopt_remote: async (a): Promise<WorkspaceInfo> => {
    const token = a.token as string;
    const fetched = PENDING.get(token);
    if (fetched === undefined)
      throw new Error(
        "The collection changed since it was reviewed — look at it again before it opens.",
      );
    PENDING.delete(token);
    return mounts.openRemote(fetched);
  },

  remote_origin: (a): Promise<RemoteOrigin | null> =>
    mounts.originOf(a.workspace as string),

  save_workspace_copy: (a): Promise<WorkspaceInfo> =>
    mounts.saveCopy(a.workspace as string, a.name as string),

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
    mutate(
      a,
      (vfs) =>
        ws.bindSecretHost(vfs, a.env as string, a.key as string, a.host as string),
      () => ENVIRONMENTS,
    ),

  delete_var: (a): Promise<void> =>
    mutate(
      a,
      (vfs) => ws.deleteVar(vfs, a.env as string, a.key as string),
      () => ENVIRONMENTS,
    ),

  save_environment: (a): Promise<string> =>
    mutate(
      a,
      (vfs) => ws.saveEnvironment(vfs, a.env as Environment),
      () => ENVIRONMENTS,
    ),

  delete_environment: (a): Promise<void> =>
    mutate(
      a,
      (vfs) => ws.deleteEnvironment(vfs, a.name as string),
      () => ENVIRONMENTS,
    ),

  list_collections: (a): Promise<CollectionList> => ws.listCollections(vfsOf(a)),

  create_collection: (a): Promise<CollectionInfo> =>
    mutate(a, (vfs) => ws.createCollection(vfs, a.name as string), () => TREE),

  rename_collection: (a): Promise<CollectionInfo> =>
    mutate(
      a,
      (vfs) => ws.renameCollection(vfs, a.slug as string, a.name as string),
      () => TREE,
    ),

  delete_collection: (a): Promise<void> =>
    mutate(a, (vfs) => ws.deleteCollection(vfs, a.slug as string), () => TREE),

  list_tree: (a): Promise<Tree> => ws.listTree(vfsOf(a)),

  save_request: (a): Promise<SavedPath> =>
    mutate(
      a,
      async (vfs) => ({
        path: await ws.saveRequest(
          vfs,
          a.collection as string,
          (a.path as string | null) ?? null,
          (a.folder as string | null) ?? null,
          a.request as SavedRequest,
        ),
      }),
      ({ path }) =>
        a.path === null
          ? TREE
          : { scope: "request", collection: a.collection as string, path },
    ),

  load_request: (a): Promise<SavedRequest> =>
    ws.loadRequest(vfsOf(a), a.collection as string, a.path as string),

  delete_request: (a): Promise<void> =>
    mutate(
      a,
      (vfs) => ws.deleteRequest(vfs, a.collection as string, a.path as string),
      () => TREE,
    ),

  create_folder: (a): Promise<void> =>
    mutate(
      a,
      (vfs) => ws.createFolder(vfs, a.collection as string, a.path as string),
      () => TREE,
    ),

  delete_folder: (a): Promise<void> =>
    mutate(
      a,
      (vfs) => ws.deleteFolder(vfs, a.collection as string, a.path as string),
      () => TREE,
    ),

  rename_folder: (a): Promise<SavedPath> =>
    mutate(
      a,
      async (vfs) => ({
        path: await ws.renameFolder(
          vfs,
          a.collection as string,
          a.path as string,
          a.name as string,
        ),
      }),
      () => TREE,
    ),

  // Reordering writes the collection manifest and rewrites request files; the
  // browser build has none of that yet. Saying so beats "unknown command".
  reorder_requests: (): Promise<never> =>
    Promise.reject(
      new Error(
        "Reordering requests is not available in the browser build yet — use the desktop app",
      ),
    ),

  move_request: (a): Promise<SavedPath> =>
    mutate(
      a,
      async (vfs) => ({
        path: await ws.moveRequest(
          vfs,
          a.collection as string,
          a.from as string,
          a.toFolder as string,
        ),
      }),
      () => TREE,
    ),

  import_postman: () => {
    throw new Error(NO_IMPORTER);
  },
  import_openapi: () => {
    throw new Error(NO_IMPORTER);
  },
  import_bundle: () => {
    throw new Error(NO_IMPORTER);
  },
  plan_export: () => {
    throw new Error(NO_EXPORTER);
  },
  run_export: () => {
    throw new Error(NO_EXPORTER);
  },
  workspace_share: () => null,
  set_workspace_share: () => {
    throw new Error(NO_EXPORTER);
  },
  read_text_file_for_import: () => {
    throw new Error(NO_FILESYSTEM);
  },
  write_text_file_for_export: () => {
    throw new Error(NO_FILESYSTEM);
  },

  git_status: () => {
    throw new Error(NO_GIT);
  },
  git_init: () => {
    throw new Error(NO_GIT);
  },
  git_clone: () => {
    throw new Error(NO_GIT);
  },
  plan_sync: () => {
    throw new Error(NO_GIT);
  },
  run_sync: () => {
    throw new Error(NO_GIT);
  },
  conflict_previews: () => {
    throw new Error(NO_GIT);
  },
  apply_conflict_choices: () => {
    throw new Error(NO_GIT);
  },
  github_status: () => ({ connected: false, login: null }),
  github_start_login: () => {
    throw new Error(NO_GIT);
  },
  github_poll_login: () => {
    throw new Error(NO_GIT);
  },
  github_logout: () => {
    throw new Error(NO_GIT);
  },
  github_list_repos: () => {
    throw new Error(NO_GIT);
  },
  github_create_repo: () => {
    throw new Error(NO_GIT);
  },
  to_curl: () => {
    throw new Error(NO_CURL);
  },

  stream_open: (a) =>
    webStreamOpen(
      a.spec as StreamSpec,
      a.channel as { onmessage?: (event: StreamEvent) => void },
    ),
  stream_send: (a) => webStreamSend(a.streamId as string, a.payload as Outgoing),
  stream_close: (a) => webStreamClose(a.streamId as string),
  stream_status: (a) => webStreamStatus(a.streamId as string),
  stream_list: () => webStreamList(),
};

/**
 * The browser stand-in for Tauri's typed channel. The engine here runs in this
 * same page, so a channel is just the callback the caller handed over — but the
 * shape has to match, or the components would have to know which host they are
 * on.
 */
export class Channel<T> {
  onmessage: ((message: T) => void) | null = null;
}

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
