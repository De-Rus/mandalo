import { randomUUID } from "node:crypto";
import * as path from "node:path";
import * as vscode from "vscode";
import type { CollectionNode, RequestKind, WorkspaceNode } from "./core/model";
import { REQUEST_KINDS } from "./core/model";
import { defaultRequestRelPath, findCollectionFor } from "./core/scan";
import { pickEnvironment, pickWorkspace } from "./env";
import type { Logger } from "./logger";
import { ResultsPanel } from "./results";
import type { MandaloStore } from "./store";
import type { MandaloTestController } from "./testing";
import type { TreeNode } from "./tree";

export interface CommandDeps {
  store: MandaloStore;
  log: Logger;
  tests: MandaloTestController;
  extensionUri: vscode.Uri;
}

interface RequestTarget {
  workspace: WorkspaceNode;
  collection: CollectionNode;
  relPath: string;
}

export interface RequestRef {
  uri: vscode.Uri;
  relPath: string;
}

function slugify(name: string): string {
  const out = name
    .split("")
    .map((char) => (/[A-Za-z0-9]/.test(char) ? char.toLowerCase() : "-"))
    .join("")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
  return out === "" ? "untitled" : out;
}

/** A CodeLens or a tree node knows which block it points at; a bare URI does not. */
function explicitRef(input: unknown): RequestRef | undefined {
  const ref = input as RequestRef | undefined;
  if (ref && typeof ref === "object" && ref.uri instanceof vscode.Uri && typeof ref.relPath === "string") {
    return ref;
  }
  const node = input as TreeNode | undefined;
  if (node && typeof node === "object" && "type" in node && node.type === "request") {
    return { uri: vscode.Uri.file(node.request.fsPath), relPath: node.request.relPath };
  }
  return undefined;
}

function resolveTarget(store: MandaloStore, input: unknown): RequestTarget | undefined {
  const ref = explicitRef(input);
  const uri = ref?.uri ?? toUri(input);
  if (!uri) return undefined;
  const workspace = store.workspaceFor(uri.fsPath);
  if (!workspace) return undefined;
  const collection = findCollectionFor(workspace, uri.fsPath);
  if (!collection) return undefined;
  return {
    workspace,
    collection,
    relPath: ref?.relPath ?? defaultRequestRelPath(collection, uri.fsPath),
  };
}

function toUri(input: unknown): vscode.Uri | undefined {
  if (input instanceof vscode.Uri) return input;
  const node = input as TreeNode | undefined;
  if (node && typeof node === "object" && "type" in node) {
    if (node.type === "request") return vscode.Uri.file(node.request.fsPath);
    if (node.type === "collection") return vscode.Uri.file(node.collection.manifestPath);
    if (node.type === "folder") {
      return vscode.Uri.file(path.join(node.collection.dirPath, node.folder.relPath));
    }
    if (node.type === "workspace") return vscode.Uri.file(node.workspace.rootPath);
  }
  return vscode.window.activeTextEditor?.document.uri;
}

async function requireTarget(
  store: MandaloStore,
  input: unknown,
): Promise<RequestTarget | undefined> {
  const target = resolveTarget(store, input);
  if (!target) {
    void vscode.window.showWarningMessage(
      "Mándalo: this file is not inside a collection of a Mándalo workspace.",
    );
    return undefined;
  }
  return target;
}

async function send(deps: CommandDeps, input: unknown, envOverride?: string | null): Promise<void> {
  const target = await requireTarget(deps.store, input);
  if (!target) return;
  const env =
    envOverride === undefined ? deps.store.selectedEnv(target.workspace) : (envOverride ?? undefined);
  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Window, title: `Mándalo · ${target.relPath}` },
    async () => {
      try {
        const result = await deps.store.executor.send(
          target.workspace,
          target.collection,
          target.relPath,
          env,
          deps.store.envVars(target.workspace) ?? {},
        );
        ResultsPanel.showSend(deps.extensionUri, result);
      } catch (error) {
        deps.log.report(error);
      }
    },
  );
}

