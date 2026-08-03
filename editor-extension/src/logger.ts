import * as vscode from "vscode";
import { CliMissingError, CliOutputError, CliUnavailableError } from "./core/cli";

const RELEASES_URL = "https://github.com/De-Rus/mandalo/releases/latest";

export class Logger {
  private readonly channel = vscode.window.createOutputChannel("Mándalo");

  line(text: string): void {
    this.channel.appendLine(text);
  }

  show(): void {
    this.channel.show(true);
  }

  // The dialog is never awaited: a blocking notification would keep the command's
  // progress indicator spinning until the user clicks something.
  report(error: unknown): void {
    if (error instanceof CliUnavailableError || error instanceof CliMissingError) {
      this.line(error.message);
      this.offerCli(error.message);
      return;
    }
    if (error instanceof CliOutputError) {
      this.line(error.message);
      this.line("--- raw CLI output ---");
      this.line(error.raw);
      this.line("----------------------");
      void vscode.window.showErrorMessage(error.message, "Show Output").then((choice) => {
        if (choice === "Show Output") this.show();
      });
      return;
    }
    const message = error instanceof Error ? error.message : String(error);
    this.line(message);
    void vscode.window.showErrorMessage(`Mándalo: ${message}`, "Show Output").then((choice) => {
      if (choice === "Show Output") this.show();
    });
  }

  private offerCli(message: string): void {
    void vscode.window
      .showErrorMessage(message, "Download from releases", "Set path…", "Open output")
      .then((choice) => {
        if (choice === "Download from releases") {
          return vscode.env.openExternal(vscode.Uri.parse(RELEASES_URL));
        }
        if (choice === "Set path…") {
          return vscode.commands.executeCommand("workbench.action.openSettings", "mandalo.cliPath");
        }
        if (choice === "Open output") this.show();
        return undefined;
      });
  }

  dispose(): void {
    this.channel.dispose();
  }
}
