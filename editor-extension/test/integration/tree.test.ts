import * as assert from "node:assert/strict";
import * as vscode from "vscode";
import type { MandaloApi } from "../../src/extension";

async function api(): Promise<MandaloApi> {
  const extension = vscode.extensions.getExtension<MandaloApi>("de-rus.mandalo");
  assert.ok(extension, "the extension must be installed");
  const exported = await extension.activate();
  await vscode.commands.executeCommand("mandalo.refresh");
  return exported;
}

suite("Mándalo workspace discovery", () => {
  test("finds the fixture workspace and both collections", async () => {
    const workspaces = (await api()).workspaces();
    assert.equal(workspaces.length, 1);
    assert.equal(workspaces[0]?.name, "Fixture");
    assert.deepEqual(workspaces[0]?.collections, ["acme-api", "broken"]);
  });

  test("mirrors collections and folders into the Testing panel", async () => {
    const tree = (await api()).testTree();
    const labels = tree.map((item) => item.label);
    assert.ok(labels.includes("Acme API"), `expected Acme API suite, got ${labels.join(", ")}`);
    assert.ok(labels.includes("auth"));
    assert.ok(labels.includes("Login"));
    assert.ok(labels.includes("Ping"));

    const acme = tree.find((item) => item.label === "Acme API");
    assert.ok(acme);
    assert.equal(acme.children.length, 4, "three folders plus the root-level Ping request");

    const auth = tree.find((item) => item.label === "auth");
    assert.ok(auth);
    assert.deepEqual(
      auth.children.map((id) => id.slice(id.lastIndexOf("::") + 2)),
      ["auth/login.http#0", "auth/login.http#1"],
      "each ### block of a .http file is its own test item",
    );
  });

  test("registers gRPC blocks alongside HTTP ones", async () => {
    const tree = (await api()).testTree();
    const grpc = tree.find((item) => item.label === "grpc");
    assert.ok(grpc, "the grpc folder must be in the test tree");
    assert.deepEqual(
      grpc.children.map((id) => id.slice(id.lastIndexOf("::") + 2)),
      ["grpc/mock.grpc#0", "grpc/mock.grpc#1"],
    );
  });

  test("never registers a .toml file as a test item", async () => {
    const tree = (await api()).testTree();
    assert.equal(
      tree.filter((item) => item.id.endsWith(".toml")).length,
      0,
    );
  });
});
