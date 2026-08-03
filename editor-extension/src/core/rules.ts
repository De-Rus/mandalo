import { spanForEntryKey, spanForKey, WHOLE_FIRST_LINE } from "./locator";
import type { Span } from "./locator";
import { ParseError, parseRequest } from "./parse";
import { collectVarReferences, validateCapture, validateKind, validateTest } from "./validate";

export type Severity = "error" | "warning";

export interface Finding extends Span {
  message: string;
  severity: Severity;
  code: string;
  variable?: string;
}

export interface LintContext {
  envName?: string | undefined;
  envVars?: Record<string, string> | undefined;
}

export function lintRequestDocument(raw: string, context: LintContext = {}): Finding[] {
  let model;
  try {
    model = parseRequest(raw);
  } catch (error) {
    const message =
      error instanceof ParseError ? error.message : `could not read request: ${(error as Error).message}`;
    return [{ ...WHOLE_FIRST_LINE, message, severity: "error", code: "mandalo.parse" }];
  }

  const findings: Finding[] = [];
  const kindProblem = validateKind(model.kind);
  if (kindProblem) {
    findings.push({
      ...spanForKey(raw, "kind"),
      message: kindProblem,
      severity: "error",
      code: "mandalo.kind",
    });
  }

  model.tests.forEach((test, index) => {
    for (const problem of validateTest(test)) {
      findings.push({
        ...spanForEntryKey(raw, "[[tests]]", index, "op"),
        message: problem,
        severity: "error",
        code: "mandalo.test",
      });
    }
  });

  model.captures.forEach((capture, index) => {
    for (const problem of validateCapture(capture)) {
      const key = problem.startsWith("invalid capture target")
        ? "into"
        : problem.startsWith("unknown capture scope")
          ? "scope"
          : "from";
      findings.push({
        ...spanForEntryKey(raw, "[[captures]]", index, key),
        message: problem,
        severity: "error",
        code: "mandalo.capture",
      });
    }
  });

  if (context.envVars) {
    for (const reference of collectVarReferences(raw)) {
      if (reference.name in context.envVars) continue;
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
