import type {
  Finding,
  RemoteEnvironment,
  RemoteOrigin,
  RemoteReview,
  RemoteScript,
} from "../api";
import { SCAN_RULES } from "./scanRules.generated";
import { parseToml, stringifyToml } from "./toml";
import { MemoryVfs, type Vfs } from "./vfs";
import { listEnvironments, listTree, loadRequest } from "./workspace";

/**
 * The same bounds the desktop enforces. A repository nobody vetted must not be
 * able to hang the tab or fill browser storage, so the number of files, the size
 * of each one, the total and the depth are all capped before anything is read.
 */
export const MAX_FILES = 400;
export const MAX_TOTAL_BYTES = 4 * 1024 * 1024;
export const MAX_FILE_BYTES = 512 * 1024;
export const MAX_DEPTH = 10;

const ALLOWED = [
  "toml",
  "http",
  "rest",
  "grpc",
  "ws",
  "mqtt",
  "proto",
  "json",
  "txt",
  "graphql",
  "gql",
  "md",
  "xml",
  "csv",
];

const LOCAL_VALUE_FILES = /^(\.env(\..*)?|\.?secrets\.toml)$|\.(local|secret)\.toml$/i;

export interface Endpoints {
  api: string;
  raw: string;
}

export const GITHUB: Endpoints = {
  api: "https://api.github.com",
  raw: "https://raw.githubusercontent.com",
};

export type RemoteSourceRef =
  | {
      kind: "repo";
      owner: string;
      name: string;
      reference: string | null;
      subdir: string | null;
    }
  | { kind: "document"; url: string };

export interface RemoteFetch {
  origin: RemoteOrigin;
  files: [string, string][];
  skipped: string[];
  bytes: number;
}

export const CREDENTIAL_IN_URL =
  "That URL carries a username and password or token in it. Mándalo will not open a collection from a URL with a credential in it — the URL would be stored as where this workspace came from and shown in every message about it. Remove the credential from the URL. If the repository is private, open it in the Mándalo desktop app, which can sign in to GitHub; a web page is not a safe place to put a token.";

export const BUNDLE_NEEDS_DESKTOP =
  "A single-file Mándalo bundle is opened by the desktop app, which is the one build that can read a bundle. In the browser, open a public repository instead — the collections in it are plain .http and .grpc files the page can read directly.";

export const PRIVATE_NEEDS_DESKTOP =
  "GitHub did not return that repository. It may not exist, or it may be private — GitHub answers the same way for both when nobody is signed in. Private repositories need the Mándalo desktop app: that is where a GitHub sign-in can be kept on your own machine, and it clones the repository into a workspace you own and can push back to. Mándalo will never ask you for a GitHub token in a web page.";

function invalid(raw: string): Error {
  return new Error(
    `“${raw}” is not a collection Mándalo can open. Give a public GitHub repository as owner/name (optionally owner/name/sub/dir#branch), its https://github.com/… URL, or the URL of a single Mándalo bundle file.`,
  );
}

function validSegment(part: string): boolean {
  return part !== "" && part !== "." && part !== ".." && /^[A-Za-z0-9._-]+$/.test(part);
}

