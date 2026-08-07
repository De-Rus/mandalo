import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useGit } from "../store/git";
import { useToasts } from "../store/toast";
import { SyncDialog } from "./SyncDialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const FINDING = {
  path: "environments/prod.toml",
  line: 4,
  rule: "aws-access-key-id",
  excerpt: "key = AKIA…",
};

function plan(over: Record<string, unknown> = {}) {
  return {
    action: "commitAndPush",
    files: [
      { path: "collections/a/req.http", change: "modified", included: true },
      { path: "environments/staging.toml", change: "new", included: true },
    ],
    included: 2,
    excluded: 0,
    remote: "https://github.com/acme/apis.git",
    branch: "main",
    targetBranch: null,
    ahead: 0,
    behind: 0,
    conflicted: [],
    findings: [],
    blocked: false,
    identity: { name: "Ada", email: "ada@ex.com", isFallback: false },
    token: "plan-sync-1",
    ...over,
  };
}

function mockApi(extra: (cmd: string, args?: unknown) => unknown = () => undefined) {
  invoke.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === "workspace_share") return Promise.resolve(null);
    if (cmd === "set_workspace_share") return Promise.resolve({});
    if (cmd === "git_status") return Promise.resolve(null);
    const hit = extra(cmd, args);
    if (hit !== undefined) return hit;
    return Promise.resolve(undefined);
  });
}

