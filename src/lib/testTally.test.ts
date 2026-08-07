import { describe, expect, it } from "vitest";
import { tallyTests } from "./testTally";

const pass = (name: string) => ({ name, passed: true, detail: null });
const fail = (name: string) => ({ name, passed: false, detail: "no" });

describe("tallyTests", () => {
  it("has nothing to say when the request carried no tests", () => {
    expect(tallyTests([], [], null)).toBeNull();
  });

  it("counts declarative assertions and pm.test together", () => {
    expect(tallyTests([pass("status"), fail("json")], [pass("script")], null)).toEqual({
      passed: 2,
      total: 3,
      ok: false,
    });
  });

  it("is ok only when every test passed", () => {
    expect(tallyTests([pass("a")], [pass("b")], null)).toEqual({
      passed: 2,
      total: 2,
      ok: true,
    });
  });

  it("a run that died before its tests is a failure, not a silent pass", () => {
    expect(tallyTests([], [], "ReferenceError: pm is not defined")).toEqual({
      passed: 0,
      total: 0,
      ok: false,
    });
  });

  it("a run that died after some tests passed is still a failure", () => {
    expect(tallyTests([], [pass("a")], "TypeError: x is undefined")).toEqual({
      passed: 1,
      total: 1,
      ok: false,
    });
  });
});
