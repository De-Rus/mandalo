export type CliSource = "setting" | "bundled" | "path";

export interface ResolvedCli {
  binary: string;
  source: CliSource;
}

export interface ResolveInput {
  settingPath: string | undefined;
  bundledPath: string | undefined;
  platform: NodeJS.Platform;
  pathEnv: string | undefined;
  pathExt: string | undefined;
  isFile: (p: string) => boolean;
}

const CLI_STEM = "mandalo";
const DEFAULT_PATHEXT = ".COM;.EXE;.BAT;.CMD";
const DOWNLOAD_URL = "https://github.com/De-Rus/mandalo/releases/latest";

function isWindows(platform: NodeJS.Platform): boolean {
  return platform === "win32";
}

function pathSeparator(platform: NodeJS.Platform): string {
  return isWindows(platform) ? "\\" : "/";
}

function pathDelimiter(platform: NodeJS.Platform): string {
  return isWindows(platform) ? ";" : ":";
}

function joinPath(platform: NodeJS.Platform, dir: string, name: string): string {
  return `${dir.replace(/[\\/]+$/, "")}${pathSeparator(platform)}${name}`;
}

export function bundledBinaryName(platform: NodeJS.Platform): string {
  return isWindows(platform) ? `${CLI_STEM}.exe` : CLI_STEM;
}

export function bundledBinaryPath(extensionPath: string, platform: NodeJS.Platform): string {
  return joinPath(platform, joinPath(platform, extensionPath, "bin"), bundledBinaryName(platform));
}

function executableCandidates(platform: NodeJS.Platform, pathExt: string | undefined): string[] {
  if (!isWindows(platform)) return [CLI_STEM];
  const raw = pathExt === undefined || pathExt.trim() === "" ? DEFAULT_PATHEXT : pathExt;
  const seen = new Set<string>();
  const names: string[] = [];
  for (const entry of raw.split(";")) {
    const ext = entry.trim();
    if (ext === "") continue;
    for (const variant of [ext, ext.toLowerCase(), ext.toUpperCase()]) {
      const name = `${CLI_STEM}${variant}`;
      if (seen.has(name)) continue;
      seen.add(name);
      names.push(name);
    }
  }
  return names;
}

function pathDirectories(platform: NodeJS.Platform, pathEnv: string | undefined): string[] {
  if (pathEnv === undefined) return [];
  return pathEnv
    .split(pathDelimiter(platform))
    .map((entry) => entry.trim().replace(/^"(.*)"$/, "$1"))
    .filter((entry) => entry !== "");
}

export function lookupOnPath(input: ResolveInput): string | null {
  const names = executableCandidates(input.platform, input.pathExt);
  for (const dir of pathDirectories(input.platform, input.pathEnv)) {
    for (const name of names) {
      const candidate = joinPath(input.platform, dir, name);
      if (input.isFile(candidate)) return candidate;
    }
  }
  return null;
}

export function resolveCli(input: ResolveInput): ResolvedCli | null {
  const setting = input.settingPath?.trim();
  if (setting !== undefined && setting !== "") return { binary: setting, source: "setting" };

  if (input.bundledPath !== undefined && input.isFile(input.bundledPath)) {
    return { binary: input.bundledPath, source: "bundled" };
  }

  const found = lookupOnPath(input);
  return found === null ? null : { binary: found, source: "path" };
}

export function cliMissingExplanation(
  platform: NodeJS.Platform,
  bundledPath: string | undefined,
): string {
  const expected =
    bundledPath === undefined
      ? "this install carries no bundled binary"
      : `no bundled binary at ${bundledPath}`;
  return [
    `Mándalo CLI not found for ${platform} — ${expected}, and "${bundledBinaryName(platform)}" is not on PATH.`,
    `Install a platform-specific build of the extension, or download the CLI from ${DOWNLOAD_URL} and point the "mandalo.cliPath" setting at it.`,
  ].join(" ");
}
