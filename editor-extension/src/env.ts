import * as vscode from "vscode";
import type { WorkspaceNode } from "./core/model";
import type { MandaloStore } from "./store";

export class EnvironmentStatus implements vscode.Disposable {
  private readonly item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);

  constructor(private readonly store: MandaloStore) {
    this.item.command = "mandalo.selectEnvironment";
    store.onDidChange(() => this.render());
    this.render();
  }

  render(): void {
    const workspaces = this.store.all();
    if (workspaces.length === 0) {
      this.item.hide();
      return;
    }
    const labels = workspaces.map((workspace) => this.store.selectedEnv(workspace) ?? "none");
    this.item.text = `$(globe) ${[...new Set(labels)].join(" · ")}`;
    this.item.tooltip = "Mándalo environment — click to change";
    this.item.show();
  }

  dispose(): void {
    this.item.dispose();
  }
}

export async function pickWorkspace(store: MandaloStore): Promise<WorkspaceNode | undefined> {
  const workspaces = store.all();
  if (workspaces.length === 0) {
    void vscode.window.showWarningMessage("No Mándalo workspace found — add a mandalo.toml first.");
    return undefined;
  }
  if (workspaces.length === 1) return workspaces[0];
  const picked = await vscode.window.showQuickPick(
    workspaces.map((workspace) => ({
      label: workspace.name,
      description: workspace.rootPath,
      workspace,
    })),
    { title: "Mándalo workspace" },
  );
  return picked?.workspace;
}

export async function pickEnvironment(
  workspace: WorkspaceNode,
  title = "Select environment",
): Promise<{ name: string | undefined } | undefined> {
  const items: (vscode.QuickPickItem & { value: string | undefined })[] = [
    { label: "No environment", description: "send raw, {{vars}} stay unresolved", value: undefined },
    ...workspace.environments.map((env) => ({
      label: env.name,
      description: `${Object.keys(env.vars).length} variables`,
      value: env.name as string | undefined,
    })),
  ];
  const picked = await vscode.window.showQuickPick(items, { title });
  if (!picked) return undefined;
  return { name: picked.value };
}
