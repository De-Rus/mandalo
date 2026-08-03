import * as vscode from "vscode";
import type { RequestOutcome, RunResult } from "./core/cli";
import type { CollectionNode, FolderNode, WorkspaceNode } from "./core/model";
import { assertionChildren, assertionIndex, failureSummary } from "./core/report";
import type { Logger } from "./logger";
import type { MandaloStore } from "./store";

interface Target {
  workspace: WorkspaceNode;
  collection: CollectionNode;
  folder?: string;
  request?: string;
}

export class MandaloTestController implements vscode.Disposable {
  private readonly controller = vscode.tests.createTestController("mandalo", "Mándalo");
  private readonly targets = new Map<string, Target>();

  constructor(
    private readonly store: MandaloStore,
    private readonly log: Logger,
  ) {
    this.controller.refreshHandler = async () => {
      await this.store.refresh();
    };
    this.controller.createRunProfile(
      "Run",
      vscode.TestRunProfileKind.Run,
      (request, token) => void this.runHandler(request, token),
      true,
    );
    store.onDidChange(() => this.rebuild());
    this.rebuild();
  }

  private id(parts: string[]): string {
    return parts.join("::");
  }

  rebuild(): void {
    this.targets.clear();
    const items: vscode.TestItem[] = [];
    for (const workspace of this.store.all()) {
      for (const collection of workspace.collections) {
        const collectionId = this.id([workspace.rootPath, collection.slug]);
        const item = this.controller.createTestItem(
          collectionId,
          collection.name,
          vscode.Uri.file(collection.manifestPath),
        );
        this.targets.set(collectionId, { workspace, collection });
        this.fillFolder(item, workspace, collection, collection.folders, collection.requests);
        items.push(item);
      }
    }
    this.controller.items.replace(items);
  }

  private fillFolder(
    parent: vscode.TestItem,
    workspace: WorkspaceNode,
    collection: CollectionNode,
    folders: FolderNode[],
    requests: CollectionNode["requests"],
  ): void {
    for (const folder of folders) {
      const folderId = this.id([workspace.rootPath, collection.slug, folder.relPath]);
      const item = this.controller.createTestItem(folderId, folder.name);
      this.targets.set(folderId, { workspace, collection, folder: folder.relPath });
      this.fillFolder(item, workspace, collection, folder.folders, folder.requests);
      parent.children.add(item);
    }
    for (const request of requests) {
      const requestId = this.id([workspace.rootPath, collection.slug, request.relPath]);
      const item = this.controller.createTestItem(
        requestId,
        request.name,
        vscode.Uri.file(request.fsPath),
      );
      item.description = request.kind === "http" ? request.method.toUpperCase() : request.kind;
      this.targets.set(requestId, { workspace, collection, request: request.relPath });
      parent.children.add(item);
    }
  }

  private collectRequestItems(item: vscode.TestItem, into: vscode.TestItem[]): void {
    const target = this.targets.get(item.id);
    if (target?.request) {
      into.push(item);
      return;
    }
    if (!target && item.parent) {
      this.collectRequestItems(item.parent, into);
      return;
    }
    item.children.forEach((child) => this.collectRequestItems(child, into));
  }

  private queue(request: vscode.TestRunRequest): vscode.TestItem[] {
    const roots: vscode.TestItem[] = [];
    if (request.include) roots.push(...request.include);
    else this.controller.items.forEach((item) => roots.push(item));
    const excluded = new Set(request.exclude?.map((item) => item.id) ?? []);
    const out: vscode.TestItem[] = [];
    for (const root of roots) this.collectRequestItems(root, out);
    return out.filter((item) => !excluded.has(item.id));
  }

