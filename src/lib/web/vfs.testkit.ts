import type { Entry, Vfs } from "./vfs";

/** An in-memory `Vfs` for tests: jsdom has no IndexedDB and no directory picker. */
export class MemoryVfs implements Vfs {
  readonly kind = "browser" as const;
  readonly id = "memory";

  private readonly files = new Map<string, string>();
  private readonly dirs = new Set<string>();

  private parent(path: string): string {
    return path.split("/").slice(0, -1).join("/");
  }

  async read(path: string): Promise<string | null> {
    return this.files.get(path) ?? null;
  }

  async write(path: string, text: string): Promise<void> {
    await this.mkdirp(this.parent(path));
    this.files.set(path, text);
  }

  async remove(path: string): Promise<void> {
    this.files.delete(path);
  }

  async removeDir(path: string): Promise<void> {
    for (const key of [...this.files.keys()])
      if (key === path || key.startsWith(`${path}/`)) this.files.delete(key);
    for (const key of [...this.dirs])
      if (key === path || key.startsWith(`${path}/`)) this.dirs.delete(key);
  }

  async mkdirp(path: string): Promise<void> {
    if (path === "") return;
    const parts = path.split("/");
    for (let i = 1; i <= parts.length; i += 1)
      this.dirs.add(parts.slice(0, i).join("/"));
  }

  async list(dir: string): Promise<Entry[]> {
    const out = new Map<string, Entry>();
    for (const path of this.files.keys())
      if (this.parent(path) === dir)
        out.set(path, { name: path.split("/").pop() as string, dir: false });
    for (const path of this.dirs)
      if (this.parent(path) === dir)
        out.set(path, { name: path.split("/").pop() as string, dir: true });
    return [...out.values()].sort((a, b) => a.name.localeCompare(b.name));
  }
}
