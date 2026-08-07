import type { TestResult } from "./api";

export interface TestTally {
  passed: number;
  total: number;
  ok: boolean;
}

/**
 * Declarative assertions and `pm.test` results counted together — they are one
 * verdict to the reader. `null` means the request carried no tests at all, which
 * is not the same claim as "everything passed"; neither is a run that died
 * before its tests could finish, so an error is a failure even at 0 of 0.
 */
export function tallyTests(
  tests: TestResult[],
  scriptTests: TestResult[],
  runError: string | null,
): TestTally | null {
  const all = [...tests, ...scriptTests];
  if (all.length === 0 && runError === null) return null;
  const passed = all.filter((t) => t.passed).length;
  return {
    passed,
    total: all.length,
    ok: runError === null && passed === all.length,
  };
}
