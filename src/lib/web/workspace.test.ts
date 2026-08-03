import { describe, expect, it } from "vitest";
import {
  bindSecretHost,
  deleteVar,
  listEnvironments,
  saveEnvironment,
  secretStatus,
} from "./workspace";
import type { Entry, Vfs } from "./vfs";

class MemoryVfs implements Vfs {
  readonly id = "mem";
  readonly kind = "browser" as const;
  private readonly files = new Map<string, string>();

  read(path: string): Promise<string | null> {
    return Promise.resolve(this.files.get(path) ?? null);
  }

  write(path: string, text: string): Promise<void> {
    this.files.set(path, text);
    return Promise.resolve();
  }

  remove(path: string): Promise<void> {
    this.files.delete(path);
    return Promise.resolve();
  }

  removeDir(dir: string): Promise<void> {
    for (const path of [...this.files.keys()])
      if (path.startsWith(`${dir}/`)) this.files.delete(path);
    return Promise.resolve();
  }

  mkdirp(): Promise<void> {
    return Promise.resolve();
  }

  list(dir: string): Promise<Entry[]> {
    const names = new Set<string>();
    for (const path of this.files.keys()) {
      if (!path.startsWith(`${dir}/`)) continue;
      names.add(path.slice(dir.length + 1).split("/")[0]);
    }
    return Promise.resolve(
      [...names].map((name) => ({ name, dir: !name.includes(".") })),
    );
  }
}

const PROD =
  'schema_version = 1\nname = "prod"\n\n' +
  '[vars.base]\nvalue = "https://api.acme.com"\n\n' +
  '[vars.token]\nsecret = true\nhosts = ["api.acme.com"]\n';

async function withProd(): Promise<MemoryVfs> {
  const vfs = new MemoryVfs();
  await vfs.write("environments/prod.toml", PROD);
  return vfs;
}

describe("web environments", () => {
  it("lists declarations, never a secret value", async () => {
    const { items } = await listEnvironments(await withProd());

    expect(items).toEqual([
      {
        name: "prod",
        vars: {
          base: {
            secret: false,
            value: "https://api.acme.com",
            set: true,
          },
          token: {
            secret: true,
            value: null,
            hosts: ["api.acme.com"],
            set: false,
          },
        },
      },
    ]);
  });

  it("keeps secret declarations when plain variables are saved", async () => {
    const vfs = await withProd();

    await saveEnvironment(vfs, { name: "prod", vars: { base: "https://x" } });

    const raw = await vfs.read("environments/prod.toml");
    expect(raw).toContain("[vars.token]");
    expect(raw).toContain("secret = true");
    expect(raw).toContain('value = "https://x"');
  });

  it("refuses to give a declared secret a value in the file", async () => {
    const vfs = await withProd();

    await expect(
      saveEnvironment(vfs, { name: "prod", vars: { token: "leaked" } }),
    ).rejects.toThrow(/declared secret/);
    expect(await vfs.read("environments/prod.toml")).not.toContain("leaked");
  });

  it("binds a host to a secret and lowercases it", async () => {
    const vfs = await withProd();

    const hosts = await bindSecretHost(vfs, "prod", "token", "Edge.Acme.COM");

    expect(hosts).toEqual(["api.acme.com", "edge.acme.com"]);
    expect(await vfs.read("environments/prod.toml")).toContain("edge.acme.com");
  });

  it("refuses to bind a host to a plain variable", async () => {
    await expect(
      bindSecretHost(await withProd(), "prod", "base", "x.dev"),
    ).rejects.toThrow(/not declared secret/);
  });

  it("deletes a variable and reports secret status as unset in the browser", async () => {
    const vfs = await withProd();

    expect(await secretStatus(vfs, "prod")).toEqual({ token: false });
    await deleteVar(vfs, "prod", "token");
    expect(await secretStatus(vfs, "prod")).toEqual({});
  });
});
