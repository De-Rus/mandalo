import { beforeEach, describe, expect, it, vi } from "vitest";

const store = new Map<string, Map<string, unknown>>();

function bucket(name: string): Map<string, unknown> {
  const found = store.get(name) ?? new Map();
  store.set(name, found);
  return found;
}

vi.mock("./idb", () => ({
  FILES: "files",
  HANDLES: "handles",
  META: "meta",
  VERSIONS: "versions",
  get: async (name: string, key: string) => bucket(name).get(key),
  put: async (name: string, key: string, value: unknown) => {
    bucket(name).set(key, value);
  },
  del: async (name: string, key: string) => {
    bucket(name).delete(key);
  },
  entries: async (name: string) => [...bucket(name).entries()],
  writeAtomically: async (options: {
    fileKey: string;
    text: string;
    dirKeys: string[];
    dirMarker: string;
  }) => {
    for (const dir of options.dirKeys) bucket("files").set(dir, options.dirMarker);
    bucket("files").set(options.fileKey, options.text);
  },
}));

import type { RemoteFetch } from "./remote";
import * as mounts from "./mounts";

const ORIGIN = {
  label: "github.com/acme/apis",
  url: "https://github.com/acme/apis",
  commit: "0f1e2d3c",
  fetchedAt: 1,
};

const FETCHED: RemoteFetch = {
  origin: ORIGIN,
  files: [
    ["mandalo.toml", 'schema_version = 1\nid = "shared"\nname = "Shared"\n'],
    [
      "collections/billing/collection.toml",
      'schema_version = 1\nid = "billing"\nname = "Billing"\n',
    ],
    [
      "collections/billing/invoices.http",
      "### GET invoices\nGET https://api.billing.example/invoices\n",
    ],
  ],
  skipped: [],
  bytes: 200,
};

beforeEach(() => {
  store.clear();
});

describe("a workspace opened from a link", () => {
  it("lands in a workspace of its own, stamped with where it came from", async () => {
    const info = await mounts.openRemote(FETCHED);

    expect(info.name).toBe("github.com/acme/apis");
    expect(await mounts.originOf(info.path)).toMatchObject({
      label: "github.com/acme/apis",
      commit: "0f1e2d3c",
    });
    const listed = await mounts.list();
    expect(listed.items.some((w) => w.id === info.id)).toBe(true);
    expect(listed.active).toBe(info.id);
  });

  it("refuses every write until it is copied out", async () => {
    const info = await mounts.openRemote(FETCHED);

    await expect(mounts.assertWritable(info.path)).rejects.toThrow(
      /read-only copy of github.com\/acme\/apis/,
    );
  });

  it("leaves browser storage writable", async () => {
    await mounts.openRemote(FETCHED);

    await expect(mounts.assertWritable(mounts.BROWSER_PATH)).resolves.toBeUndefined();
  });

  it("copies out into an ordinary workspace and leaves the original alone", async () => {
    const info = await mounts.openRemote(FETCHED);

    const copy = await mounts.saveCopy(info.path, "Billing");

    expect(copy.id).toBe("browser");
    await expect(mounts.assertWritable(copy.path)).resolves.toBeUndefined();
    expect(await mounts.originOf(info.path)).not.toBeNull();
    expect(
      await mounts.vfsFor(copy.path).read("collections/billing/invoices.http"),
    ).toContain("api.billing.example");
  });

  it("copies beside a collection of the same name rather than over it", async () => {
    const browser = mounts.vfsFor(mounts.BROWSER_PATH);
    await browser.write(
      "collections/billing/mine.http",
      "### GET mine\nGET https://mine.dev/x\n",
    );
    const info = await mounts.openRemote(FETCHED);

    await mounts.saveCopy(info.path, "Billing");

    expect(await browser.read("collections/billing/mine.http")).toContain("mine.dev");
    expect(await browser.read("collections/billing-2/invoices.http")).toContain(
      "api.billing.example",
    );
  });

  it("can be forgotten, taking its files with it", async () => {
    const info = await mounts.openRemote(FETCHED);

    await mounts.forget(info.id);

    const listed = await mounts.list();
    expect(listed.items.some((w) => w.id === info.id)).toBe(false);
    expect(listed.active).toBe("browser");
  });
});
