import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  BUNDLE_NEEDS_DESKTOP,
  CREDENTIAL_IN_URL,
  MAX_FILE_BYTES,
  MAX_FILES,
  PRIVATE_NEEDS_DESKTOP,
  fetchRemote,
  parseSource,
  readOrigin,
  reviewFetch,
  sourceFromLocation,
  stampOrigin,
} from "./remote";
import { MemoryVfs } from "./vfs.testkit";

const SHA = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";
const ENDPOINTS = { api: "https://api.test", raw: "https://raw.test" };

const REPO: [string, string][] = [
  ["mandalo.toml", 'schema_version = 1\nid = "shared"\nname = "Shared APIs"\n'],
  [
    "collections/billing/collection.toml",
    'schema_version = 1\nid = "billing"\nname = "Billing"\n',
  ],
  [
    "collections/billing/invoices.http",
    [
      "### GET invoices",
      "GET https://api.billing.example/invoices",
      "",
      "### POST charge",
      "POST https://payments.example.dev/charges",
      "",
      "> {%",
      'pm.environment.set("last", "x");',
      "%}",
      "",
    ].join("\n"),
  ],
  ["collections/billing/lookup.http", "### GET lookup\nGET {{baseUrl}}/lookup\n"],
  [
    "environments/staging.toml",
    'name = "staging"\n\n[vars]\nbaseUrl = "https://staging.example"\n\n[vars.apiToken]\nsecret = true\nhosts = ["api.billing.example"]\n',
  ],
];

function route(files: [string, string][], sizes?: Map<string, number>) {
  return (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === `${ENDPOINTS.api}/repos/acme/apis/commits/HEAD`)
      return Promise.resolve(
        new Response(JSON.stringify({ sha: SHA }), { status: 200 }),
      );
    if (url === `${ENDPOINTS.api}/repos/acme/apis/git/trees/${SHA}?recursive=1`)
      return Promise.resolve(
        new Response(
          JSON.stringify({
            truncated: false,
            tree: files.map(([path, text]) => ({
              path,
              type: "blob",
              size: sizes?.get(path) ?? text.length,
            })),
          }),
          { status: 200 },
        ),
      );
    const prefix = `${ENDPOINTS.raw}/acme/apis/${SHA}/`;
    if (url.startsWith(prefix)) {
      const found = files.find(([path]) => path === url.slice(prefix.length));
      if (found) return Promise.resolve(new Response(found[1], { status: 200 }));
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  };
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn(route(REPO));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("the URL forms a collection can be named by", () => {
  it("reads owner/name, a branch and a subdirectory", () => {
    expect(parseSource("acme/apis")).toEqual({
      kind: "repo",
      owner: "acme",
      name: "apis",
      reference: null,
      subdir: null,
    });
    expect(parseSource("acme/apis/billing/v2#next")).toEqual({
      kind: "repo",
      owner: "acme",
      name: "apis",
      reference: "next",
      subdir: "billing/v2",
    });
  });

  it("reads the url a user copies out of the address bar", () => {
    expect(parseSource("https://github.com/acme/apis/tree/main/billing")).toEqual({
      kind: "repo",
      owner: "acme",
      name: "apis",
      reference: "main",
      subdir: "billing",
    });
    expect(parseSource("https://github.com/acme/apis.git")).toMatchObject({
      owner: "acme",
      name: "apis",
    });
  });

  it("refuses nonsense, other protocols and a url with a credential in it", () => {
    expect(() => parseSource("")).toThrow();
    expect(() => parseSource("acme")).toThrow();
    expect(() => parseSource("acme/../etc")).toThrow();
    expect(() => parseSource("ssh://git@github.com/acme/apis")).toThrow(/http or https/);
    expect(() => parseSource("https://ghp_0000000000@github.com/acme/apis")).toThrow(
      CREDENTIAL_IN_URL,
    );
  });

  it("never echoes the credential it refused", () => {
    try {
      parseSource("https://ghp_secretsecretsecret@github.com/acme/apis");
      throw new Error("should have refused");
    } catch (e) {
      expect(String(e)).not.toContain("ghp_secret");
    }
  });
});

describe("reading a public repository", () => {
  it("goes straight to GitHub, with no proxy of ours and no credential", async () => {
    await fetchRemote(parseSource("acme/apis"), ENDPOINTS);

    for (const call of fetchMock.mock.calls) {
      expect(String(call[0])).toMatch(/^https:\/\/(api|raw)\.test\//);
      expect(call[1]).toMatchObject({ credentials: "omit" });
      expect(JSON.stringify(call[1] ?? {})).not.toMatch(/authorization|token/i);
    }
  });

  it("describes what it is before anything is opened", async () => {
    const fetched = await fetchRemote(parseSource("acme/apis"), ENDPOINTS);
    const review = await reviewFetch(fetched);

    expect(review.collections).toBe(1);
    expect(review.requests).toBe(3);
    expect(review.hosts).toEqual([
      "api.billing.example",
      "payments.example.dev",
    ]);
    expect(review.templatedHosts).toEqual(["{{baseUrl}}/lookup"]);
    expect(review.scripts).toHaveLength(1);
    expect(review.environments[0]).toMatchObject({
      name: "staging",
      sharedValues: 1,
      awaitingValues: 1,
    });
    expect(review.origin.commit).toBe(SHA);
  });

  it("surfaces a credential the repository carries", async () => {
    const leaky: [string, string][] = [
      ...REPO,
      [
        "collections/billing/leaky.http",
        "### GET leaky\nGET https://api.billing.example/me\nAuthorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk\n",
      ],
    ];
    vi.stubGlobal("fetch", vi.fn(route(leaky)));

    const review = await reviewFetch(
      await fetchRemote(parseSource("acme/apis"), ENDPOINTS),
    );

    expect(review.findings.some((f) => f.rule === "jwt")).toBe(true);
  });

  it("skips a dotfile, a machine-local values file and anything not a workspace file", async () => {
    const noisy: [string, string][] = [
      ...REPO,
      [".env", "TOKEN=hunter2hunter2\n"],
      ["scripts/run.sh", "#!/bin/sh\n"],
      ["collections/billing/.secrets.toml", "x = 1\n"],
    ];
    vi.stubGlobal("fetch", vi.fn(route(noisy)));

    const fetched = await fetchRemote(parseSource("acme/apis"), ENDPOINTS);

    expect(fetched.files.map(([p]) => p)).not.toContain(".env");
    expect(fetched.files.map(([p]) => p)).not.toContain("scripts/run.sh");
    expect(fetched.skipped.join(" ")).toContain(".env");
  });

  it("never even asks for a file over the per-file cap", async () => {
    const sizes = new Map([["collections/billing/lookup.http", MAX_FILE_BYTES + 1]]);
    const mock = vi.fn(route(REPO, sizes));
    vi.stubGlobal("fetch", mock);

    const fetched = await fetchRemote(parseSource("acme/apis"), ENDPOINTS);

    expect(fetched.skipped.join(" ")).toContain("lookup.http");
    expect(
      mock.mock.calls.some((call) => String(call[0]).includes("lookup.http")),
    ).toBe(false);
  });

  it("refuses a repository with too many files", async () => {
    const many: [string, string][] = Array.from(
      { length: MAX_FILES + 5 },
      (_, n) => [`collections/billing/r${n}.http`, "### GET x\nGET https://x.dev/\n"],
    );
    vi.stubGlobal("fetch", vi.fn(route(many)));

    await expect(fetchRemote(parseSource("acme/apis"), ENDPOINTS)).rejects.toThrow(
      /more than/,
    );
  });

  it("fails loud on a repository that is not a workspace", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(route([["README.md", "# hello\n"]])),
    );

    await expect(fetchRemote(parseSource("acme/apis"), ENDPOINTS)).rejects.toThrow(
      /not a Mándalo workspace|no Mándalo collection/,
    );
  });

  it("fails loud rather than half-loading a malformed collection", async () => {
    const broken: [string, string][] = REPO.map(([path, text]) =>
      path === "collections/billing/collection.toml"
        ? [path, "not toml [[["]
        : [path, text],
    );
    vi.stubGlobal("fetch", vi.fn(route(broken)));

    await expect(
      reviewFetch(await fetchRemote(parseSource("acme/apis"), ENDPOINTS)),
    ).rejects.toThrow(/did not load whole/);
  });
});

