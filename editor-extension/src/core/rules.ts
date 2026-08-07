import { parseTextDocument, resolveVars, TextFormatError, type TextFileKind } from "../../../src/lib/format/httpFormat";
import { lintScripts } from "./scripts";
import { collectVarReferences } from "./validate";

export type Severity = "error" | "warning";

export interface Finding {
  offset: number;
  length: number;
  message: string;
  severity: Severity;
  code: string;
  variable?: string;
}

export interface LintContext {
  envName?: string | undefined;
  envVars?: Record<string, string> | undefined;
}

const WHOLE_FIRST_LINE: Finding = {
  offset: 0,
  length: 1,
  message: "",
  severity: "error",
  code: "mandalo.parse",
};

export function lintRequestDocument(
  raw: string,
  fileKind: TextFileKind,
  context: LintContext = {},
): Finding[] {
  const findings: Finding[] = [];
  let fileVars = new Map<string, string>();
  try {
    fileVars = resolveVars(parseTextDocument("lint", raw, fileKind).vars);
  } catch (error) {
    if (error instanceof TextFormatError) {
      findings.push({
        offset: error.offset,
        length: error.length,
        message: error.message,
        severity: "error",
        code: "mandalo.parse",
      });
    } else {
      findings.push({ ...WHOLE_FIRST_LINE, message: (error as Error).message });
    }
    return findings;
  }

  findings.push(...lintScripts(raw));

  if (context.envVars) {
    for (const reference of collectVarReferences(raw)) {
      if (fileVars.has(reference.name) || reference.name in context.envVars) continue;
      const where = context.envName ? `environment "${context.envName}"` : "the selected environment";
      findings.push({
        offset: reference.offset,
        length: reference.length,
        message: `"${reference.name}" is not defined in ${where}`,
        severity: "warning",
        code: "mandalo.unresolvedVar",
        variable: reference.name,
      });
    }
  }
  return findings;
}
