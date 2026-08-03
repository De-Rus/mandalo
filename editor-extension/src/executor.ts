import { readFile } from "node:fs/promises";
import * as path from "node:path";
import type { MandaloCli, RunResult, SendResult } from "./core/cli";
import type { CollectionNode, FolderNode, RequestNode, WorkspaceNode } from "./core/model";
import { parseRequest } from "./core/parse";
import { isTextRequestPath, requestFilePath } from "./core/textFormat";
import { EscalateError, failed, runMany, runOne, type EngineRequest } from "./engine/run";

const TEXT_FORMAT_REASON =
  "the .http/.rest/.grpc text format is parsed by the Mándalo CLI, not by the in-process engine";

export type ExecutionMode = "auto" | "cli" | "in-process";
export type Engine = "in-process" | "cli";

export interface ExecutorDeps {
  cli: MandaloCli;
  hasCli: () => boolean;
  mode: () => ExecutionMode;
  log: (line: string) => void;
}

export interface RunOptions {
  folder?: string | undefined;
  env?: string | undefined;
}

function collect(folders: readonly FolderNode[], into: RequestNode[]): void {
  for (const folder of folders) {
    into.push(...folder.requests);
    collect(folder.folders, into);
  }
}

export function suiteRequests(collection: CollectionNode, folder?: string): RequestNode[] {
  const all: RequestNode[] = [...collection.requests];
  collect(collection.folders, all);
  const sorted = all.sort((a, b) => {
    const left = requestFilePath(a.relPath);
    const right = requestFilePath(b.relPath);
    return left === right ? a.index - b.index : left.localeCompare(right);
  });
  if (folder === undefined || folder === "") return sorted;
  const prefix = `${folder}/`;
  return sorted.filter((request) => request.relPath.startsWith(prefix));
}

async function load(request: { relPath: string; fsPath: string }): Promise<EngineRequest> {
  const raw = await readFile(request.fsPath, "utf8");
  return { model: parseRequest(raw), relPath: request.relPath };
}

function isTextFormat(relPath: string): boolean {
  return isTextRequestPath(requestFilePath(relPath));
}

export class MandaloExecutor {
  constructor(private readonly deps: ExecutorDeps) {}

  /** `auto` keeps HTTP and GraphQL in the editor process; everything else needs the CLI. */
  private engineFor(kinds: readonly string[]): Engine {
    const mode = this.deps.mode();
    if (mode === "cli") return "cli";
    if (mode === "in-process") return "in-process";
    return kinds.every((kind) => kind === "http" || kind === "graphql") ? "in-process" : "cli";
  }

  private escalate(where: string, reason: string): void {
    if (!this.deps.hasCli()) {
      throw new Error(
        `${reason}. The Mándalo CLI would handle it, but no binary is available — install a platform build of the extension or set "mandalo.cliPath".`,
      );
    }
    this.deps.log(`${where}: falling back to the CLI — ${reason}`);
  }

  private requireCli(where: string, reason: string): void {
    this.escalate(where, reason);
    this.deps.log(`engine: cli · ${where}`);
  }

  async send(
    workspace: WorkspaceNode,
    collection: CollectionNode,
    relPath: string,
    env: string | undefined,
    vars: Record<string, string>,
  ): Promise<SendResult> {
    if (isTextFormat(relPath)) {
      this.requireCli(relPath, TEXT_FORMAT_REASON);
      return this.deps.cli.send(workspace.rootPath, collection.slug, relPath, env);
    }

    const fsPath = path.join(collection.dirPath, requestFilePath(relPath));
    const request = await load({ relPath, fsPath });

    if (this.engineFor([request.model.kind]) === "in-process") {
      this.deps.log(`engine: in-process · ${relPath}`);
      try {
        return await runOne(request, collection.slug, env ?? null, { ...vars }, this.deps);
      } catch (error) {
        if (!(error instanceof EscalateError)) {
          return { ...failed(request, error), collection: collection.slug, env: env ?? null };
        }
        this.escalate(relPath, error.reason);
      }
    } else {
      this.deps.log(`engine: cli · ${relPath}`);
    }
    return this.deps.cli.send(workspace.rootPath, collection.slug, relPath, env);
  }

  async run(
    workspace: WorkspaceNode,
    collection: CollectionNode,
    options: RunOptions,
    vars: Record<string, string>,
  ): Promise<RunResult> {
    const nodes = suiteRequests(collection, options.folder);
    const label = `${collection.slug}${options.folder ? `/${options.folder}` : ""}`;

    if (nodes.some((node) => isTextFormat(node.relPath))) {
      this.requireCli(label, TEXT_FORMAT_REASON);
      return this.deps.cli.run(workspace.rootPath, collection.slug, options);
    }

    const requests = await Promise.all(nodes.map(load));

    if (this.engineFor(requests.map((request) => request.model.kind)) === "in-process") {
      this.deps.log(`engine: in-process · ${label} (${requests.length} requests)`);
      try {
        return await runMany(requests, collection.slug, options.env ?? null, { ...vars }, this.deps);
      } catch (error) {
        if (!(error instanceof EscalateError)) throw error;
        this.escalate(label, error.reason);
      }
    } else {
      this.deps.log(`engine: cli · ${label}`);
    }
    return this.deps.cli.run(workspace.rootPath, collection.slug, options);
  }
}
