#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EXTENSION_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = resolve(EXTENSION_ROOT, "..");
const REPO_SLUG = "De-Rus/mandalo";

const TARGET_TRIPLES = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function die(reason) {
  process.stderr.write(`fetch-cli: ${reason}\n`);
  process.exit(1);
}

function parseArgs(argv) {
  const flags = { target: undefined, version: undefined, out: undefined, build: false, force: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--build") {
      flags.build = true;
    } else if (arg === "--force") {
      flags.force = true;
    } else if (arg === "--target" || arg === "--version" || arg === "--out") {
      const value = argv[i + 1];
      if (value === undefined || value.startsWith("--")) die(`${arg} needs a value`);
      flags[arg.slice(2)] = value;
      i += 1;
    } else {
      die(`unknown argument "${arg}"`);
    }
  }
  return flags;
}

function hostTarget() {
  const target = `${process.platform}-${process.arch}`;
  return target in TARGET_TRIPLES ? target : null;
}

function extensionVersion() {
  const manifest = join(EXTENSION_ROOT, "package.json");
  try {
    const version = JSON.parse(readFileSync(manifest, "utf8")).version;
    if (typeof version !== "string" || version === "") die(`no "version" field in ${manifest}`);
    return version;
  } catch (error) {
    die(`cannot read ${manifest}: ${error.message}`);
  }
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function parseDigest(text) {
  const first = text.trim().split(/\s+/)[0] ?? "";
  return /^[0-9a-f]{64}$/i.test(first) ? first.toLowerCase() : null;
}

function runBinaryVersion(binary) {
  const result = spawnSync(binary, ["--version"], { encoding: "utf8" });
  if (result.error) die(`cannot run ${binary}: ${result.error.message}`);
  if (result.status !== 0) die(`${binary} --version exited with code ${result.status}`);
  return `${result.stdout}${result.stderr}`.trim();
}

function verify(binary, expectedVersion, runnable) {
  if (!runnable) {
    process.stdout.write("verify: skipped (binary is not runnable on this host)\n");
    return;
  }
  const actual = runBinaryVersion(binary);
  const expected = `mandalo ${expectedVersion}`;
  if (actual !== expected) {
    rmSync(binary, { force: true });
    die(`version mismatch: ${binary} reports "${actual}", expected "${expected}"`);
  }
  process.stdout.write(`verify: ${actual}\n`);
}

async function fetchAsset(url) {
  let response;
  try {
    response = await fetch(url, { redirect: "follow" });
  } catch (error) {
    return { ok: false, missing: false, reason: error.message };
  }
  if (response.status === 404) return { ok: false, missing: true, reason: "404" };
  if (!response.ok) return { ok: false, missing: false, reason: `HTTP ${response.status}` };
  return { ok: true, response };
}

async function download(assetUrl, outPath, envDigest) {
  const asset = await fetchAsset(assetUrl);
  if (!asset.ok) return { ok: false, missing: asset.missing, reason: asset.reason };

  let expected = envDigest;
  if (expected === undefined) {
    const sidecar = await fetchAsset(`${assetUrl}.sha256`);
    if (!sidecar.ok) return { ok: false, missing: sidecar.missing, reason: `no checksum (${sidecar.reason})` };
    expected = parseDigest(await sidecar.response.text());
    if (expected === null) die(`malformed checksum sidecar at ${assetUrl}.sha256`);
  }

  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, Buffer.from(await asset.response.arrayBuffer()));
  const actual = sha256(outPath);
  if (actual !== expected) {
    rmSync(outPath, { force: true });
    die(`checksum mismatch for ${assetUrl}: got ${actual}, expected ${expected}`);
  }
  return { ok: true };
}

function cargoBuild(triple, binaryName, outPath) {
  // release-cli is the size-tuned profile the released binaries use; a VSIX built
  // with plain --release would be ~7 MB heavier than the one people download.
  const args = ["build", "--profile", "release-cli", "-p", "mandalo-cli", "--target", triple];
  process.stdout.write(`cargo ${args.join(" ")}\n`);
  const result = spawnSync("cargo", args, { cwd: REPO_ROOT, stdio: "inherit" });
  if (result.error) die(`cannot run cargo: ${result.error.message}`);
  if (result.status !== 0) die(`cargo build failed with code ${result.status}`);

  const built = join(REPO_ROOT, "target", triple, "release-cli", binaryName);
  if (!existsSync(built)) die(`cargo build succeeded but ${built} is missing`);
  mkdirSync(dirname(outPath), { recursive: true });
  copyFileSync(built, outPath);
}

async function main() {
  const flags = parseArgs(process.argv.slice(2));
  const target = flags.target ?? process.env.MANDALO_CLI_TARGET ?? hostTarget();

  if (target === "universal") {
    process.stdout.write("target universal: a universal VSIX carries no CLI binary, nothing to do\n");
    return;
  }
  if (target === null) {
    die(`no CLI build for host ${process.platform}-${process.arch}; pass --target explicitly`);
  }
  const triple = TARGET_TRIPLES[target];
  if (triple === undefined) {
    die(`unknown target "${target}"; expected one of ${Object.keys(TARGET_TRIPLES).join(", ")}, or universal`);
  }

  const version = flags.version ?? extensionVersion();
  const outDir = flags.out === undefined ? join(EXTENSION_ROOT, "bin") : resolve(flags.out);
  const isWindows = target.startsWith("win32");
  const binaryName = isWindows ? "mandalo.exe" : "mandalo";
  const outPath = join(outDir, binaryName);
  const runnable = target === hostTarget();

  if (existsSync(outPath) && !flags.force) {
    verify(outPath, version, runnable);
    const bytes = statSync(outPath).size;
    process.stdout.write(
      `source=reused target=${target} bytes=${bytes} sha256=${sha256(outPath)}\n` +
        `reusing existing ${outPath} (pass --force to replace it)\n`,
    );
    return;
  }

  const envDigest = process.env.MANDALO_CLI_SHA256;
  if (envDigest !== undefined && parseDigest(envDigest) === null) {
    die("MANDALO_CLI_SHA256 is set but is not a 64-char hex digest");
  }

  const assetUrl = `https://github.com/${REPO_SLUG}/releases/download/v${version}/mandalo-${target}${isWindows ? ".exe" : ""}`;
  process.stdout.write(`downloading ${assetUrl}\n`);
  const downloaded = await download(assetUrl, outPath, envDigest === undefined ? undefined : parseDigest(envDigest));

  let source = "download";
  if (!downloaded.ok) {
    if (!downloaded.missing) die(`download failed: ${downloaded.reason}`);
    process.stdout.write(`no release asset (${downloaded.reason}), falling back to cargo\n`);
    if (!runnable && !flags.build) {
      die(
        `refusing to cross-compile ${target} from ${process.platform}-${process.arch} implicitly; pass --build to allow it`,
      );
    }
    cargoBuild(triple, binaryName, outPath);
    source = "cargo";
  }

  if (!isWindows) chmodSync(outPath, 0o755);
  verify(outPath, version, runnable);
  process.stdout.write(
    `source=${source} target=${target} bytes=${statSync(outPath).size} sha256=${sha256(outPath)}\n` +
      `wrote ${outPath}\n`,
  );
}

main().catch((error) => die(error.stack ?? String(error)));
