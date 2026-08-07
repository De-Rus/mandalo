import { describe, expect, it } from "vitest";
import { collect } from "./export";
import { MemoryVfs } from "./vfs.testkit";
import { crc32, zip } from "./zip";

async function workspace(): Promise<MemoryVfs> {
  const vfs = new MemoryVfs();
  await vfs.write("mandalo.toml", 'schema_version = 1\nname = "Acme"\n');
  await vfs.write("environments/prod.toml", 'name = "prod"\n');
  await vfs.write("collections/acme/http/get.http", "GET https://api.dev\n");
  return vfs;
}

function readU32(bytes: Uint8Array, at: number): number {
  return new DataView(bytes.buffer).getUint32(at, true);
}

describe("taking a copy out of the browser", () => {
  it("collects every file in the workspace, folders and all", async () => {
    const entries = await collect(await workspace());

    expect(entries.map((e) => e.path).sort()).toEqual([
      "collections/acme/http/get.http",
      "environments/prod.toml",
      "mandalo.toml",
    ]);
  });

  it("keeps the real .http text, not a re-encoded copy", async () => {
    const entries = await collect(await workspace());

    expect(entries.find((e) => e.path.endsWith("get.http"))?.text).toBe(
      "GET https://api.dev\n",
    );
  });

  it("produces a zip a real unzipper will accept", async () => {
    const blob = zip([{ path: "a.http", text: "GET https://api.dev\n" }]);
    const bytes = new Uint8Array(await blob.arrayBuffer());

    expect(readU32(bytes, 0)).toBe(0x04034b50);
    expect(blob.type).toBe("application/zip");
  });

  it("checksums each entry so a corrupt copy is detectable", () => {
    expect(crc32(new TextEncoder().encode("hello"))).toBe(0x3610a686);
  });

  it("refuses to pretend an empty workspace was exported", async () => {
    const { exportWorkspace } = await import("./export");

    await expect(exportWorkspace(new MemoryVfs())).rejects.toThrow(/nothing/);
  });
});
