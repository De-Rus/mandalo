import { describe, expect, it } from "vitest";
import {
  bodyText,
  formatBytes,
  formatDuration,
  prettyJson,
  statusTone,
} from "./format";

describe("formatBytes", () => {
  it("formats bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("formats kilobytes", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(1024 * 1024 - 1)).toBe("1024.0 KB");
  });

  it("formats megabytes", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.00 MB");
    expect(formatBytes(2.5 * 1024 * 1024)).toBe("2.50 MB");
  });
});

describe("formatDuration", () => {
  it("shows ms under a second", () => {
    expect(formatDuration(42)).toBe("42 ms");
  });

  it("shows seconds above a second", () => {
    expect(formatDuration(1500)).toBe("1.50 s");
  });
});

describe("statusTone", () => {
  it("maps status classes to tones", () => {
    expect(statusTone(200)).toBe("success");
    expect(statusTone(204)).toBe("success");
    expect(statusTone(301)).toBe("info");
    expect(statusTone(404)).toBe("warn");
    expect(statusTone(500)).toBe("error");
    expect(statusTone(0)).toBe("muted");
  });
});

describe("prettyJson", () => {
  it("pretty prints valid JSON", () => {
    expect(prettyJson('{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(prettyJson("[1,2]")).toBe("[\n  1,\n  2\n]");
  });

  it("returns null for invalid JSON", () => {
    expect(prettyJson("{a:1}")).toBeNull();
    expect(prettyJson("")).toBeNull();
  });

  it("returns null for non-JSON text", () => {
    expect(prettyJson("hello world")).toBeNull();
  });
});

describe("bodyText", () => {
  it("pretty prints JSON when the body is text", () => {
    expect(bodyText('{"a":1}', false)).toBe('{\n  "a": 1\n}');
  });

  it("leaves a binary body untouched even when it parses as JSON", () => {
    expect(bodyText('{"a":1}', true)).toBe('{"a":1}');
  });

  it("falls back to the raw body when JSON parsing fails", () => {
    expect(bodyText("hello world", false)).toBe("hello world");
  });
});
