import { beforeEach, describe, expect, it } from "vitest";
import { candidates, collect, store } from "./protos";
import { MemoryVfs } from "./vfs.testkit";

const MOCK = 'syntax = "proto3";\npackage mock.v1;\n';
const COMMON = 'syntax = "proto3";\npackage mock.v1;\n';

let vfs: MemoryVfs;

beforeEach(() => {
  vfs = new MemoryVfs();
});

describe("candidates", () => {
  it("drops the leading slash of an absolute path and falls back to protos/", () => {
    expect(candidates("/tmp/mandalo-mock/mock.proto")).toEqual([
      "tmp/mandalo-mock/mock.proto",
      "protos/mock.proto",
      "mock.proto",
    ]);
  });

  it("keeps a workspace-relative path first", () => {
    expect(candidates("protos/mock.proto")).toEqual([
      "protos/mock.proto",
      "mock.proto",
    ]);
  });

  it("normalises windows separators", () => {
    expect(candidates("C:\\protos\\mock.proto")[0]).toBe("C:/protos/mock.proto");
  });
});

describe("collect", () => {
  it("resolves an absolute desktop path to the workspace copy", async () => {
    await vfs.write("protos/mock.proto", MOCK);
    expect(await collect(vfs, ["/tmp/mandalo-mock/mock.proto"])).toEqual([
      { path: "protos/mock.proto", contents: MOCK },
    ]);
  });

  it("carries every other proto along so imports can resolve", async () => {
    await vfs.write("protos/mock.proto", MOCK);
    await vfs.write("protos/common.proto", COMMON);
    const files = await collect(vfs, ["mock.proto"]);
    expect(files.map((f) => f.path).sort()).toEqual([
      "protos/common.proto",
      "protos/mock.proto",
    ]);
  });

  it("reads nested proto folders", async () => {
    await vfs.write("protos/mock.proto", MOCK);
    await vfs.write("protos/sub/common.proto", COMMON);
    const files = await collect(vfs, ["mock.proto"]);
    expect(files.map((f) => f.path)).toContain("protos/sub/common.proto");
  });

  it("does not list the same file twice", async () => {
    await vfs.write("protos/mock.proto", MOCK);
    const files = await collect(vfs, ["mock.proto", "protos/mock.proto"]);
    expect(files).toHaveLength(1);
  });

  it("fails loud with every path it tried", async () => {
    await expect(collect(vfs, ["/opt/svc/api.proto"])).rejects.toThrow(
      /"opt\/svc\/api.proto", "protos\/api.proto", "api.proto"/,
    );
  });

  it("fails loud on an empty path list", async () => {
    await expect(collect(vfs, [])).rejects.toThrow("no proto files given");
  });
});

describe("store", () => {
  it("writes an attached file into the workspace proto folder", async () => {
    expect(await store(vfs, "mock.proto", MOCK)).toBe("protos/mock.proto");
    expect(await vfs.read("protos/mock.proto")).toBe(MOCK);
  });

  it("keeps only the file name of a path the browser hands over", async () => {
    expect(await store(vfs, "some/where/mock.proto", MOCK)).toBe(
      "protos/mock.proto",
    );
  });

  it("refuses anything that is not a .proto", async () => {
    await expect(store(vfs, "mock.txt", MOCK)).rejects.toThrow(
      '"mock.txt" is not a .proto file',
    );
  });
});
