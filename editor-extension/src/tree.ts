import * as vscode from "vscode";
import type { CollectionNode, FolderNode, RequestNode, WorkspaceNode } from "./core/model";
import type { MandaloStore } from "./store";

export type TreeNode =
  | { type: "workspace"; workspace: WorkspaceNode }
  | { type: "collection"; workspace: WorkspaceNode; collection: CollectionNode }
  | { type: "folder"; workspace: WorkspaceNode; collection: CollectionNode; folder: FolderNode }
  | { type: "request"; workspace: WorkspaceNode; collection: CollectionNode; request: RequestNode }
  | { type: "message"; text: string };

const KIND_ICONS: Record<string, string> = {
  http: "globe",
  graphql: "type-hierarchy",
  grpc: "server-process",
};

const METHOD_COLORS: Record<string, string> = {
  GET: "charts.blue",
  POST: "charts.green",
  PUT: "charts.orange",
  PATCH: "charts.purple",
  DELETE: "charts.red",
};

export function requestIcon(request: RequestNode): vscode.ThemeIcon {
  const icon = KIND_ICONS[request.kind] ?? "question";
  const color = METHOD_COLORS[request.method.toUpperCase()];
  return new vscode.ThemeIcon(icon, color ? new vscode.ThemeColor(color) : undefined);
}

export function requestLabel(request: RequestNode): string {
  if (request.kind === "http") return request.name;
  return request.name;
}

export function requestDescription(request: RequestNode): string {
  if (request.kind === "graphql") return "GraphQL";
  if (request.kind === "grpc") return "gRPC";
  return request.method.toUpperCase();
}

export class MandaloTreeProvider implements vscode.TreeDataProvider<TreeNode> {
  private readonly changed = new vscode.EventEmitter<TreeNode | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  constructor(private readonly store: MandaloStore) {
    store.onDidChange(() => this.changed.fire(undefined));
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    switch (node.type) {
      case "message": {
        const item = new vscode.TreeItem(node.text, vscode.TreeItemCollapsibleState.None);
        item.contextValue = "mandalo.message";
        return item;
      }
      case "workspace": {
        const item = new vscode.TreeItem(node.workspace.name, vscode.TreeItemCollapsibleState.Expanded);
        item.description = this.store.selectedEnv(node.workspace) ?? "no environment";
        item.iconPath = new vscode.ThemeIcon("root-folder");
        item.contextValue = "mandalo.workspace";
        item.resourceUri = vscode.Uri.file(node.workspace.rootPath);
        return item;
      }
      case "collection": {
        const item = new vscode.TreeItem(node.collection.name, vscode.TreeItemCollapsibleState.Collapsed);
        item.description = node.collection.slug;
        item.iconPath = new vscode.ThemeIcon("library");
        item.contextValue = "mandalo.collection";
        item.resourceUri = vscode.Uri.file(node.collection.manifestPath);
        return item;
      }
      case "folder": {
        const item = new vscode.TreeItem(node.folder.name, vscode.TreeItemCollapsibleState.Collapsed);
        item.iconPath = new vscode.ThemeIcon("folder");
        item.contextValue = "mandalo.folder";
        return item;
      }
      case "request": {
        const item = new vscode.TreeItem(requestLabel(node.request), vscode.TreeItemCollapsibleState.None);
        item.description = requestDescription(node.request);
        item.iconPath = requestIcon(node.request);
        item.contextValue = "mandalo.request";
        item.resourceUri = vscode.Uri.file(node.request.fsPath);
        item.tooltip = `${node.request.method.toUpperCase()} ${node.request.relPath}`;
        const at = new vscode.Range(node.request.line, 0, node.request.line, 0);
        item.command = {
          command: "vscode.open",
          title: "Open",
          arguments: [vscode.Uri.file(node.request.fsPath), { selection: at }],
        };
        return item;
      }
    }
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (!node) {
      const workspaces = this.store.all();
      if (workspaces.length === 1) return this.collectionsOf(workspaces[0]!);
      return workspaces.map((workspace) => ({ type: "workspace", workspace }) as TreeNode);
    }
    switch (node.type) {
      case "workspace":
        return this.collectionsOf(node.workspace);
      case "collection":
        return [
          ...node.collection.folders.map(
            (folder) =>
              ({ type: "folder", workspace: node.workspace, collection: node.collection, folder }) as TreeNode,
          ),
          ...node.collection.requests.map(
            (request) =>
              ({ type: "request", workspace: node.workspace, collection: node.collection, request }) as TreeNode,
          ),
        ];
      case "folder":
        return [
          ...node.folder.folders.map(
            (folder) =>
              ({ type: "folder", workspace: node.workspace, collection: node.collection, folder }) as TreeNode,
          ),
          ...node.folder.requests.map(
            (request) =>
              ({ type: "request", workspace: node.workspace, collection: node.collection, request }) as TreeNode,
          ),
        ];
      default:
        return [];
    }
  }

  private collectionsOf(workspace: WorkspaceNode): TreeNode[] {
    return workspace.collections.map(
      (collection) => ({ type: "collection", workspace, collection }) as TreeNode,
    );
  }
}
