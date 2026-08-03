import { chmodSync, constants, accessSync, statSync } from "node:fs";
import * as vscode from "vscode";
import { CliUnavailableError } from "./core/cli";
import {
  bundledBinaryPath,
  cliMissingExplanation,
  resolveCli,
  type ResolvedCli,
} from "./core/resolve";

function isFile(candidate: string): boolean {
  try {
    return statSync(candidate).isFile();
  } catch {
    return false;
  }
}

/** The explicit setting only counts when the user actually wrote one. */
function explicitCliPath(): string | undefined {
  const inspected = vscode.workspace.getConfiguration("mandalo").inspect<string>("cliPath");
  const value =
    inspected?.workspaceFolderValue ?? inspected?.workspaceValue ?? inspected?.globalValue;
  return value === undefined || value.trim() === "" ? undefined : value;
}

export class CliResolver {
  private announced: string | undefined;
  private versionChecked = false;

  constructor(
    private readonly extensionPath: string,
    private readonly extensionVersion: string,
    private readonly log: (line: string) => void,
  ) {}

  bundledPath(): string {
    return bundledBinaryPath(this.extensionPath, process.platform);
  }

  find(): ResolvedCli | null {
    return resolveCli({
      settingPath: explicitCliPath(),
      bundledPath: this.bundledPath(),
      platform: process.platform,
      pathEnv: process.env["PATH"],
      pathExt: process.env["PATHEXT"],
      isFile,
    });
  }

  /** Throws [`CliUnavailableError`] when no binary exists anywhere. */
  binary(): string {
    const found = this.find();
    if (!found) {
      throw new CliUnavailableError(
        cliMissingExplanation(process.platform, this.bundledPath()),
        process.platform,
        this.bundledPath(),
      );
    }
    const line = `cli: ${found.binary} (${found.source})`;
    if (this.announced !== line) {
      this.announced = line;
      this.log(line);
    }
    return found.binary;
  }

  /** Packaging can drop the executable bit; restoring it is cheaper than failing. */
  ensureExecutable(): void {
    if (process.platform === "win32") return;
    const bundled = this.bundledPath();
    if (!isFile(bundled)) return;
    try {
      accessSync(bundled, constants.X_OK);
    } catch {
      try {
        chmodSync(bundled, 0o755);
        this.log(`cli: restored the executable bit on ${bundled}`);
      } catch (error) {
        this.log(`cli: ${bundled} is not executable and could not be fixed: ${String(error)}`);
      }
    }
  }

  async warnOnVersionSkew(
    version: (binary: string) => Promise<string | null>,
  ): Promise<void> {
    if (this.versionChecked) return;
    this.versionChecked = true;
    const found = this.find();
    if (!found || found.source === "bundled") return;
    const reported = await version(found.binary);
    if (reported === null || reported === this.extensionVersion) return;
    this.log(
      `cli: ${found.binary} reports ${reported}, the extension is ${this.extensionVersion}`,
    );
    void vscode.window
      .showWarningMessage(
        `The Mándalo CLI at ${found.binary} is version ${reported}, but this extension is ${this.extensionVersion}. Their JSON contracts may differ.`,
        "Use the bundled CLI",
        "Show Output",
      )
      .then((choice) => {
        if (choice === "Use the bundled CLI") {
          return vscode.commands.executeCommand("workbench.action.openSettings", "mandalo.cliPath");
        }
        if (choice === "Show Output") return vscode.commands.executeCommand("mandalo.showLog");
        return undefined;
      });
  }
}

export function parseVersion(stdout: string): string | null {
  const match = /^mandalo\s+(\S+)/.exec(stdout.trim());
  return match?.[1] ?? null;
}
