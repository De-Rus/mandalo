import { describe, expect, it } from "vitest";
import {
  buildRequestMerge,
  matchRequestUnits,
  splitRequestUnits,
  type UnitPick,
} from "./conflictUnits";

describe("conflictUnits", () => {
  it("splits ### requests and matches by name", () => {
    const ours = `### Login

POST {{baseUrl}}/login

### Me

GET {{baseUrl}}/me
`;
    const theirs = `### Login

POST {{baseUrl}}/auth/login

### Profile

GET {{baseUrl}}/profile
`;
    const units = matchRequestUnits(ours, theirs);
    expect(units.map((u) => [u.kind, u.name])).toEqual([
      ["changed", "Login"],
      ["oursOnly", "Me"],
      ["theirsOnly", "Profile"],
    ]);
  });

  it("builds a merge with yours, theirs, or both", () => {
    const ours = `### A

GET /a

### B

GET /b
`;
    const theirs = `### A

GET /a2

### C

GET /c
`;
    const units = matchRequestUnits(ours, theirs);
    const picks: Record<string, UnitPick> = Object.fromEntries(
      units.map((u) => {
        if (u.kind === "changed") return [u.id, "both" as const];
        if (u.kind === "oursOnly") return [u.id, "ours" as const];
        return [u.id, "theirs" as const];
      }),
    );
    const merged = buildRequestMerge(units, picks);
    expect(merged).toContain("### A\n\nGET /a");
    expect(merged).toContain("### A (remote)");
    expect(merged).toContain("GET /a2");
    expect(merged).toContain("### B");
    expect(merged).toContain("### C");
    expect(splitRequestUnits(merged)).toHaveLength(4);
  });
});
