import type { RequestSpec } from "../api";
import { webExecuteScript } from "./script";

const REFERENCE = /\{\{\s*([^}\s]+)\s*\}\}/g;

function namesIn(template: string | null | undefined, out: Set<string>): void {
  if (!template) return;
  for (const match of template.matchAll(REFERENCE))
    if (match[1].startsWith("$")) out.add(match[1]);
}

export function dynamicNames(spec: RequestSpec): string[] {
  const names = new Set<string>();
  namesIn(spec.method, names);
  namesIn(spec.url, names);
  for (const [key, value] of spec.headers) {
    namesIn(key, names);
    namesIn(value, names);
  }
  namesIn(spec.body, names);
  namesIn(spec.graphql?.query, names);
  namesIn(spec.graphql?.variables, names);
  const auth = spec.auth;
  if (auth.type === "bearer") namesIn(auth.token, names);
  if (auth.type === "basic") {
    namesIn(auth.username, names);
    namesIn(auth.password, names);
  }
  if (auth.type === "apikey") {
    namesIn(auth.key, names);
    namesIn(auth.value, names);
  }
  return [...names].filter((name) => !(name in spec.vars)).sort();
}

/**
 * Postman's `{{$guid}}`-style values have exactly one generator in this
 * repository: the DYNAMIC map inside the Rust prelude, which the browser
 * already runs byte-identical inside the script worker. Reaching it through
 * `pm.variables.replaceIn` keeps the browser from growing a second, drifting
 * copy of the table.
 */
export async function resolveDynamic(
  names: readonly string[],
): Promise<Record<string, string>> {
  if (names.length === 0) return {};
  const source = `
    var names = ${JSON.stringify(names)};
    for (var i = 0; i < names.length; i++) {
      pm.variables.set(names[i], pm.variables.replaceIn("{{" + names[i] + "}}"));
    }
  `;
  const outcome = await webExecuteScript(source, {
    vars: {},
    requestName: "",
    request: { method: "GET", url: "", headers: [], body: null },
    response: null,
  });
  return outcome.varSets;
}