describe("SyncDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    useCollection.setState({ workspace: "/ws" });
    useGit.setState({ status: null, error: null, busy: false });
    useToasts.setState({ items: [] });
  });

  afterEach(cleanup);

  it("plans on open and runs only after a message is set", async () => {
    mockApi((cmd) => {
      if (cmd === "plan_sync") return Promise.resolve(plan());
      if (cmd === "run_sync")
        return Promise.resolve({
          kind: "pushed",
          sha: "abcdef0123456789",
          ahead: 1,
          identity: { name: "Ada", email: "ada@ex.com", isFallback: false },
        });
    });
    const onClose = vi.fn();
    render(<SyncDialog onClose={onClose} />);

    await waitFor(() =>
      expect(screen.getByText(/Commit and push/)).toBeTruthy(),
    );
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("run_sync");

    const syncBtn = screen.getByRole("button", { name: "Sync" }) as HTMLButtonElement;
    expect(syncBtn.disabled).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("What changed?"), {
      target: { value: "tweak auth" },
    });
    expect(syncBtn.disabled).toBe(false);

    fireEvent.click(syncBtn);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("run_sync", {
        workspace: "/ws",
        selection: null,
        token: "plan-sync-1",
        message: "tweak auth",
        gitToken: null,
        force: false,
      }),
    );
    expect(useToasts.getState().items.map((t) => t.text)).toContain(
      "Pushed abcdef0",
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("blocks on scanner findings until they are acknowledged", async () => {
    mockApi((cmd) => {
      if (cmd === "plan_sync")
        return Promise.resolve(plan({ findings: [FINDING], blocked: true }));
      if (cmd === "run_sync")
        return Promise.resolve({
          kind: "pushed",
          sha: "deadbeef",
          ahead: 1,
          identity: { name: "Ada", email: "ada@ex.com", isFallback: false },
        });
    });
    render(<SyncDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText("aws-access-key-id")).toBeTruthy());
    fireEvent.change(screen.getByPlaceholderText("What changed?"), {
      target: { value: "oops" },
    });
    expect(
      (screen.getByRole("button", { name: "Sync" }) as HTMLButtonElement).disabled,
    ).toBe(true);

    fireEvent.click(
      screen.getByText(/I understand these values will leave this machine/),
    );
    fireEvent.click(screen.getByRole("button", { name: "Sync" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_sync",
        expect.objectContaining({ force: true }),
      ),
    );
  });

  it("leaves unchecked files out of the commit via except", async () => {
    mockApi((cmd, args) => {
      if (cmd === "plan_sync") {
        const except =
          ((args as { selection?: { except?: string[] } } | undefined)?.selection
            ?.except) ?? [];
        return Promise.resolve(
          plan({
            excluded: except.length,
            included: 2 - except.length,
            files: [
              {
                path: "collections/a/req.http",
                change: "modified",
                included: !except.includes("collections/a/req.http"),
              },
              {
                path: "environments/staging.toml",
                change: "new",
                included: !except.includes("environments/staging.toml"),
              },
            ],
          }),
        );
      }
      if (cmd === "run_sync")
        return Promise.resolve({
          kind: "committed",
          sha: "abc",
          identity: { name: "Ada", email: "ada@ex.com", isFallback: false },
        });
    });
    render(<SyncDialog onClose={() => {}} />);

    await waitFor(() =>
      expect(screen.getByText("collections/a/req.http")).toBeTruthy(),
    );
    const boxes = screen.getAllByRole("checkbox");
    fireEvent.click(boxes[0]);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "plan_sync",
        expect.objectContaining({
          selection: { except: ["collections/a/req.http"] },
        }),
      ),
    );

    fireEvent.change(screen.getByPlaceholderText("What changed?"), {
      target: { value: "only env" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sync" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_sync",
        expect.objectContaining({
          selection: { except: ["collections/a/req.http"] },
          message: "only env",
        }),
      ),
    );
  });

  it("persists Postman mirror to mandalo.toml and re-plans", async () => {
    mockApi((cmd) => {
      if (cmd === "plan_sync")
        return Promise.resolve(
          plan({
            shareDir: cmd === "plan_sync" ? "postman" : undefined,
            files: [
              ...plan().files,
              { path: "postman/api.json", change: "new", included: true },
            ],
            included: 3,
          }),
        );
    });
    // First plan without shareDir until Postman is chosen.
    let postmanOn = false;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share")
        return Promise.resolve(postmanOn ? { format: "postman" } : null);
      if (cmd === "set_workspace_share") {
        postmanOn = true;
        return Promise.resolve({});
      }
      if (cmd === "plan_sync")
        return Promise.resolve(
          plan({
            shareDir: postmanOn ? "postman" : null,
            files: postmanOn
              ? [
                  ...plan().files,
                  { path: "postman/api.json", change: "new", included: true },
                ]
              : plan().files,
            included: postmanOn ? 3 : 2,
          }),
        );
      if (cmd === "git_status") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    render(<SyncDialog onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText(/Commit and push/)).toBeTruthy());

    await waitFor(() => expect(screen.getByText(/^Postman$/)).toBeTruthy());
    fireEvent.click(screen.getByText(/^Postman$/));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("set_workspace_share", {
        workspace: "/ws",
        format: "postman",
        dir: null,
      }),
    );
    await waitFor(() =>
      expect(screen.getByText(/v2\.1 mirror under/)).toBeTruthy(),
    );
  });

  it("opens visual resolve when sync returns conflicted", async () => {
    mockApi((cmd) => {
      if (cmd === "plan_sync") return Promise.resolve(plan());
      if (cmd === "run_sync")
        return Promise.resolve({
          kind: "conflicted",
          files: ["environments/local.toml"],
          items: [
            {
              path: "environments/local.toml",
              ours: {
                exists: true,
                kind: "environment",
                name: "local",
                text: 'value = "yours"\n',
              },
              theirs: {
                exists: true,
                kind: "environment",
                name: "local",
                text: 'value = "theirs"\n',
              },
            },
          ],
        });
      if (cmd === "apply_conflict_choices") return Promise.resolve({});
    });
    const onClose = vi.fn();
    render(<SyncDialog onClose={onClose} />);

    await waitFor(() =>
      expect(screen.getByText(/Commit and push/)).toBeTruthy(),
    );
    fireEvent.change(screen.getByPlaceholderText("What changed?"), {
      target: { value: "collide" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sync" }));

    await waitFor(() => expect(screen.getByText("Resolve")).toBeTruthy());
    expect(screen.getByText(/value = "yours"/)).toBeTruthy();
    expect(screen.getByText(/value = "theirs"/)).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Yours diff"));
    fireEvent.click(screen.getByRole("button", { name: "Keep these" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("apply_conflict_choices", {
        workspace: "/ws",
        decisions: [
          { path: "environments/local.toml", choice: "ours" },
        ],
      }),
    );
    await waitFor(() =>
      expect(useToasts.getState().items.map((t) => t.text)).toContain(
        "Choices kept — sync again to finish",
      ),
    );
  });

  it("opens Resolve as soon as the plan lists conflicted paths", async () => {
    mockApi((cmd) => {
      if (cmd === "plan_sync")
        return Promise.resolve(
          plan({
            conflicted: ["environments/local.toml"],
            conflictItems: [
              {
                path: "environments/local.toml",
                ours: {
                  exists: true,
                  kind: "environment",
                  name: "local",
                  text: 'value = "local"\n',
                },
                theirs: {
                  exists: true,
                  kind: "environment",
                  name: "local",
                  text: 'value = "remote"\n',
                },
              },
            ],
            action: "nothing",
          }),
        );
    });
    render(<SyncDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText(/value = "local"/)).toBeTruthy());
    expect(screen.getByText(/value = "remote"/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Keep these" })).toBeTruthy();
  });
});
