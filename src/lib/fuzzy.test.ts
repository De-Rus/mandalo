import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzy matching", () => {
  it("ranks a prefix match above a mid-string match", () => {
    expect(fuzzyScore("List users", "list")).toBeGreaterThan(
      fuzzyScore("Delete list", "list"),
    );
  });

  it("matches scattered characters", () => {
    expect(fuzzyScore("Create user", "cu")).toBeGreaterThan(0);
  });

  it("scores zero when a character is missing", () => {
    expect(fuzzyScore("Create user", "zz")).toBe(0);
  });

  it("filters and orders by score", () => {
    const items = ["Delete user", "User list", "Health"];
    expect(fuzzyFilter(items, "user", (i) => i)).toEqual([
      "User list",
      "Delete user",
    ]);
  });

  it("returns everything for a blank query", () => {
    const items = ["a", "b"];
    expect(fuzzyFilter(items, "  ", (i) => i)).toBe(items);
  });
});
