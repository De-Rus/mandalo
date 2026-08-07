export type ImportKind = "bundle" | "openapi" | "postman";

export interface Detection {
  kind: ImportKind;
  confident: boolean;
  reason: string;
}

export const IMPORT_LABELS: Record<ImportKind, string> = {
  bundle: "Mándalo bundle",
  openapi: "OpenAPI / Swagger",
  postman: "Postman collection",
};

export const IMPORT_KINDS: ImportKind[] = ["bundle", "openapi", "postman"];

/**
 * A key the document *declares*, not the word appearing in a description. Both
 * the Rust `openapi::looks_like_openapi` and the CLI's router work this way, and
 * the three of them have to agree or the GUI imports a file as something the CLI
 * would read differently.
 */
function declaresJsonKey(source: string, key: string): boolean {
  return source
    .split(`"${key}"`)
    .slice(1)
    .some((rest) => rest.trimStart().startsWith(":"));
}

function declaresYamlKey(source: string, key: string): boolean {
  return source.split("\n").some((line) => line.startsWith(`${key}:`));
}

function looksLikeOpenapi(source: string): boolean {
  return (
    declaresJsonKey(source, "openapi") ||
    declaresJsonKey(source, "swagger") ||
    declaresYamlKey(source, "openapi") ||
    declaresYamlKey(source, "swagger")
  );
}

function looksLikePostman(source: string): boolean {
  if (declaresJsonKey(source, "_postman_id")) return true;
  return declaresJsonKey(source, "info") && declaresJsonKey(source, "item");
}

export function detectImportKind(source: string): Detection {
  if (declaresJsonKey(source, "mandaloBundle"))
    return {
      kind: "bundle",
      confident: true,
      reason: "This document declares mandaloBundle, so it is a Mándalo bundle.",
    };
  if (looksLikeOpenapi(source))
    return {
      kind: "openapi",
      confident: true,
      reason:
        "This document declares an openapi or swagger version, so it is an API specification.",
    };
  if (looksLikePostman(source))
    return {
      kind: "postman",
      confident: true,
      reason:
        "This document has a Postman info block and an item list, so it is a Postman collection.",
    };
  return {
    kind: "postman",
    confident: false,
    reason:
      "Nothing in this document names its format. Mándalo will read it as a Postman collection — pick another importer if that is wrong.",
  };
}
