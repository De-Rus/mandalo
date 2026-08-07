import {
  importBundle,
  importOpenapi,
  importPostman,
  type ImportReport,
} from "./api";
import type { ImportKind } from "./importKind";

const IMPORTERS: Record<
  ImportKind,
  (workspace: string, source: string) => Promise<ImportReport>
> = {
  bundle: importBundle,
  openapi: importOpenapi,
  postman: importPostman,
};

export function importAs(
  workspace: string,
  kind: ImportKind,
  source: string,
): Promise<ImportReport> {
  return IMPORTERS[kind](workspace, source);
}
