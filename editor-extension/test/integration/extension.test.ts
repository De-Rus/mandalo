import * as assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as http from "node:http";
import * as path from "node:path";
import * as vscode from "vscode";
import type { MandaloApi } from "../../src/extension";

const EXTENSION_ID = "de-rus.mandalo";

async function workspaceRoot(): Promise<string> {
  const folder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(folder, "the fixture workspace must be open");
  return folder.uri.fsPath;
}

suite("Mándalo extension", () => {
  suiteSetup(async () => {
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    assert.ok(extension, `${EXTENSION_ID} must be installed`);
    await extension.activate();
    await vscode.commands.executeCommand("mandalo.refresh");
  });

  test("activates on a workspace holding mandalo.toml", () => {
    assert.equal(vscode.extensions.getExtension(EXTENSION_ID)?.isActive, true);
  });

  test("registers every contributed command", async () => {
    const commands = await vscode.commands.getCommands(true);
    for (const id of [
      "mandalo.sendRequest",
      "mandalo.sendRequestWithEnv",
      "mandalo.runRequestTests",
      "mandalo.runCollection",
      "mandalo.runFolder",
      "mandalo.selectEnvironment",
      "mandalo.newRequest",
      "mandalo.newCollection",
      "mandalo.refresh",
      "mandalo.openWorkspaceInApp",
      "mandalo.showLog",
      "mandalo.addVariable",
    ]) {
      assert.ok(commands.includes(id), `${id} is not registered`);
    }
  });

  test("claims only its own language ids, and no custom editor at all", () => {
    const manifest = vscode.extensions.getExtension(EXTENSION_ID)?.packageJSON as {
      contributes: { languages: { id: string }[]; customEditors?: unknown[] };
    };
    assert.deepEqual(
      manifest.contributes.languages.map((language) => language.id),
      ["mandalo-http", "mandalo-grpc"],
    );
    assert.equal(
      manifest.contributes.customEditors,
      undefined,
      "requests are plain text — nothing may take over an editor tab",
    );
  });

  test("offers Send / Send with env above every block of a .http file", async () => {
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "acme-api", "auth", "login.http"));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>(
      "vscode.executeCodeLensProvider",
      uri,
    );
    assert.deepEqual(
      (lenses ?? []).map((lens) => [lens.range.start.line, lens.command?.title]),
      [
        [2, "▶ Send"],
        [2, "▶ Send with env…"],
        [12, "▶ Send"],
        [12, "▶ Send with env…"],
      ],
    );
    assert.deepEqual(
      (lenses ?? []).map((lens) => (lens.command?.arguments?.[0] as { relPath: string }).relPath),
      ["auth/login.http#0", "auth/login.http#0", "auth/login.http#1", "auth/login.http#1"],
    );
  });

  test("offers the same lenses on a .grpc file", async () => {
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "acme-api", "grpc", "mock.grpc"));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>(
      "vscode.executeCodeLensProvider",
      uri,
    );
    assert.deepEqual(
      (lenses ?? []).map((lens) => (lens.command?.arguments?.[0] as { relPath: string }).relPath),
      ["grpc/mock.grpc#0", "grpc/mock.grpc#0", "grpc/mock.grpc#1", "grpc/mock.grpc#1"],
    );
  });

  test("offers a Run all CodeLens on collection.toml", async () => {
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "acme-api", "collection.toml"));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>(
      "vscode.executeCodeLensProvider",
      uri,
    );
    assert.deepEqual(
      (lenses ?? []).map((lens) => lens.command?.title),
      ["▶ Run all"],
    );
  });

  test("reports diagnostics for a malformed request", async () => {
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "broken", "bad-capture.toml"));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    await new Promise((resolve) => setTimeout(resolve, 500));
    const found = vscode.languages.getDiagnostics(uri).filter((d) => d.source === "mandalo");
    const codes = found.map((d) => String(d.code));
    assert.ok(codes.includes("mandalo.kind"), `expected a kind diagnostic, got ${codes.join(",")}`);
    assert.ok(codes.includes("mandalo.capture"));
    assert.ok(codes.includes("mandalo.test"));
  });

  test("keeps healthy requests diagnostic-free", async () => {
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "acme-api", "ping.toml"));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    await new Promise((resolve) => setTimeout(resolve, 500));
    assert.deepEqual(
      vscode.languages.getDiagnostics(uri).filter((d) => d.source === "mandalo"),
      [],
    );
  });

  test("degrades with a clear message when the CLI is absent", async () => {
    const config = vscode.workspace.getConfiguration("mandalo");
    const previous = config.get<string>("cliPath");
    await config.update("cliPath", "mandalo-does-not-exist", vscode.ConfigurationTarget.Workspace);
    const root = await workspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "collections", "acme-api", "ping.toml"));
    await assert.doesNotReject(() =>
      Promise.resolve(vscode.commands.executeCommand("mandalo.sendRequest", uri)),
    );
    await config.update("cliPath", previous, vscode.ConfigurationTarget.Workspace);
  });

  // The point of the in-process engine: a user who installed nothing but the extension can
  // still send. cliPath is aimed at a binary that does not exist, so any shell-out fails.
  test("sends a real HTTP request with no usable CLI anywhere", async () => {
    const config = vscode.workspace.getConfiguration("mandalo");
    const previousPath = config.get<string>("cliPath");
    const previousMode = config.get<string>("executionMode");
    await config.update("cliPath", "mandalo-does-not-exist", vscode.ConfigurationTarget.Workspace);
    await config.update("executionMode", "auto", vscode.ConfigurationTarget.Workspace);

    const server = http.createServer((_request, response) => {
      response.writeHead(200, { "content-type": "application/json", "x-proof": "in-process" });
      response.end('{"pong":true}');
    });
    await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
    const address = server.address();
    assert.ok(address !== null && typeof address !== "string");

    const root = await workspaceRoot();
    const dir = path.join(root, "collections", "acme-api");
    const fsPath = path.join(dir, "inproc.toml");
    await fs.writeFile(
      fsPath,
      `schema_version = 1
id = "inproc"
name = "In process"
kind = "http"
method = "GET"
url = "http://127.0.0.1:${address.port}/ping"

[[tests]]
kind = "status"
op = "eq"
value = 200

[[tests]]
kind = "json"
path = "$.pong"
op = "eq"
value = true

[[captures]]
from = "header.x-proof"
into = "proof"
scope = "run"
`,
    );

    try {
      await vscode.commands.executeCommand("mandalo.refresh");
      const api = await vscode.extensions.getExtension<MandaloApi>(EXTENSION_ID)?.activate();
      assert.ok(api, "the extension API must be available");
      const result = await api.send(fsPath);
      assert.equal(result.response?.status, 200);
      assert.deepEqual(
        result.tests.map((test) => test.passed),
        [true, true],
      );
      assert.equal(result.captures[0]?.value, "in-process");
      assert.equal(result.passed, true);
    } finally {
      await fs.rm(fsPath, { force: true });
      await new Promise<void>((done) => server.close(() => done()));
      await config.update("cliPath", previousPath, vscode.ConfigurationTarget.Workspace);
      await config.update("executionMode", previousMode, vscode.ConfigurationTarget.Workspace);
      await vscode.commands.executeCommand("mandalo.refresh");
    }
  });
});
