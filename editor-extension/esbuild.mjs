import { context } from "esbuild";
import { glob } from "node:fs/promises";

const watch = process.argv.includes("--watch");
const withTests = process.argv.includes("--tests");
const production = process.argv.includes("--production") || process.env["NODE_ENV"] === "production";

const shared = {
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node18",
  external: ["vscode"],
  sourcemap: !production,
  logLevel: "info",
};

const builds = [
  await context({ ...shared, entryPoints: ["src/extension.ts"], outfile: "dist/extension.js", minify: production }),
];

if (withTests) {
  const entryPoints = [];
  for await (const file of glob("test/integration/**/*.ts")) entryPoints.push(file);
  builds.push(
    await context({
      ...shared,
      entryPoints,
      outdir: "out/integration",
      external: ["vscode", "mocha"],
      sourcemap: true,
    }),
  );
}

for (const build of builds) {
  if (watch) {
    await build.watch();
  } else {
    await build.rebuild();
    await build.dispose();
  }
}
