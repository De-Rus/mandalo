import { describe, expect, it } from "vitest";
import { FolderAccessLapsed, ensureGranted, looksLapsed } from "./mounts";

function handle(state: PermissionState): FileSystemDirectoryHandle {
  return {
    name: "api-collections",
    queryPermission: () => Promise.resolve(state),
  } as unknown as FileSystemDirectoryHandle;
}

describe("a folder whose permission lapsed", () => {
  it("passes straight through while access is still granted", async () => {
    await expect(ensureGranted(handle("granted"), "folder:k")).resolves.toBeUndefined();
  });

  it("fails loudly rather than writing into the void", async () => {
    const failure = ensureGranted(handle("prompt"), "folder:k");

    await expect(failure).rejects.toBeInstanceOf(FolderAccessLapsed);
    await expect(failure).rejects.toThrow(/Reconnect the folder/);
  });

  it("says nothing in the folder was changed, because nothing was", async () => {
    await expect(ensureGranted(handle("denied"), "folder:k")).rejects.toThrow(
      /nothing in it has been changed/,
    );
  });

  it("names the folder the user has to reconnect", async () => {
    await expect(ensureGranted(handle("prompt"), "folder:k")).rejects.toThrow(
      /api-collections/,
    );
  });

  it("is recognisable to the shell so it can offer a re-prompt", async () => {
    const failure = await ensureGranted(handle("prompt"), "folder:k").then(
      () => null,
      (e: Error) => e,
    );

    expect(looksLapsed(failure?.message ?? null)).toBe(true);
    expect(looksLapsed("Something else went wrong")).toBe(false);
    expect(looksLapsed(null)).toBe(false);
  });
});
