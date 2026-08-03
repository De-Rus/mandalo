import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

const at = (path: string) => fileURLToPath(new URL(path, import.meta.url));

// The browser build is the same app with the Tauri seam swapped for a
// client-side one; nothing under src/components or src/store knows the
// difference.
const web = {
  root: at("web"),
  base: "/app/",
  publicDir: at("src/lib/web/public"),
  resolve: {
    alias: {
      "@tauri-apps/api/core": at("src/lib/web/invoke.ts"),
      "@tauri-apps/plugin-dialog": at("src/lib/web/dialog.ts"),
    },
  },
  build: {
    outDir: at("landing/app"),
    emptyOutDir: true,
    target: "es2022",
  },
};

// The VS Code build is the same app again, this time with the seam pointed at the
// extension host over postMessage. Fixed asset names keep the CSP-locked HTML the
// extension generates simple, and inlining every asset keeps it to two files.
const vscode = {
  root: at("src/lib/vscode/host"),
  base: "./",
  resolve: {
    alias: {
      "@tauri-apps/api/core": at("src/lib/vscode/invoke.ts"),
      "@tauri-apps/plugin-dialog": at("src/lib/vscode/dialog.ts"),
    },
  },
  build: {
    outDir: at("editor-extension/media/editor"),
    emptyOutDir: true,
    target: "es2022",
    sourcemap: false,
    assetsInlineLimit: 1024 * 1024,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
        entryFileNames: "editor.js",
        assetFileNames: "editor.[ext]",
      },
    },
  },
};

export default defineConfig(({ mode }) => ({
  plugins: [react()],
  clearScreen: false,
  ...(mode === "web" ? web : {}),
  ...(mode === "vscode" ? vscode : {}),
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1431,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
}));
