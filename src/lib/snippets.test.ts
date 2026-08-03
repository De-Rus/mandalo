import { describe, expect, it } from "vitest";
import { insertAt, PRE_SNIPPETS, TEST_SNIPPETS } from "./snippets";

describe("script snippets", () => {
  it("inserts at the cursor and returns the new cursor position", () => {
    const { text, cursor } = insertAt("ab", 1, "X");
    expect(text).toBe("a\nXb");
    expect(text[cursor]).toBe("b");
  });

  it("does not add a leading newline at the start of a line", () => {
    expect(insertAt("", 0, "X").text).toBe("X");
    expect(insertAt("a\n", 2, "X").text).toBe("a\nX");
  });

  it("ships Postman-style starters for both script slots", () => {
    expect(PRE_SNIPPETS.map((s) => s.label)).toContain(
      "Get an environment variable",
    );
    expect(TEST_SNIPPETS.map((s) => s.label)).toContain(
      "Status code: Code is 200",
    );
    expect(
      TEST_SNIPPETS.every((s) => s.code.trim() !== ""),
    ).toBe(true);
  });
});