async function runCollection(deps: CommandDeps, input: unknown, folder?: string): Promise<void> {
  const target = await requireTarget(deps.store, input);
  if (!target) return;
  const env = deps.store.selectedEnv(target.workspace);
  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Window, title: `Mándalo · ${target.collection.name}` },
    async () => {
      try {
        const result = await deps.store.executor.run(
          target.workspace,
          target.collection,
          { env, folder },
          deps.store.envVars(target.workspace) ?? {},
        );
        ResultsPanel.showRun(deps.extensionUri, result);
      } catch (error) {
        deps.log.report(error);
      }
    },
  );
}

function scaffold(kind: RequestKind, name: string): string {
  if (kind === "grpc") {
    return `### ${name}\n{{grpc_host}}/package.Service/Method\nproto: proto/service.proto\n\n{}\n`;
  }
  if (kind === "graphql") {
    return `### ${name}\nPOST {{base}}/graphql\nContent-Type: application/json\nX-REQUEST-TYPE: GraphQL\n\nquery {\n  __typename\n}\n\n{}\n`;
  }
  return `### ${name}\nGET {{base}}/\nAccept: application/json\n`;
}

async function newRequest(deps: CommandDeps, input: unknown): Promise<void> {
  const fromTree = resolveTarget(deps.store, input);
  const workspace = fromTree?.workspace ?? (await pickWorkspace(deps.store));
  if (!workspace) return;
  if (workspace.collections.length === 0) {
    void vscode.window.showWarningMessage("Create a collection first (Mándalo: New Collection).");
    return;
  }
  const collection =
    fromTree?.collection ??
    (
      await vscode.window.showQuickPick(
        workspace.collections.map((item) => ({ label: item.name, description: item.slug, item })),
        { title: "Collection" },
      )
    )?.item;
  if (!collection) return;

  const kindPick = await vscode.window.showQuickPick(
    REQUEST_KINDS.map((kind) => ({
      label: kind === "http" ? "HTTP" : kind === "graphql" ? "GraphQL" : "gRPC",
      description: kind,
      value: kind,
    })),
    { title: "Request kind" },
  );
  if (!kindPick) return;

  const name = await vscode.window.showInputBox({
    title: "Request name",
    placeHolder: "Login",
    validateInput: (value) => (value.trim() === "" ? "Name must not be empty" : undefined),
  });
  if (name === undefined) return;

  const suggestedFolder =
    input && typeof input === "object" && "type" in (input as TreeNode) && (input as TreeNode).type === "folder"
      ? ((input as Extract<TreeNode, { type: "folder" }>).folder.relPath ?? "")
      : "";
  const folder = await vscode.window.showInputBox({
    title: "Folder inside the collection (blank for the root)",
    value: suggestedFolder,
    placeHolder: "auth/session",
    validateInput: (value) =>
      value.split("/").some((part) => part.startsWith(".") || part.includes("\\"))
        ? "Folder segments must not start with a dot or contain backslashes"
        : undefined,
  });
  if (folder === undefined) return;

  const dir = path.join(collection.dirPath, ...folder.split("/").filter((part) => part !== ""));
  const extension = kindPick.value === "grpc" ? "grpc" : "http";
  const target = vscode.Uri.file(path.join(dir, `${slugify(name)}.${extension}`));
  await vscode.workspace.fs.createDirectory(vscode.Uri.file(dir));
  try {
    await vscode.workspace.fs.stat(target);
    void vscode.window.showErrorMessage(`Mándalo: ${path.basename(target.fsPath)} already exists.`);
    return;
  } catch {
    // absent is the happy path
  }
  await vscode.workspace.fs.writeFile(
    target,
    Buffer.from(scaffold(kindPick.value, name.trim()), "utf8"),
  );
  await deps.store.refresh();
  await vscode.window.showTextDocument(target);
}