describe("what the browser will not do", () => {
  it("says a private repository needs the desktop app and offers no token field", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response("{}", { status: 404 }))),
    );

    await expect(fetchRemote(parseSource("acme/apis"), ENDPOINTS)).rejects.toThrow(
      PRIVATE_NEEDS_DESKTOP,
    );
    expect(PRIVATE_NEEDS_DESKTOP).toMatch(/never ask you for a GitHub token/);
  });

  it("sends a single bundle url to the desktop rather than half-reading it", async () => {
    await expect(
      fetchRemote(parseSource("https://acme.dev/team.json"), ENDPOINTS),
    ).rejects.toThrow(BUNDLE_NEEDS_DESKTOP);
  });
});

describe("the read-only stamp", () => {
  it("survives a round trip through the manifest", async () => {
    const vfs = new MemoryVfs();
    await vfs.write("mandalo.toml", 'schema_version = 1\nid = "x"\nname = "X"\n');

    await stampOrigin(vfs, {
      label: "github.com/acme/apis",
      url: "https://github.com/acme/apis",
      commit: SHA,
      fetchedAt: 1234,
    });

    expect(await readOrigin(vfs)).toEqual({
      label: "github.com/acme/apis",
      url: "https://github.com/acme/apis",
      commit: SHA,
      fetchedAt: 1234,
    });
  });

  it("is absent from a workspace the user owns", async () => {
    const vfs = new MemoryVfs();
    await vfs.write("mandalo.toml", 'schema_version = 1\nid = "x"\nname = "X"\n');

    expect(await readOrigin(vfs)).toBeNull();
  });
});

describe("the deep link", () => {
  it("reads ?repo and ?bundle and nothing else", () => {
    expect(sourceFromLocation("?repo=acme/apis")).toBe("acme/apis");
    expect(sourceFromLocation("?bundle=https://acme.dev/t.json")).toBe(
      "https://acme.dev/t.json",
    );
    expect(sourceFromLocation("?token=abc")).toBeNull();
    expect(sourceFromLocation("")).toBeNull();
  });
});