function carriesCredentials(url: string): boolean {
  const at = url.indexOf("://");
  if (at === -1) return false;
  const scheme = url.slice(0, at);
  if (scheme !== "http" && scheme !== "https") return false;
  const authority = url.slice(at + 3).split(/[/?#]/)[0] ?? "";
  return authority.includes("@");
}

function finishRepo(
  owner: string,
  name: string,
  reference: string | null,
  subdir: string | null,
  raw: string,
): RemoteSourceRef {
  const repo = name.endsWith(".git") ? name.slice(0, -4) : name;
  if (!validSegment(owner) || !validSegment(repo)) throw invalid(raw);
  if (reference !== null && !validSegment(reference)) throw invalid(raw);
  if (subdir !== null && !subdir.split("/").every(validSegment)) throw invalid(raw);
  return { kind: "repo", owner, name: repo, reference, subdir };
}

function parseRepo(rest: string, raw: string): RemoteSourceRef {
  const hash = rest.indexOf("#");
  const given = hash === -1 ? null : rest.slice(hash + 1) || null;
  const parts = (hash === -1 ? rest : rest.slice(0, hash))
    .split("/")
    .filter((p) => p !== "");
  if (parts.length < 2) throw invalid(raw);
  const owner = parts.shift() as string;
  const name = parts.shift() as string;
  if ((parts[0] === "tree" || parts[0] === "blob") && given === null && parts.length >= 2)
    return finishRepo(owner, name, parts[1], parts.slice(2).join("/") || null, raw);
  return finishRepo(owner, name, given, parts.join("/") || null, raw);
}

/** Mirrors `parse_source` in `crates/core/src/remote.rs`, form for form. */
export function parseSource(raw: string): RemoteSourceRef {
  const trimmed = raw.trim();
  if (trimmed === "") throw invalid(raw);
  if (
    trimmed.includes("://") &&
    !trimmed.startsWith("http://") &&
    !trimmed.startsWith("https://")
  )
    throw new Error(
      `${trimmed} is not an http or https URL — a public collection is read over the web.`,
    );
  if (carriesCredentials(trimmed)) throw new Error(CREDENTIAL_IN_URL);

  for (const prefix of [
    "github:",
    "https://github.com/",
    "http://github.com/",
    "https://www.github.com/",
  ]) {
    if (trimmed.startsWith(prefix))
      return parseRepo(trimmed.slice(prefix.length).replace(/\/+$/, ""), raw);
  }

  for (const prefix of [
    "https://raw.githubusercontent.com/",
    "http://raw.githubusercontent.com/",
  ]) {
    if (!trimmed.startsWith(prefix)) continue;
    const parts = trimmed.slice(prefix.length).split("/");
    if (parts.length >= 4 && parts.slice(3).join("/").endsWith(".json"))
      return { kind: "document", url: trimmed };
    if (parts.length < 3) throw invalid(raw);
    return finishRepo(parts[0], parts[1], parts[2], parts.slice(3).join("/") || null, raw);
  }

  if (trimmed.startsWith("https://") || trimmed.startsWith("http://"))
    return { kind: "document", url: trimmed };
  return parseRepo(trimmed.replace(/\/+$/, ""), raw);
}

function extensionOf(path: string): string {
  const name = path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
}

function whyRefused(path: string): string | null {
  const parts = path.split("/");
  if (parts.length > MAX_DEPTH) return `${path}: more than ${MAX_DEPTH} directories deep`;
  if (parts.some((p) => p === "" || p === "." || p === ".."))
    return `${path}: not a path inside the collection`;
  if (parts.some((p) => p.startsWith(".")))
    return `${path}: a dot directory or dotfile is never a request`;
  if (!ALLOWED.includes(extensionOf(path)))
    return `${path}: not a file a workspace is made of`;
  if (LOCAL_VALUE_FILES.test(parts[parts.length - 1] ?? ""))
    return `${path}: this file holds values that belong to one machine and is never adopted`;
  return null;
}

/**
 * `raw.githubusercontent.com` and `api.github.com` both send CORS headers, so a
 * public repository is readable straight from the page — no proxy of ours in the
 * middle, and nothing to trust but GitHub. Neither call carries a credential, and
 * there is no code path here that would accept one.
 */
async function getJson(url: string): Promise<unknown> {
  const answer = await fetch(url, { credentials: "omit", redirect: "follow" });
  if (answer.status === 401 || answer.status === 403 || answer.status === 404)
    throw new Error(PRIVATE_NEEDS_DESKTOP);
  if (!answer.ok) throw new Error(`GitHub answered ${answer.status} — try again in a moment`);
  return answer.json();
}

function subdirOf(path: string, subdir: string | null): string | null {
  if (subdir === null) return path;
  return path.startsWith(`${subdir}/`) ? path.slice(subdir.length + 1) : null;
}

export async function fetchRemote(
  source: RemoteSourceRef,
  endpoints: Endpoints = GITHUB,
): Promise<RemoteFetch> {
  if (source.kind === "document") throw new Error(BUNDLE_NEEDS_DESKTOP);

  const { owner, name, reference, subdir } = source;
  const api = endpoints.api.replace(/\/+$/, "");
  const head = (await getJson(`${api}/repos/${owner}/${name}/commits/${reference ?? "HEAD"}`)) as {
    sha?: string;
  };
  const commit = head.sha;
  if (typeof commit !== "string")
    throw new Error(`${owner}/${name} has no commit called “${reference ?? "HEAD"}”`);

  const tree = (await getJson(
    `${api}/repos/${owner}/${name}/git/trees/${commit}?recursive=1`,
  )) as { truncated?: boolean; tree?: { path?: string; type?: string; size?: number }[] };
  if (tree.truncated === true)
    throw new Error(
      `${owner}/${name} is too large to list in one read — point at a subdirectory, as owner/name/that/dir`,
    );

  const skipped: string[] = [];
  const wanted: { path: string; relative: string }[] = [];
  let declared = 0;
  for (const blob of tree.tree ?? []) {
    if (blob.type !== "blob" || typeof blob.path !== "string") continue;
    const relative = subdirOf(blob.path, subdir);
    if (relative === null) continue;
    const refused = whyRefused(relative);
    if (refused !== null) {
      skipped.push(refused);
      continue;
    }
    const size = blob.size ?? 0;
    if (size > MAX_FILE_BYTES) {
      skipped.push(
        `${relative}: ${size} bytes, over the ${MAX_FILE_BYTES} byte limit for one file`,
      );
      continue;
    }
    declared += size;
    if (declared > MAX_TOTAL_BYTES)
      throw new Error(
        `${owner}/${name} holds more than ${MAX_TOTAL_BYTES} bytes of collection files — Mándalo will not open it`,
      );
    if (wanted.length >= MAX_FILES)
      throw new Error(
        `${owner}/${name} holds more than ${MAX_FILES} collection files — Mándalo will not open it`,
      );
    wanted.push({ path: blob.path, relative });
  }
  if (wanted.length === 0)
    throw new Error(
      `${owner}/${name} holds no Mándalo collection — a collection is a directory with mandalo.toml and collections/`,
    );

  const base = `${endpoints.raw.replace(/\/+$/, "")}/${owner}/${name}/${commit}`;
  const files: [string, string][] = [];
  let bytes = 0;
  for (const item of wanted) {
    const answer = await fetch(`${base}/${item.path}`, {
      credentials: "omit",
      redirect: "follow",
    });
    if (!answer.ok) throw new Error(`${item.relative} answered ${answer.status}`);
    const text = await answer.text();
    bytes += text.length;
    if (bytes > MAX_TOTAL_BYTES)
      throw new Error(`${owner}/${name} sent more than ${MAX_TOTAL_BYTES} bytes`);
    files.push([item.relative, text]);
  }
  files.sort((a, b) => a[0].localeCompare(b[0]));

  if (
    !files.some(([path]) => path === "mandalo.toml") &&
    !files.some(([path]) => path.startsWith("collections/"))
  )
    throw new Error(
      `${owner}/${name} has no collections/ directory — it is a repository, but not a Mándalo workspace`,
    );

  return {
    origin: {
      label: `github.com/${owner}/${name}${reference ? `#${reference}` : ""}${
        subdir ? ` · ${subdir}` : ""
      }`,
      url: `https://github.com/${owner}/${name}`,
      commit,
      fetchedAt: Math.floor(Date.now() / 1000),
    },
    files,
    skipped,
    bytes,
  };
}

function scan(path: string, text: string): Finding[] {
  const out: Finding[] = [];
  const lines = text.split("\n");
  for (const [rule, pattern] of SCAN_RULES) {
    const regex = new RegExp(pattern);
    for (let i = 0; i < lines.length; i += 1) {
      const hit = regex.exec(lines[i] as string);
      if (hit === null) continue;
      if (rule === "base32-token" && (hit[0].match(/[2-7]/g) ?? []).length < 2) continue;
      const excerpt = hit[0].length > 24 ? `${hit[0].slice(0, 12)}…${hit[0].slice(-6)}` : hit[0];
      out.push({ path, line: i + 1, rule, excerpt });
      break;
    }
  }
  return out;
}

function hostOf(url: string): { host: string } | { template: string } | null {
  const trimmed = url.trim();
  if (trimmed === "") return null;
  const full = trimmed.includes("://") ? trimmed : `https://${trimmed}`;
  const after = full.slice(full.indexOf("://") + 3);
  const authority = after.split(/[/?#]/)[0] ?? "";
  if (authority.includes("{{")) return { template: trimmed };
  try {
    const parsed = new URL(full);
    return { host: parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname };
  } catch {
    return null;
  }
}

/**
 * Where the browser's answer is honestly weaker than the desktop's, and the
 * review says so rather than letting an absence of findings read as a clean
 * bill of health.
 */
export const SCANNER_NOTE =
  "Scanned in the browser for known credential shapes. The desktop app also weighs entropy and decodes embedded payloads, so open it there for the deeper pass.";

export async function reviewFetch(fetched: RemoteFetch): Promise<RemoteReview> {
  const vfs = new MemoryVfs();
  await materialize(fetched, vfs);

  const tree = await listTree(vfs);
  const environments = await listEnvironments(vfs);
  const corrupt = [...tree.skipped, ...environments.skipped];
  if (corrupt.length > 0)
    throw new Error(
      `This collection did not load whole — Mándalo could not read ${corrupt.join("; ")}. Nothing has been opened.`,
    );

  const hosts = new Set<string>();
  const templated = new Set<string>();
  const scripts: RemoteScript[] = [];
  let requests = 0;
  for (const node of tree.collections) {
    const paths: string[] = [];
    const walk = (folders: typeof node.folders) => {
      for (const folder of folders) {
        for (const request of folder.requests) paths.push(request.path);
        walk(folder.folders);
      }
    };
    for (const request of node.requests) paths.push(request.path);
    walk(node.folders);
    for (const path of paths) {
      const request = await loadRequest(vfs, node.slug, path);
      requests += 1;
      const found = hostOf(request.url);
      if (found !== null && "host" in found) hosts.add(found.host);
      else if (found !== null) templated.add(found.template);
      for (const hook of ["pre", "post"] as const) {
        const source = request.scripts?.[hook];
        if (typeof source === "string" && source !== "")
          scripts.push({
            collection: node.slug,
            request: request.name,
            hook,
            lines: source.split("\n").length,
          });
      }
    }
  }

  const envs: RemoteEnvironment[] = environments.items.map((env) => {
    const vars = Object.values(env.vars);
    return {
      name: env.name,
      declared: Object.keys(env.vars).sort(),
      sharedValues: vars.filter((v) => v.shared).length,
      awaitingValues: vars.filter((v) => !v.shared).length,
    };
  });

  const findings: Finding[] = [];
  for (const [path, text] of fetched.files) findings.push(...scan(path, text));

  return {
    origin: fetched.origin,
    files: fetched.files.length,
    bytes: fetched.bytes,
    collections: tree.collections.length,
    requests,
    environments: envs,
    hosts: [...hosts].sort(),
    templatedHosts: [...templated].sort(),
    scripts,
    findings,
    skipped: fetched.skipped,
    token: `${fetched.origin.url}${fetched.origin.commit ?? ""}${fetched.bytes}`,
  };
}

const MANIFEST = "mandalo.toml";

export async function materialize(fetched: RemoteFetch, vfs: Vfs): Promise<void> {
  for (const [path, text] of fetched.files) {
    if (whyRefused(path) !== null) throw new Error(`refusing to write ${path}`);
    await vfs.write(path, text);
  }
  if ((await vfs.read(MANIFEST)) === null)
    await vfs.write(
      MANIFEST,
      stringifyToml({
        schema_version: 1,
        id: crypto.randomUUID(),
        name: fetched.origin.label,
      }),
    );
}

/** Stamps the workspace as somebody else's, which is what makes it read-only. */
export async function stampOrigin(vfs: Vfs, origin: RemoteOrigin): Promise<void> {
  const raw = (await vfs.read(MANIFEST)) ?? "";
  const table = raw === "" ? {} : (parseToml(raw) as Record<string, unknown>);
  await vfs.write(
    MANIFEST,
    stringifyToml({
      schema_version: 1,
      id: (table.id as string) ?? crypto.randomUUID(),
      name: (table.name as string) ?? origin.label,
      remote: {
        label: origin.label,
        url: origin.url,
        ...(origin.commit === null ? {} : { commit: origin.commit }),
        fetchedAt: origin.fetchedAt,
      },
    }),
  );
}

export async function readOrigin(vfs: Vfs): Promise<RemoteOrigin | null> {
  const raw = await vfs.read(MANIFEST);
  if (raw === null) return null;
  try {
    const table = parseToml(raw) as Record<string, unknown>;
    const remote = table.remote as Record<string, unknown> | undefined;
    if (remote === undefined) return null;
    return {
      label: String(remote.label ?? "somewhere else"),
      url: String(remote.url ?? ""),
      commit: remote.commit === undefined ? null : String(remote.commit),
      fetchedAt: Number(remote.fetchedAt ?? 0),
    };
  } catch {
    return null;
  }
}

/** The deep link: `mandalo.dev/app?repo=owner/name` opens that collection. */
export function sourceFromLocation(search: string): string | null {
  const params = new URLSearchParams(search);
  const repo = params.get("repo");
  if (repo !== null && repo.trim() !== "") return repo.trim();
  const bundle = params.get("bundle");
  return bundle !== null && bundle.trim() !== "" ? bundle.trim() : null;
}
