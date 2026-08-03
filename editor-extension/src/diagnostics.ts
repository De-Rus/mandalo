import * as path from "node:path";
import * as vscode from "vscode";
import { lintRequestDocument } from "./core/rules";
import type { Finding } from "./core/rules";
import { findCollectionFor } from "./core/scan";
import type { MandaloStore } from "./store";

const UNRESOLVED_VAR = "mandalo.unresolvedVar";

function toDiagnostic(document: vscode.TextDocument, finding: Finding): vscode.Diagnostic {
  const range = new vscode.Range(
    document.positionAt(finding.offset),
    document.positionAt(finding.offset + finding.length),
  );
  const diagnostic = new vscode.Diagnostic(
    range,
    finding.message,
    finding.severity === "error"
      ? vscode.DiagnosticSeverity.Error
      : vscode.DiagnosticSeverity.Warning,
  );
  diagnostic.source = "mandalo";
  diagnostic.code = finding.code;
  return diagnostic;
}

export class MandaloDiagnostics implements vscode.Disposable {
  private readonly collection = vscode.languages.createDiagnosticCollection("mandalo");
  private readonly disposables: vscode.Disposable[] = [];

  constructor(private readonly store: MandaloStore) {
    this.disposables.push(
      vscode.workspace.onDidOpenTextDocument((document) => this.refresh(document)),
      vscode.workspace.onDidChangeTextDocument((event) => this.refresh(event.document)),
      vscode.workspace.onDidCloseTextDocument((document) => this.collection.delete(document.uri)),
      store.onDidChange(() => this.refreshAll()),
    );
    this.refreshAll();
  }

  refreshAll(): void {
    for (const document of vscode.workspace.textDocuments) this.refresh(document);
  }

  isRequestDocument(document: vscode.TextDocument): boolean {
    const fsPath = document.uri.fsPath;
    if (!fsPath.endsWith(".toml") || path.basename(fsPath) === "collection.toml") return false;
    if (path.basename(fsPath) === "mandalo.toml") return false;
    const workspace = this.store.workspaceFor(fsPath);
    if (!workspace) return false;
    return findCollectionFor(workspace, fsPath) !== undefined;
  }

  refresh(document: vscode.TextDocument): void {
    if (!vscode.workspace.getConfiguration("mandalo").get<boolean>("diagnostics.enabled", true)) {
      this.collection.delete(document.uri);
      return;
    }
    if (!this.isRequestDocument(document)) {
      this.collection.delete(document.uri);
      return;
    }
    const workspace = this.store.workspaceFor(document.uri.fsPath)!;
    const findings = lintRequestDocument(document.getText(), {
      envName: this.store.selectedEnv(workspace),
      envVars: this.store.envVars(workspace),
    });
    this.collection.set(
      document.uri,
      findings.map((finding) => toDiagnostic(document, finding)),
    );
  }

  dispose(): void {
    this.collection.dispose();
    for (const disposable of this.disposables) disposable.dispose();
  }
}

export class AddVariableActionProvider implements vscode.CodeActionProvider {
  static readonly metadata: vscode.CodeActionProviderMetadata = {
    providedCodeActionKinds: [vscode.CodeActionKind.QuickFix],
  };

  constructor(private readonly store: MandaloStore) {}

  provideCodeActions(
    document: vscode.TextDocument,
    _range: vscode.Range | vscode.Selection,
    context: vscode.CodeActionContext,
  ): vscode.CodeAction[] {
    const workspace = this.store.workspaceFor(document.uri.fsPath);
    if (!workspace) return [];
    const envName = this.store.selectedEnv(workspace);
    if (!envName) return [];
    const envPath = path.join(workspace.rootPath, "environments", `${envName}.toml`);
    const actions: vscode.CodeAction[] = [];
    for (const diagnostic of context.diagnostics) {
      if (diagnostic.code !== UNRESOLVED_VAR) continue;
      const name = document.getText(diagnostic.range).replace(/[{}]/g, "").trim();
      if (name === "") continue;
      const action = new vscode.CodeAction(
        `Add "${name}" to environment "${envName}"`,
        vscode.CodeActionKind.QuickFix,
      );
      action.diagnostics = [diagnostic];
      action.command = {
        command: "mandalo.addVariable",
        title: "Add variable",
        arguments: [envPath, name],
      };
      actions.push(action);
    }
    return actions;
  }
}

export async function addVariable(envPath: string, name: string): Promise<void> {
  const uri = vscode.Uri.file(envPath);
  const document = await vscode.workspace.openTextDocument(uri);
  const text = document.getText();
  const edit = new vscode.WorkspaceEdit();
  if (/^\s*\[vars\]\s*$/m.test(text)) {
    const match = /^\s*\[vars\]\s*$/m.exec(text)!;
    const insertAt = document.positionAt(match.index + match[0].length);
    edit.insert(uri, insertAt, `\n${name} = ""`);
  } else {
    const end = document.positionAt(text.length);
    edit.insert(uri, end, `${text.endsWith("\n") ? "" : "\n"}\n[vars]\n${name} = ""\n`);
  }
  await vscode.workspace.applyEdit(edit);
  await document.save();
  const editor = await vscode.window.showTextDocument(document);
  const inserted = editor.document.getText().indexOf(`${name} = ""`);
  if (inserted >= 0) {
    const position = editor.document.positionAt(inserted + name.length + 4);
    editor.selection = new vscode.Selection(position, position);
  }
}
