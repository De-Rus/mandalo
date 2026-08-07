import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { addSampleCollection } from "./mounts";
import { seedFiles } from "./seed";
import { MemoryVfs } from "./vfs";

/**
 * The one list the browser glob and `crates/core/build.rs` both have to agree on.
 * If it drifts, the two hosts write different trees for the same action, which is
 * exactly the bug this file exists to catch.
 */
const EXTENSIONS = ["toml", "http", "rest", "grpc", "ws", "mqtt", "proto", "json", "txt"];

const ROOT = resolve(__dirname, "../../../examples/mock-workspace");

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(path));
    else if (EXTENSIONS.includes(path.split(".").pop() ?? "")) out.push(path);
  }
  return out;
}

describe("the sample collection is one fixture, not two copies", () => {
  it("inlines exactly the files on disk, byte for byte", () => {
    const onDisk = walk(ROOT)
      .map((path) => relative(ROOT, path).split("\\").join("/"))
      .sort();

    const inlined = seedFiles();

    expect(inlined.map(([path]) => path)).toEqual(onDisk);
    for (const [path, text] of inlined)
      expect(text).toBe(readFileSync(join(ROOT, path), "utf8"));
  });

  it("carries no binary fixture the desktop table would not have", () => {
    expect(seedFiles().every(([path]) => !path.endsWith(".pdf"))).toBe(true);
  });

  it("writes the tree the desktop writes, path for path", async () => {
    const vfs = new MemoryVfs();

    const slug = await addSampleCollection(vfs);

    const expected = seedFiles()
      .filter(([path]) => path.startsWith("collections/mock/"))
      .map(([path]) => `collections/${slug}/${path.slice("collections/mock/".length)}`)
      .sort();
    const written: string[] = [];
    const walkVfs = async (dir: string) => {
      for (const entry of await vfs.list(dir)) {
        const path = dir === "" ? entry.name : `${dir}/${entry.name}`;
        if (entry.dir) await walkVfs(path);
        else written.push(path);
      }
    };
    await walkVfs(`collections/${slug}`);

    expect(written.sort()).toEqual(expected);
  });

  it("brings the supporting files the sample's requests point at", async () => {
    const vfs = new MemoryVfs();

    await addSampleCollection(vfs);

    for (const [path] of seedFiles()) {
      if (path.startsWith("collections/") || path === "mandalo.toml") continue;
      expect(await vfs.read(path)).not.toBeNull();
    }
  });
});
