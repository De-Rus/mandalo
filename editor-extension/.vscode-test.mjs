import { defineConfig } from "@vscode/test-cli";

export default defineConfig({
  label: "integration",
  files: "out/integration/**/*.test.js",
  workspaceFolder: "./fixtures/workspace",
  version: "stable",
  mocha: {
    ui: "tdd",
    timeout: 60000,
  },
});
