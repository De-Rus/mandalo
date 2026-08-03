import * as path from "node:path";
import * as vscode from "vscode";
import { requestLenses } from "./core/lenses";
import { findCollectionFor, requestRelPath } from "./core/scan";
import { textFileKind } from "./core/textFormat";
import type { MandaloStore } from "./store";

export class MandaloCodeLensProvider implements vscode.CodeLensProvider {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.changed.event;

  constructor(private readonly store: MandaloStore) {
    store.onDidChange(() => this.changed.fire());
  }

  provideCodeLenses(document: vscode.TextDocument): vscode.CodeLens[] {
    if (!vscode.workspace.getConfiguration("mandalo").get<boolean>("codeLens.enabled", true)) return [];
    const fsPath = document.uri.fsPath;
    const workspace = this.store.workspaceFor(fsPath);
    if (!workspace) return [];
    const collection = findCollectionFor(workspace, fsPath);
    if (!collection) return [];

    const fileKind = textFileKind(fsPath);
    if (fileKind !== undefined) {
      const relPath = requestRelPath(collection, fsPath);
      return requestLenses(document.getText(), fileKind, relPath).map(
        (lens) =>
          new vscode.CodeLens(new vscode.Range(lens.line, 0, lens.line, 0), {
            title: lens.title,
            command: lens.command,
            arguments: [{ uri: document.uri, relPath: lens.relPath }],
          }),
      );
    }

    if (path.basename(fsPath) === "collection.toml") {
      return [
        new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
          title: "▶ Run all",
          command: "mandalo.runCollection",
          arguments: [document.uri],
        }),
      ];
    }

    return [];
  }
}
