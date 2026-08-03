import { describe, expect, it } from "vitest";
import {
  bundledBinaryName,
  bundledBinaryPath,
  cliMissingExplanation,
  lookupOnPath,
  resolveCli,
  type ResolveInput,
} from "../../src/core/resolve";

function probe(...files: string[]): (p: string) => boolean {
  const set = new Set(files);
  return (p) => set.has(p);
}

function posixInput(overrides: Partial<ResolveInput> = {}): ResolveInput {
  return {
    settingPath: undefined,
    bundledPath: "/ext/bin/mandalo",
    platform: "darwin",
    pathEnv: "/usr/local/bin:/usr/bin",
    pathExt: undefined,
    isFile: probe(),
    ...overrides,
  };
}

function winInput(overrides: Partial<ResolveInput> = {}): ResolveInput {
  return {
    settingPath: undefined,
    bundledPath: "C:\\ext\\bin\\mandalo.exe",
    platform: "win32",
    pathEnv: "C:\\tools;C:\\Windows\\System32",
    pathExt: ".COM;.EXE;.CMD",
    isFile: probe(),
    ...overrides,
  };
}

describe("bundled binary naming", () => {
  it("appends .exe only on win32", () => {
    expect(bundledBinaryName("darwin")).toBe("mandalo");
    expect(bundledBinaryName("linux")).toBe("mandalo");
    expect(bundledBinaryName("win32")).toBe("mandalo.exe");
  });

  it("places the binary under bin/ with platform separators", () => {
    expect(bundledBinaryPath("/ext", "linux")).toBe("/ext/bin/mandalo");
    expect(bundledBinaryPath("/ext/", "linux")).toBe("/ext/bin/mandalo");
    expect(bundledBinaryPath("C:\\ext", "win32")).toBe("C:\\ext\\bin\\mandalo.exe");
  });
});

describe("resolution order", () => {
  it("prefers the bundled binary when it exists", () => {
    const input = posixInput({
      isFile: probe("/ext/bin/mandalo", "/usr/local/bin/mandalo"),
    });
    expect(resolveCli(input)).toEqual({ binary: "/ext/bin/mandalo", source: "bundled" });
  });

  it("falls back to PATH when the bundled binary is missing", () => {
    const input = posixInput({ isFile: probe("/usr/bin/mandalo") });
    expect(resolveCli(input)).toEqual({ binary: "/usr/bin/mandalo", source: "path" });
  });

  it("falls back to PATH when no bundled path is known at all", () => {
    const input = posixInput({ bundledPath: undefined, isFile: probe("/usr/bin/mandalo") });
    expect(resolveCli(input)).toEqual({ binary: "/usr/bin/mandalo", source: "path" });
  });

  it("returns null when the CLI is nowhere", () => {
    expect(resolveCli(posixInput())).toBeNull();
  });

  it("lets an explicit setting win over both bundled and PATH", () => {
    const input = posixInput({
      settingPath: "/opt/custom/mandalo",
      isFile: probe("/ext/bin/mandalo", "/usr/local/bin/mandalo"),
    });
    expect(resolveCli(input)).toEqual({ binary: "/opt/custom/mandalo", source: "setting" });
  });

  it("honours a setting that points at a file which does not exist", () => {
    const input = posixInput({ settingPath: "/typo/mandalo", isFile: probe("/ext/bin/mandalo") });
    expect(resolveCli(input)).toEqual({ binary: "/typo/mandalo", source: "setting" });
  });

  it("treats a blank setting as unset", () => {
    const input = posixInput({ settingPath: "   ", isFile: probe("/ext/bin/mandalo") });
    expect(resolveCli(input)).toEqual({ binary: "/ext/bin/mandalo", source: "bundled" });
  });

  it("trims a setting before using it", () => {
    const input = posixInput({ settingPath: "  /opt/mandalo  " });
    expect(resolveCli(input)).toEqual({ binary: "/opt/mandalo", source: "setting" });
  });
});

describe("PATH lookup", () => {
  it("scans directories in order", () => {
    const input = posixInput({ isFile: probe("/usr/local/bin/mandalo", "/usr/bin/mandalo") });
    expect(lookupOnPath(input)).toBe("/usr/local/bin/mandalo");
  });

  it("does not select a PATH entry that resolves to a directory", () => {
    const dirs = new Set(["/usr/local/bin/mandalo"]);
    const input = posixInput({
      isFile: (p) => !dirs.has(p) && p === "/usr/bin/mandalo",
    });
    expect(lookupOnPath(input)).toBe("/usr/bin/mandalo");
  });

  it("returns null when PATH is unset", () => {
    expect(lookupOnPath(posixInput({ pathEnv: undefined }))).toBeNull();
  });

  it("skips blank PATH segments and strips quotes", () => {
    const input = posixInput({
      pathEnv: '::"/opt/bin":',
      isFile: probe("/opt/bin/mandalo"),
    });
    expect(lookupOnPath(input)).toBe("/opt/bin/mandalo");
  });

  it("splits on ; and appends PATHEXT suffixes on win32", () => {
    const input = winInput({ isFile: probe("C:\\Windows\\System32\\mandalo.CMD") });
    expect(lookupOnPath(input)).toBe("C:\\Windows\\System32\\mandalo.CMD");
  });

  it("matches PATHEXT case-insensitively on win32", () => {
    const input = winInput({ isFile: probe("C:\\tools\\mandalo.exe") });
    expect(lookupOnPath(input)).toBe("C:\\tools\\mandalo.exe");
  });

  it("falls back to the default PATHEXT when the variable is unset", () => {
    const input = winInput({ pathExt: undefined, isFile: probe("C:\\tools\\mandalo.EXE") });
    expect(lookupOnPath(input)).toBe("C:\\tools\\mandalo.EXE");
  });

  it("never accepts the bare stem on win32", () => {
    const input = winInput({ isFile: probe("C:\\tools\\mandalo") });
    expect(lookupOnPath(input)).toBeNull();
  });
});

describe("missing-CLI explanation", () => {
  it("names the platform, the expected bundled path and the setting", () => {
    const message = cliMissingExplanation("win32", "C:\\ext\\bin\\mandalo.exe");
    expect(message).toContain("win32");
    expect(message).toContain("C:\\ext\\bin\\mandalo.exe");
    expect(message).toContain("mandalo.cliPath");
  });

  it("says so when no bundled path is known", () => {
    expect(cliMissingExplanation("linux", undefined)).toContain("no bundled binary");
  });
});
