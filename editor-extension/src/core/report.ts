import type { TestOutcome } from "./cli";

export interface AssertionChild {
  id: string;
  label: string;
  test: TestOutcome;
}

// Child items are keyed by the CLI's stable `test.id`; the label stays the human-readable
// name, which is NOT unique — two assertions may share one name.
export function assertionChildren(itemId: string, tests: TestOutcome[]): AssertionChild[] {
  return tests.map((test) => ({ id: `${itemId}::${test.id}`, label: test.name, test }));
}

export function assertionIndex(itemId: string, tests: TestOutcome[]): Map<string, TestOutcome> {
  return new Map(assertionChildren(itemId, tests).map((child) => [child.id, child.test]));
}

export function failureSummary(tests: TestOutcome[]): string {
  return tests
    .filter((test) => !test.passed)
    .map((test) => `${test.name}${test.detail ? ` — ${test.detail}` : ""}`)
    .join("\n");
}
