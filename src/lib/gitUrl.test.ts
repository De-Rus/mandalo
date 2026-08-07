import { describe, expect, it } from "vitest";
import { repoFolderName } from "./gitUrl";

describe("repoFolderName", () => {
  it("takes the last path segment and strips .git", () => {
    expect(repoFolderName("https://github.com/acme/apis.git")).toBe("apis");
    expect(repoFolderName("https://github.com/acme/apis/")).toBe("apis");
    expect(repoFolderName("git@github.com:acme/apis.git")).toBe("apis");
  });

  it("falls back when the URL is empty noise", () => {
    expect(repoFolderName("")).toBe("workspace");
    expect(repoFolderName("///")).toBe("workspace");
  });
});
