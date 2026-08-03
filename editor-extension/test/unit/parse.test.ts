import { describe, expect, it } from "vitest";
import {
  parseCollectionManifest,
  parseEnvironment,
  parseWorkspaceManifest,
} from "../../src/core/parse";

describe("manifests and environments", () => {
  it("reads a collection manifest", () => {
    expect(parseCollectionManifest('schema_version = 1\nid = "x"\nname = "Acme"')).toEqual({
      schemaVersion: 1,
      id: "x",
      name: "Acme",
    });
  });

  it("reads a workspace manifest", () => {
    expect(parseWorkspaceManifest('schema_version = 1\nid = "w"\nname = "Personal"')).toEqual({
      schemaVersion: 1,
      id: "w",
      name: "Personal",
    });
  });

  it("reads an environment and falls back to the file name", () => {
    expect(parseEnvironment('[vars]\nbase = "https://x"', "staging")).toEqual({
      name: "staging",
      vars: { base: "https://x" },
    });
    expect(parseEnvironment('name = "prod"\n[vars]\nbase = "https://x"', "ignored").name).toBe("prod");
  });

  it("fails loud on a non-string variable", () => {
    expect(() => parseEnvironment("[vars]\nport = 8080", "local")).toThrow(/must be a string/);
  });
});
