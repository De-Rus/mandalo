import { readFile } from "node:fs/promises";
import * as path from "node:path";
import type { MandaloCli, RunResult, SendResult } from "./core/cli";
import type { CollectionNode, FolderNode, RequestNode, WorkspaceNode } from "./core/model";
import { parseTextDocument, withFileVars } from "../../src/lib/format/httpFormat";
import {
  indexIn,
  requestFilePath,
  requestFragmentOf,
  textFileKind,
} from "../../src/lib/format/textFormat";
import { EscalateError, failed, runMany, runOne, type EngineRequest } from "./engine/run";

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
  const file = requestFilePath(request.relPath);
  const fileKind = textFileKind(file);
  if (fileKind === undefined) {
    throw new Error(
      `${file} is not a request file — Mándalo stores HTTP and GraphQL in .http and gRPC in .grpc`,
    );
  }
  const raw = await readFile(request.fsPath, "utf8");
  const blocks = withFileVars(parseTextDocument(file, raw, fileKind));
  const index = indexIn(
    blocks.map((block) => block.name),
    file,
    requestFragmentOf(request.relPath),
  );
  return { model: blocks[index]!.model, relPath: request.relPath };
}

export class MandaloExecutor {
  constructor(private readonly deps: ExecutorDeps) {}

  /**
   * `auto` keeps HTTP and GraphQL in the editor process; everything else needs the
   * CLI. The verdict is per request: one `.grpc` file in a collection must not move
   * the unrelated requests around it onto a different engine, because the two put
   * different bytes on the wire.
   */
  private engineFor(request: EngineRequest): Engine {
    const mode = this.deps.mode();
    if (mode === "cli") return "cli";
    if (mode === "in-process") return "in-process";
    const { model } = request;
    if (model.kind !== "http" && model.kind !== "graphql") return "cli";
    return model.bodyFile === undefined && model.formdata === undefined ? "in-process" : "cli";
  }

  /**
   * A suite shares one variable scope, so a capture in request 3 has to reach request
   * 7; splitting it across two engines would break that chain. One request that needs
   * the CLI therefore still moves the whole suite — which is only safe because both
   * engines now put the same bytes on the wire for the requests they share.
   */
  private engineForSuite(requests: readonly EngineRequest[]): Engine {
    return requests.every((request) => this.engineFor(request) === "in-process")
      ? "in-process"
      : "cli";
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
    const fsPath = path.join(collection.dirPath, requestFilePath(relPath));
    let request: EngineRequest;
    try {
      request = await load({ relPath, fsPath });
    } catch (error) {
      if (!(error instanceof EscalateError)) throw error;
      this.requireCli(relPath, error.reason);
      return this.deps.cli.send(workspace.rootPath, collection.slug, relPath, env);
    }

    if (this.engineFor(request) === "in-process") {
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

    let requests: EngineRequest[];
    try {
      requests = await Promise.all(nodes.map(load));
    } catch (error) {
      if (!(error instanceof EscalateError)) throw error;
      this.requireCli(label, error.reason);
      return this.deps.cli.run(workspace.rootPath, collection.slug, options);
    }

    if (this.engineForSuite(requests) === "in-process") {
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
