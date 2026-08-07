/**
 * The workspace every browser session starts with is `examples/mock-workspace`
 * itself — the same files the desktop app and the CLI use as a fixture, inlined
 * at build time. There is no second copy of it to drift.
 */
const FILES = import.meta.glob(
  "../../../examples/mock-workspace/**/*.{toml,http,rest,grpc,ws,mqtt,proto,json,txt}",
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
) as Record<string, string>;

const PREFIX = "examples/mock-workspace/";

export function seedFiles(): [string, string][] {
  const out: [string, string][] = [];
  for (const [key, text] of Object.entries(FILES)) {
    const at = key.indexOf(PREFIX);
    if (at === -1) continue;
    out.push([key.slice(at + PREFIX.length), text]);
  }
  if (out.length === 0)
    throw new Error(
      "examples/mock-workspace is missing: the browser build has no sample workspace to seed",
    );
  return out.sort(([a], [b]) => a.localeCompare(b));
}
