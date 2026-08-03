import type { MessageShape, ProtoField } from "./api";

function placeholder(field: ProtoField): unknown {
  if (field.repeated) return [];
  switch (field.type) {
    case "message":
      return field.message ? shapeValue(field.message) : {};
    case "enum":
      return field.enumValues[0] ?? "";
    case "bool":
      return false;
    case "number":
      return 0;
    default:
      return "";
  }
}

function shapeValue(shape: MessageShape): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const field of shape.fields) out[field.name] = placeholder(field);
  return out;
}

export function skeletonFor(shape: MessageShape): string {
  return JSON.stringify(shapeValue(shape), null, 2);
}

function normalize(text: string): string | null {
  const trimmed = text.trim();
  if (trimmed === "") return "";
  try {
    return JSON.stringify(JSON.parse(trimmed));
  } catch {
    return null;
  }
}

/**
 * Replacing what the user wrote is never acceptable; replacing an untouched
 * skeleton is what they expect. Anything unparsable counts as written.
 */
export function replaceable(current: string, previous: string | null): boolean {
  const now = normalize(current);
  if (now === "") return true;
  if (now === null || previous === null) return false;
  return now === normalize(previous);
}

export function unknownFields(current: string, shape: MessageShape): string[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(current);
  } catch {
    return [];
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed))
    return [];
  const known = new Set(shape.fields.map((f) => f.name));
  return Object.keys(parsed as Record<string, unknown>).filter(
    (key) => !known.has(key),
  );
}
