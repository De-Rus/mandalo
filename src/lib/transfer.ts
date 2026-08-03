import { importBundle, importPostman, type ImportReport } from "./api";

export function isBundle(json: string): boolean {
  const parsed: unknown = JSON.parse(json);
  return (
    typeof parsed === "object" && parsed !== null && "mandaloBundle" in parsed
  );
}

export function importFromText(
  workspace: string,
  json: string,
): Promise<ImportReport> {
  return isBundle(json)
    ? importBundle(workspace, json)
    : importPostman(workspace, json);
}
