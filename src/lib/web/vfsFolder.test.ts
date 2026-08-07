import { beforeEach, describe, expect, it } from "vitest";
import { FolderVfs } from "./vfs";

class FakeWritable {
  private buffer = "";
  constructor(
    private readonly handle: FakeFileHandle,
    private readonly onWrite: (text: string) => void,
  ) {}

  async write(text: string): Promise<void> {
    this.onWrite(text);
    this.buffer += text;
  }

  async close(): Promise<void> {
    this.handle.text = this.buffer;
  }

  async abort(): Promise<void> {}
}

class FakeFileHandle {
  readonly kind = "file";
  text = "";
  move?: (parent: FakeDir, name: string) => Promise<void>;

  constructor(
    readonly name: string,
    private readonly dir: FakeDir,
    private readonly root: FakeRoot,
  ) {
    if (root?.canMove) this.move = (parent, to) => this.doMove(parent, to);
  }

  async getFile(): Promise<{ text: () => Promise<string> }> {
    return { text: async () => this.text };
  }

  async createWritable(): Promise<FakeWritable> {
    return new FakeWritable(this, (text) => this.root.onWrite(text));
  }

  private async doMove(parent: FakeDir, name: string): Promise<void> {
    this.dir.children.delete(this.name);
    const moved = new FakeFileHandle(name, parent, this.root);
    moved.text = this.text;
    parent.children.set(name, moved);
  }
}

class FakeDir {
  readonly kind = "directory";
  readonly children = new Map<string, FakeDir | FakeFileHandle>();

  constructor(
    readonly name: string,
    private readonly root: FakeRoot,
  ) {}

  async getDirectoryHandle(name: string, o?: { create?: boolean }): Promise<FakeDir> {
    const found = this.children.get(name);
    if (found instanceof FakeDir) return found;
    if (!o?.create) throw new Error("not found");
    const made = new FakeDir(name, this.root);
    this.children.set(name, made);
    return made;
  }

  async getFileHandle(name: string, o?: { create?: boolean }): Promise<FakeFileHandle> {
    const found = this.children.get(name);
    if (found instanceof FakeFileHandle) return found;
    if (!o?.create) throw new Error("not found");
    const made = new FakeFileHandle(name, this, this.root);
    this.children.set(name, made);
    return made;
  }

  async removeEntry(name: string): Promise<void> {
    this.children.delete(name);
  }

  async *entries(): AsyncGenerator<[string, FakeDir | FakeFileHandle]> {
    for (const entry of this.children.entries()) yield entry;
  }
}

class FakeRoot extends FakeDir {
  canMove = true;
  failWrite = false;

  constructor() {
    super("root", null as unknown as FakeRoot);
    Object.defineProperty(this, "root", { value: this });
  }

  onWrite(_: string): void {
    if (this.failWrite) throw new Error("the disk went away");
  }
}

function names(dir: FakeDir): string[] {
  return [...dir.children.keys()].sort();
}

let root: FakeRoot;
let vfs: FolderVfs;

beforeEach(() => {
  root = new FakeRoot();
  vfs = new FolderVfs("k", root as unknown as FileSystemDirectoryHandle);
});

describe("writing into a real folder", () => {
  it("lands the content and leaves no temp file behind", async () => {
    await vfs.write("collections/a/x.http", "hello");

    expect(await vfs.read("collections/a/x.http")).toBe("hello");
    const dir = await root.getDirectoryHandle("collections");
    expect(names(await dir.getDirectoryHandle("a"))).toEqual(["x.http"]);
  });

  it("works the same where the browser has no move()", async () => {
    root.canMove = false;

    await vfs.write("collections/a/x.http", "hello");

    expect(await vfs.read("collections/a/x.http")).toBe("hello");
    const dir = await (await root.getDirectoryHandle("collections")).getDirectoryHandle("a");
    expect(names(dir)).toEqual(["x.http"]);
  });

  it("leaves the previous version whole when the write fails", async () => {
    await vfs.write("collections/a/x.http", "first");
    root.failWrite = true;

    await expect(vfs.write("collections/a/x.http", "second")).rejects.toThrow(
      /disk went away/,
    );

    expect(await vfs.read("collections/a/x.http")).toBe("first");
  });

  it("cleans up the temp file when the write fails", async () => {
    await vfs.write("collections/a/x.http", "first");
    root.failWrite = true;
    await vfs.write("collections/a/x.http", "second").catch(() => undefined);

    const dir = await (await root.getDirectoryHandle("collections")).getDirectoryHandle("a");
    expect(names(dir)).toEqual(["x.http"]);
  });

  it("never shows a temp file as a request", async () => {
    const dir = await (
      await root.getDirectoryHandle("collections", { create: true })
    ).getDirectoryHandle("a", { create: true });
    await dir.getFileHandle(".x.http.mandalo-tmp", { create: true });
    await dir.getFileHandle("x.http", { create: true });

    expect((await vfs.list("collections/a")).map((e) => e.name)).toEqual(["x.http"]);
  });
});
