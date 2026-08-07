// The single reader of the sandbox facts that live in crates/core/src/script.rs. The
// browser worker and its tests both go through here, so a limit or a blocked global can
// never be transcribed by hand into one of them and drift.

const PRELUDE_OPEN = 'const PRELUDE: &str = r#"';
const BLOCKED_OPEN = "const BLOCKED_GLOBALS: &[(&str, &str)] = &[";

export function readPrelude(rust, where = "crates/core/src/script.rs") {
  const start = rust.indexOf(PRELUDE_OPEN);
  if (start === -1) throw new Error(`PRELUDE not found in ${where}`);
  const from = start + PRELUDE_OPEN.length;
  const end = rust.indexOf('"#;', from);
  if (end === -1) throw new Error(`unterminated PRELUDE raw string in ${where}`);
  return rust.slice(from, end);
}

export function readBlocked(rust, where = "crates/core/src/script.rs") {
  const start = rust.indexOf(BLOCKED_OPEN);
  if (start === -1) throw new Error(`BLOCKED_GLOBALS not found in ${where}`);
  const end = rust.indexOf("];", start);
  const entries = [
    ...rust
      .slice(start, end)
      .matchAll(/"([A-Za-z]+)"\s*,\s*(?:\n\s*)?"((?:[^"\\]|\\.)*)"/g),
  ].map(([, name, why]) => [name, why.replace(/\\"/g, '"')]);
  if (entries.length === 0) throw new Error(`BLOCKED_GLOBALS is empty in ${where}`);
  return entries;
}

export function readLimits(rust, where = "crates/core/src/script.rs") {
  const start = rust.indexOf("impl Default for Limits");
  if (start === -1) throw new Error(`Limits default not found in ${where}`);
  const block = rust.slice(start, rust.indexOf("}\n}", start));
  const memory = /memory_bytes:\s*([0-9*\s]+),/.exec(block);
  const timeout = /timeout_ms:\s*([0-9]+),/.exec(block);
  if (!memory || !timeout) throw new Error(`could not read Limits defaults from ${where}`);
  const memoryBytes = memory[1]
    .split("*")
    .map((part) => Number(part.trim()))
    .reduce((a, b) => a * b, 1);
  return { memoryBytes, timeoutMs: Number(timeout[1]) };
}

// A browser reaches further than QuickJS does, so the seal has to be wider than
// BLOCKED_GLOBALS. It stays limited to names that grant host access or storage: trapping
// a harmless global would only make `typeof` throw where the Rust engine answers
// "undefined". This list is shared with the extension's Node worker on purpose — two
// sandboxes that seal different things are two different languages.
export const EXTRA_SEALED = [
  ["importScripts", "scripts cannot import code"],
  ["Worker", "scripts have no host runtime access"],
  ["SharedWorker", "scripts have no host runtime access"],
  ["EventSource", "scripts cannot make network requests"],
  ["indexedDB", "scripts have no persistent storage"],
  ["caches", "scripts have no persistent storage"],
  ["sessionStorage", "scripts have no persistent storage"],
  ["navigator", "scripts have no host runtime access"],
  ["self", "scripts do not run in a browser"],
  ["postMessage", "scripts cannot make network requests"],
  ["parentPort", "scripts have no host runtime access"],
  ["MessageChannel", "scripts have no host runtime access"],
  ["MessagePort", "scripts have no host runtime access"],
  ["BroadcastChannel", "scripts cannot make network requests"],
  ["WebAssembly", "scripts cannot import code"],
];
