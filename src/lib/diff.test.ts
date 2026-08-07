import { describe, expect, it } from "vitest";
import { lineDiff } from "./diff";

describe("lineDiff", () => {
  it("lists every inserted and deleted line", () => {
    const lines = lineDiff("a\nb\nc\n", "a\nB\nc\n");
    expect(lines.filter((l) => l.op === "delete").map((l) => l.text)).toEqual([
      "b",
    ]);
    expect(lines.filter((l) => l.op === "insert").map((l) => l.text)).toEqual([
      "B",
    ]);
    expect(lines.filter((l) => l.op === "equal").map((l) => l.text)).toEqual([
      "a",
      "c",
    ]);
  });
});
