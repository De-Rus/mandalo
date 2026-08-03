import * as vscode from "vscode";
import { parseVersion } from "./cliResolver";
import { MandaloCodeLensProvider } from "./codelens";
import { registerCommands } from "./commands";
import { addVariable, AddVariableActionProvider, MandaloDiagnostics } from "./diagnostics";
import type { SendResult } from "./core/cli";
import { defaultRequestRelPath, findCollectionFor } from "./core/scan";
import { EnvironmentStatus } from "./env";
import { Logger } from "./logger";
import { MandaloStore } from "./store";
import { MandaloTestController } from "./testing";
import { MandaloTreeProvider } from "./tree";

const TOML_SELECTOR: vscode.DocumentSelector = [
  { scheme: "file", pattern: "**/*.toml" },
  { scheme: "file", language: "toml" },
];

// Requests match by file pattern, not by language id: another extension may own the
// `.http` language, and our lenses have to show up either way.
const LENS_SELECTOR: vscode.DocumentSelector = [
  { scheme: "file", pattern: "**/*.toml" },
  { scheme: "file", language: "toml" },
  { scheme: "file", pattern: "**/*.{http,rest,grpc}" },
];

export interface MandaloApi {
  workspaces(): { name: string; rootPath: string; collections: string[] }[];
  testTree(): { id: string; label: string; children: string[] }[];
  send(fsPath: string, env?: string): Promise<SendResult>;
}

export async function activate(context: vscode.ExtensionContext): Promise<MandaloApi> {
  const log = new Logger();
  const store = new MandaloStore(context, log);
  const tree = new MandaloTreeProvider(store);
  const tests = new MandaloTestController(store, log);
  const diagnostics = new MandaloDiagnostics(store);
  const status = new EnvironmentStatus(store);

  const watcher = vscode.workspace.createFileSystemWatcher("**/*.{toml,http,rest,grpc}");
  const onFileChange = (): void => {
    void store.refresh();
  };
  watcher.onDidCreate(onFileChange);
  watcher.onDidDelete(onFileChange);
  watcher.onDidChange(onFileChange);

  context.subscriptions.push(
    log,
    store,
    tests,
    diagnostics,
    status,
    watcher,
    vscode.window.createTreeView("mandalo.requests", { treeDataProvider: tree, showCollapseAll: true }),
    vscode.languages.registerCodeLensProvider(LENS_SELECTOR, new MandaloCodeLensProvider(store)),
    vscode.languages.registerCodeActionsProvider(
      TOML_SELECTOR,
      new AddVariableActionProvider(store),
      AddVariableActionProvider.metadata,
    ),
    vscode.commands.registerCommand("mandalo.addVariable", (envPath: string, name: string) =>
      addVariable(envPath, name),
    ),
    vscode.workspace.onDidChangeWorkspaceFolders(onFileChange),
    ...registerCommands({ store, log, tests, extensionUri: context.extensionUri }),
  );

  store.resolver.ensureExecutable();
  void store.resolver.warnOnVersionSkew(async (binary) => {
    const raw = await store.cli.version(binary);
    return raw === null ? null : parseVersion(raw);
  });

  await store.refresh();

  return {
    workspaces: () =>
      store.all().map((workspace) => ({
        name: workspace.name,
        rootPath: workspace.rootPath,
        collections: workspace.collections.map((collection) => collection.slug),
      })),
    testTree: () => tests.snapshot(),
    send: async (fsPath, env) => {
      const workspace = store.workspaceFor(fsPath);
      if (!workspace) throw new Error(`${fsPath} is not inside a Mándalo workspace`);
      const collection = findCollectionFor(workspace, fsPath);
      if (!collection) throw new Error(`${fsPath} is not inside a collection`);
      return store.executor.send(
        workspace,
        collection,
        defaultRequestRelPath(collection, fsPath),
        env,
        store.envVars(workspace) ?? {},
      );
    },
  };
}

export function deactivate(): void {}
