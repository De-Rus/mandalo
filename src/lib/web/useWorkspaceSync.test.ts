import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../../store/collection";
import { useEnv } from "../../store/env";
import { applyChange } from "./useWorkspaceSync";

const invoke = vi.hoisted(() => vi.fn(() => Promise.resolve(undefined)));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const WORKSPACE = "browser://Browser storage";

let applyRemoteRequest: (collection: string, path: string) => Promise<void>;
let applyRemoteTree: () => Promise<void>;
let refresh: () => Promise<void>;
let scheduleTree: () => void;

beforeEach(() => {
  applyRemoteRequest = vi.fn((_c: string, _p: string) => Promise.resolve());
  applyRemoteTree = vi.fn(() => Promise.resolve());
  refresh = vi.fn(() => Promise.resolve());
  scheduleTree = vi.fn();
  useCollection.setState({ workspace: WORKSPACE, applyRemoteRequest, applyRemoteTree });
  useEnv.setState({ refresh });
});

describe("routing a change from another tab", () => {
  it("reloads just the file that changed", () => {
    applyChange(
      { workspace: WORKSPACE, scope: "request", collection: "mock", path: "a.http" },
      scheduleTree,
    );

    expect(applyRemoteRequest).toHaveBeenCalledWith("mock", "a.http");
    expect(applyRemoteTree).not.toHaveBeenCalled();
  });

  it("refreshes the sidebar on a trailing timer, not per keystroke", () => {
    applyChange(
      { workspace: WORKSPACE, scope: "request", collection: "mock", path: "a.http" },
      scheduleTree,
    );

    expect(scheduleTree).toHaveBeenCalledTimes(1);
  });

  it("refreshes the tree at once when a request appeared or vanished", () => {
    applyChange({ workspace: WORKSPACE, scope: "tree" }, scheduleTree);

    expect(applyRemoteTree).toHaveBeenCalled();
    expect(scheduleTree).not.toHaveBeenCalled();
  });

  it("reloads environments when they changed", () => {
    applyChange({ workspace: WORKSPACE, scope: "environments" }, scheduleTree);

    expect(refresh).toHaveBeenCalled();
  });

  it("ignores a change to a workspace this tab is not showing", () => {
    applyChange(
      { workspace: "folder://other", scope: "request", collection: "m", path: "a" },
      scheduleTree,
    );

    expect(applyRemoteRequest).not.toHaveBeenCalled();
    expect(applyRemoteTree).not.toHaveBeenCalled();
  });

  it("ignores anything that arrives before this tab has a workspace", () => {
    useCollection.setState({ workspace: null });

    applyChange({ workspace: WORKSPACE, scope: "tree" }, scheduleTree);

    expect(applyRemoteTree).not.toHaveBeenCalled();
  });
});
