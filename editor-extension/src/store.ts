import * as path from "node:path";
import * as vscode from "vscode";
import { CliResolver } from "./cliResolver";
import { MandaloCli } from "./core/cli";
import type { WorkspaceNode } from "./core/model";
import { scanWorkspace } from "./core/scan";
import { MandaloExecutor, type ExecutionMode } from "./executor";
import type { Logger } from "./logger";

const ENV_STATE_PREFIX = "mandalo.env:";

export class MandaloStore implements vscode.Disposable {
  private workspaces: WorkspaceNode[] = [];
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChange = this.changed.event;
  readonly cli: MandaloCli;
  readonly resolver: CliResolver;
  readonly executor: MandaloExecutor;
  private pending: Promise<void> | undefined;

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly log: Logger,
  ) {
    this.resolver = new CliResolver(
      context.extensionPath,
      String(context.extension.packageJSON.version ?? "0.0.0"),
      (line) => this.log.line(line),
    );
    this.cli = new MandaloCli({
      cliPath: () => this.resolver.binary(),
      timeoutMs: () => vscode.workspace.getConfiguration("mandalo").get<number>("timeoutMs", 120_000),
      cwd: () => vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
      log: (line) => this.log.line(line),
    });
    this.executor = new MandaloExecutor({
      cli: this.cli,
      hasCli: () => this.resolver.find() !== null,
      mode: () =>
        vscode.workspace.getConfiguration("mandalo").get<ExecutionMode>("executionMode", "auto"),
      log: (line) => this.log.line(line),
    });
  }

  all(): WorkspaceNode[] {
    return this.workspaces;
  }

  async refresh(): Promise<void> {
    if (this.pending) return this.pending;
    this.pending = this.doRefresh().finally(() => {
      this.pending = undefined;
    });
    return this.pending;
  }

  private async doRefresh(): Promise<void> {
    const manifests = await vscode.workspace.findFiles("**/mandalo.toml", "**/node_modules/**", 32);
    const roots = [...new Set(manifests.map((uri) => path.dirname(uri.fsPath)))].sort();
    const found: WorkspaceNode[] = [];
    for (const root of roots) {
      try {
        const workspace = await scanWorkspace(root);
        for (const problem of workspace.skipped) this.log.line(`skipped ${problem}`);
        found.push(workspace);
      } catch (error) {
        this.log.line(`${root}: ${(error as Error).message}`);
      }
    }
    this.workspaces = found;
    await vscode.commands.executeCommand("setContext", "mandalo.hasWorkspace", found.length > 0);
    this.changed.fire();
  }

  workspaceFor(fsPath: string): WorkspaceNode | undefined {
    return this.workspaces
      .filter((workspace) => fsPath === workspace.rootPath || fsPath.startsWith(workspace.rootPath + path.sep))
      .sort((a, b) => b.rootPath.length - a.rootPath.length)[0];
  }

  selectedEnv(workspace: WorkspaceNode): string | undefined {
    const stored = this.context.workspaceState.get<string>(ENV_STATE_PREFIX + workspace.rootPath);
    if (stored && workspace.environments.some((env) => env.name === stored)) return stored;
    return undefined;
  }

  async setSelectedEnv(workspace: WorkspaceNode, name: string | undefined): Promise<void> {
    await this.context.workspaceState.update(ENV_STATE_PREFIX + workspace.rootPath, name);
    this.changed.fire();
  }

  envVars(workspace: WorkspaceNode): Record<string, string> | undefined {
    const name = this.selectedEnv(workspace);
    if (!name) return undefined;
    return workspace.environments.find((env) => env.name === name)?.vars;
  }

  dispose(): void {
    this.changed.dispose();
  }
}