async function newCollection(deps: CommandDeps): Promise<void> {
  const workspace = await pickWorkspace(deps.store);
  if (!workspace) return;
  const name = await vscode.window.showInputBox({
    title: "Collection name",
    placeHolder: "Acme API",
    validateInput: (value) => (value.trim() === "" ? "Name must not be empty" : undefined),
  });
  if (name === undefined) return;
  const slug = slugify(name);
  const dir = vscode.Uri.file(path.join(workspace.rootPath, "collections", slug));
  const manifest = vscode.Uri.file(path.join(dir.fsPath, "collection.toml"));
  try {
    await vscode.workspace.fs.stat(manifest);
    void vscode.window.showErrorMessage(`Mándalo: collection "${slug}" already exists.`);
    return;
  } catch {
    // absent is the happy path
  }
  await vscode.workspace.fs.createDirectory(dir);
  const body = `schema_version = 1\nid = ${JSON.stringify(randomUUID())}\nname = ${JSON.stringify(name.trim())}\n`;
  await vscode.workspace.fs.writeFile(manifest, Buffer.from(body, "utf8"));
  await deps.store.refresh();
  await vscode.window.showTextDocument(manifest);
}

async function selectEnvironment(deps: CommandDeps): Promise<void> {
  const workspace = await pickWorkspace(deps.store);
  if (!workspace) return;
  const picked = await pickEnvironment(workspace, `Environment · ${workspace.name}`);
  if (!picked) return;
  await deps.store.setSelectedEnv(workspace, picked.name);
}

async function runRequestTests(deps: CommandDeps, input: unknown): Promise<void> {
  const target = await requireTarget(deps.store, input);
  if (!target) return;
  const item = deps.tests.itemFor(target.workspace, target.collection, target.relPath);
  if (!item) {
    void vscode.window.showWarningMessage("Mándalo: this request is not in the test tree yet — refresh.");
    return;
  }
  await deps.tests.runItems([item]);
  await vscode.commands.executeCommand("workbench.view.testing.focus");
}

async function openWorkspaceInApp(deps: CommandDeps, input: unknown): Promise<void> {
  const workspace = resolveTarget(deps.store, input)?.workspace ?? (await pickWorkspace(deps.store));
  if (!workspace) return;
  const uri = vscode.Uri.parse(`mandalo://open?workspace=${encodeURIComponent(workspace.rootPath)}`);
  const opened = await vscode.env.openExternal(uri).then(
    (value) => value,
    () => false,
  );
  if (!opened) deps.log.line(`could not hand ${uri.toString()} to the Mándalo desktop app`);
}

export function registerCommands(deps: CommandDeps): vscode.Disposable[] {
  const register = (id: string, handler: (...args: unknown[]) => unknown): vscode.Disposable =>
    vscode.commands.registerCommand(id, handler);

  return [
    register("mandalo.sendRequest", (input) => send(deps, input)),
    register("mandalo.sendRequestWithEnv", async (input) => {
      const target = await requireTarget(deps.store, input);
      if (!target) return;
      const picked = await pickEnvironment(target.workspace, "Send with environment");
      if (!picked) return;
      await send(deps, input, picked.name ?? null);
    }),
    register("mandalo.runRequestTests", (input) => runRequestTests(deps, input)),
    register("mandalo.runCollection", (input) => runCollection(deps, input)),
    register("mandalo.runFolder", (input) => {
      const node = input as TreeNode | undefined;
      const folder = node && node.type === "folder" ? node.folder.relPath : undefined;
      return runCollection(deps, input, folder);
    }),
    register("mandalo.selectEnvironment", () => selectEnvironment(deps)),
    register("mandalo.newRequest", (input) => newRequest(deps, input)),
    register("mandalo.newCollection", () => newCollection(deps)),
    register("mandalo.refresh", () => deps.store.refresh()),
    register("mandalo.openWorkspaceInApp", (input) => openWorkspaceInApp(deps, input)),
    register("mandalo.showLog", () => deps.log.show()),
  ];
}

export const __test = { slugify, scaffold };
