import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { bundledBinaryName, resolveCli } from "../../../src/core/resolve";

const extensionRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const repoRoot = resolve(extensionRoot, "..");

export interface BinaryProbe {
  binary: string | null;
  reason: string;
}

/** Resolves the CLI exactly as the extension does, then the repo's own build outputs. */
export function probeCli(): BinaryProbe {
  const stem = bundledBinaryName(process.platform);
  const resolved = resolveCli({
    settingPath: undefined,
    bundledPath: join(extensionRoot, "bin", stem),
    platform: process.platform,
    pathEnv: process.env["PATH"],
    pathExt: process.env["PATHEXT"],
    isFile: existsSync,
  });
  const candidates = [
    process.env["MANDALO_BIN"],
    resolved?.binary,
    join(repoRoot, "target", "release", stem),
    join(repoRoot, "target", "debug", stem),
  ].filter((value): value is string => value !== undefined);

  const tried: string[] = [];
  for (const candidate of candidates) {
    if (!existsSync(candidate)) {
      tried.push(`${candidate} (absent)`);
      continue;
    }
    const help = spawnSync(candidate, ["--help"], { timeout: 20_000, encoding: "utf8" });
    if (help.status === 0 && (help.stdout ?? "").includes("<COMMAND>")) {
      return { binary: candidate, reason: "" };
    }
    tried.push(`${candidate} (not the Mándalo CLI)`);
  }
  return {
    binary: null,
    reason:
      `no usable mandalo CLI: ${tried.join(", ")}. Run \`node scripts/fetch-cli.mjs\` from ` +
      "editor-extension/, or set MANDALO_BIN.",
  };
}

/** CI sets this so a missing binary fails the suite instead of quietly skipping it. */
export function cliIsRequired(): boolean {
  return process.env["MANDALO_REQUIRE_CLI"] === "1";
}
