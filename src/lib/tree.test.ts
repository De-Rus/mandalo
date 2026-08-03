import { describe, expect, it } from "vitest";
import type { CollectionNode } from "./api";
import {
  applyDraftOverrides,
  collectionRequestCount,
  filterTree,
  flattenTree,
  folderOf,
  locateRequests,
} from "./tree";

const TREE: CollectionNode[] = [
  {
    id: "c1",
    slug: "acme",
    name: "Acme API",
    folders: [
      {
        name: "Users",
        path: "users",
        folders: [
          {
            name: "Admin",
            path: "users/admin",
            folders: [],
            requests: [
              {
                id: "r3",
                name: "Impersonate",
                kind: "http",
                method: "POST",
                path: "users/admin/impersonate.toml",
              },
            ],
          },
        ],
        requests: [
          {
            id: "r1",
            name: "List users",
            kind: "http",
            method: "GET",
            path: "users/list.toml",
          },
        ],
      },
    ],
    requests: [
      {
        id: "r2",
        name: "Health check",
        kind: "http",
        method: "GET",
        path: "health.toml",
      },
    ],
  },
  {
    id: "c2",
    slug: "playground",
    name: "Playground",
    folders: [],
    requests: [
      {
        id: "r4",
        name: "Rates",
        kind: "http",
        method: "GET",
        path: "rates.toml",
      },
    ],
  },
];

describe("tree", () => {
  it("counts every request in a collection, nested folders included", () => {
    expect(collectionRequestCount(TREE[0])).toBe(3);
  });

  it("keeps only matching branches when filtering", () => {
    const filtered = filterTree(TREE, "imperson");
    expect(filtered).toHaveLength(1);
    expect(filtered[0].requests).toHaveLength(0);
    expect(filtered[0].folders[0].folders[0].requests[0].name).toBe(
      "Impersonate",
    );
  });

  it("drops collections with no match at all", () => {
    expect(filterTree(TREE, "rates").map((c) => c.slug)).toEqual(["playground"]);
    expect(filterTree(TREE, "zzzz")).toEqual([]);
  });

  it("returns the whole tree for a blank query", () => {
    expect(filterTree(TREE, "   ")).toBe(TREE);
  });

  it("flattens requests with their collection and folder path", () => {
    const flat = flattenTree(TREE);
    expect(flat.map((r) => r.id)).toEqual(["r2", "r1", "r3", "r4"]);
    expect(flat.find((r) => r.id === "r3")?.folder).toBe("Users / Admin");
    expect(flat.find((r) => r.id === "r2")?.folder).toBeNull();
  });

  it("locates every request by id", () => {
    const located = locateRequests(TREE);
    expect(located.get("r3")).toEqual({
      collection: "acme",
      path: "users/admin/impersonate.toml",
    });
    expect(located.size).toBe(4);
  });

  it("overrides tree labels with unsaved draft values", () => {
    const overridden = applyDraftOverrides(TREE, [
      { id: "r1", name: "List users v2", kind: "graphql", method: "POST" },
    ]);
    expect(overridden[0].folders[0].requests[0].name).toBe("List users v2");
    expect(overridden[0].folders[0].requests[0].kind).toBe("graphql");
    expect(overridden[0].requests[0].name).toBe("Health check");
  });

  it("derives the parent folder of a request path", () => {
    expect(folderOf("users/admin/impersonate.toml")).toBe("users/admin");
    expect(folderOf("health.toml")).toBe("");
  });
});
