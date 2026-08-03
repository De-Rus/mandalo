import * as path from "node:path";
import { describe, expect, it } from "vitest";
import {
  defaultRequestRelPath,
  findCollectionFor,
  flattenRequests,
  requestRelPath,
  scanEnvironments,
  scanWorkspace,
} from "../../src/core/scan";

const FIXTURE = path.resolve(__dirname, "../../fixtures/workspace");

describe("scanWorkspace", () => {
  it("reads the workspace manifest", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    expect(workspace.name).toBe("Fixture");
    expect(workspace.rootPath).toBe(FIXTURE);
  });

  it("builds collections sorted by name", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    expect(workspace.collections.map((c) => c.slug)).toEqual(["acme-api", "broken"]);
    expect(workspace.collections[0]?.name).toBe("Acme API");
  });

  it("nests folders and keeps root-level requests apart", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const acme = workspace.collections[0]!;
    expect(acme.requests.map((r) => r.name)).toEqual(["Ping"]);
    expect(acme.folders.map((f) => f.name)).toEqual(["auth", "grpc", "users"]);
    expect(acme.folders[0]?.requests[0]).toMatchObject({
      name: "Login",
      method: "POST",
      kind: "http",
      relPath: "auth/login.http#0",
      index: 0,
      line: 2,
    });
    expect(acme.folders[2]?.requests[0]?.kind).toBe("graphql");
  });

  it("gives every block in a file its own sibling entry, in file order", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const auth = workspace.collections[0]!.folders[0]!;
    expect(auth.requests.map((r) => [r.name, r.relPath, r.method])).toEqual([
      ["Login", "auth/login.http#0", "POST"],
      ["Get profile", "auth/login.http#1", "GET"],
    ]);
  });

  it("reads .grpc files as gRPC requests", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const grpc = workspace.collections[0]!.folders[1]!;
    expect(grpc.requests.map((r) => [r.name, r.relPath, r.kind])).toEqual([
      ["Say hello", "grpc/mock.grpc#0", "grpc"],
      ["Say again", "grpc/mock.grpc#1", "grpc"],
    ]);
  });

  it("never treats a .toml file as a request", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const paths = flattenRequests(workspace.collections[0]!).map((r) => r.relPath);
    expect(paths.some((path) => path.endsWith(".toml"))).toBe(false);
    expect(paths.sort()).toEqual([
      "auth/login.http#0",
      "auth/login.http#1",
      "grpc/mock.grpc#0",
      "grpc/mock.grpc#1",
      "ping.http#0",
      "users/search.http#0",
    ]);
  });

  it("loads environments sorted by name", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    expect(workspace.environments.map((e) => e.name)).toEqual(["local", "prod"]);
    expect(workspace.environments[1]?.vars).toEqual({
      base: "https://api.example.com",
      token: "prod-token",
    });
  });

  it("keeps a collection whose only file is a legacy .toml request empty", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const broken = workspace.collections[1]!;
    expect(broken.requests).toEqual([]);
    expect(workspace.skipped).toEqual([]);
  });

  it("throws when the folder holds no mandalo.toml", async () => {
    await expect(scanWorkspace(path.join(FIXTURE, "collections"))).rejects.toThrow();
  });
});

describe("path helpers", () => {
  it("maps a file back to its collection", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const fsPath = path.join(FIXTURE, "collections", "acme-api", "auth", "login.http");
    const collection = findCollectionFor(workspace, fsPath);
    expect(collection?.slug).toBe("acme-api");
    expect(requestRelPath(collection!, fsPath)).toBe("auth/login.http");
  });

  it("addresses the first block when only a file is known", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    const collection = workspace.collections[0]!;
    expect(
      defaultRequestRelPath(collection, path.join(collection.dirPath, "auth", "login.http")),
    ).toBe("auth/login.http#0");
    expect(defaultRequestRelPath(collection, path.join(collection.dirPath, "collection.toml"))).toBe(
      "collection.toml",
    );
  });

  it("returns nothing for a file outside every collection", async () => {
    const workspace = await scanWorkspace(FIXTURE);
    expect(findCollectionFor(workspace, path.join(FIXTURE, "mandalo.toml"))).toBeUndefined();
  });

  it("scans environments standalone", async () => {
    expect((await scanEnvironments(FIXTURE)).map((e) => e.name)).toEqual(["local", "prod"]);
  });

  it("returns no environments for a folder without the directory", async () => {
    expect(await scanEnvironments(path.join(FIXTURE, "collections"))).toEqual([]);
  });
});