  private async runHandler(
    request: vscode.TestRunRequest,
    token: vscode.CancellationToken,
  ): Promise<void> {
    const run = this.controller.createTestRun(request);
    const items = this.queue(request);
    const byCollection = new Map<string, vscode.TestItem[]>();
    for (const item of items) {
      const target = this.targets.get(item.id);
      if (!target) continue;
      const key = this.id([target.workspace.rootPath, target.collection.slug]);
      byCollection.set(key, [...(byCollection.get(key) ?? []), item]);
      run.enqueued(item);
    }

    for (const [key, group] of byCollection) {
      if (token.isCancellationRequested) break;
      const target = this.targets.get(key);
      if (!target) continue;
      for (const item of group) run.started(item);
      const env = this.store.selectedEnv(target.workspace);
      try {
        const result = await this.store.executor.run(
          target.workspace,
          target.collection,
          { folder: this.sharedFolder(group), env },
          this.store.envVars(target.workspace) ?? {},
        );
        this.report(run, group, result);
      } catch (error) {
        const message = new vscode.TestMessage(
          error instanceof Error ? error.message : String(error),
        );
        for (const item of group) run.errored(item, message);
        this.log.report(error);
      }
    }
    run.end();
  }

  // `mandalo run` is per-collection; a --folder narrows it only when every selected
  // request lives under one folder, otherwise the whole collection runs and the
  // unselected results are simply discarded by report().
  private sharedFolder(group: vscode.TestItem[]): string | undefined {
    const paths = group
      .map((item) => this.targets.get(item.id)?.request)
      .filter((value): value is string => value !== undefined)
      .map((value) => value.split("/").slice(0, -1).join("/"));
    if (paths.length === 0) return undefined;
    const first = paths[0]!;
    if (first === "") return undefined;
    return paths.every((value) => value === first) ? first : undefined;
  }

  private report(run: vscode.TestRun, group: vscode.TestItem[], result: RunResult): void {
    const outcomes = new Map(result.requests.map((outcome) => [outcome.path, outcome]));
    for (const item of group) {
      const relPath = this.targets.get(item.id)?.request;
      const outcome = relPath ? outcomes.get(relPath) : undefined;
      if (!outcome) {
        run.skipped(item);
        continue;
      }
      this.syncAssertions(item, outcome);
      const duration = outcome.response?.durationMs;
      if (outcome.error) {
        run.failed(item, new vscode.TestMessage(outcome.error), duration);
        item.children.forEach((child) => run.skipped(child));
        continue;
      }
      const byId = assertionIndex(item.id, outcome.tests);
      let failed = false;
      item.children.forEach((child) => {
        const test = byId.get(child.id);
        if (!test) {
          run.skipped(child);
          return;
        }
        if (test.passed) {
          run.passed(child, duration);
          return;
        }
        failed = true;
        run.failed(child, new vscode.TestMessage(test.detail ?? `${test.name} failed`), duration);
      });
      if (failed) {
        run.failed(item, new vscode.TestMessage(failureSummary(outcome.tests)), duration);
      } else {
        run.passed(item, duration);
      }
    }
  }

  private syncAssertions(item: vscode.TestItem, outcome: RequestOutcome): void {
    item.children.replace(
      assertionChildren(item.id, outcome.tests).map((child) =>
        this.controller.createTestItem(child.id, child.label, item.uri),
      ),
    );
  }

  itemFor(workspace: WorkspaceNode, collection: CollectionNode, relPath?: string): vscode.TestItem | undefined {
    const key = this.id(relPath ? [workspace.rootPath, collection.slug, relPath] : [workspace.rootPath, collection.slug]);
    let found: vscode.TestItem | undefined;
    const walk = (collectionItems: vscode.TestItemCollection): void => {
      collectionItems.forEach((item) => {
        if (found) return;
        if (item.id === key) {
          found = item;
          return;
        }
        walk(item.children);
      });
    };
    walk(this.controller.items);
    return found;
  }

  snapshot(): { id: string; label: string; children: string[] }[] {
    const out: { id: string; label: string; children: string[] }[] = [];
    const walk = (items: vscode.TestItemCollection): void => {
      items.forEach((item) => {
        const children: string[] = [];
        item.children.forEach((child) => children.push(child.id));
        out.push({ id: item.id, label: item.label, children });
        walk(item.children);
      });
    };
    walk(this.controller.items);
    return out;
  }

  async runItems(items: vscode.TestItem[]): Promise<void> {
    const request = new vscode.TestRunRequest(items);
    await this.runHandler(request, new vscode.CancellationTokenSource().token);
  }

  dispose(): void {
    this.controller.dispose();
  }
}
