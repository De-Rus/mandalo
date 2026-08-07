import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { useToasts } from "../store/toast";
import { ExportDialog } from "./ExportDialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { open, save } = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open, save }));

const FINDING = {
  path: "environments/prod.toml",
  line: 4,
  rule: "aws-access-key-id",
  excerpt: "key = AKIA…",
};

function plan(over: Record<string, unknown> = {}) {
  return {
    included: {
      collections: [{ slug: "acme", name: "Acme API", requests: [{ path: "a.toml", name: "a" }] }],
      environments: ["staging"],
      requestCount: 1,
    },
    excluded: {
      secretValues: 1,
      localValues: 0,
      withheldNames: [],
      collections: [],
      requests: 0,
      environments: [],
    },
    findings: [],
    bytes: 512,
    blocked: false,
    token: "plan-1",
    format: "bundle",
    warnings: [],
    ...over,
  };
}

const RECEIPT = {
  path: "/out.json",
  bytes: 512,
  requests: 1,
  collections: 1,
  environments: 1,
  forced: false,
};

describe("ExportDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    save.mockReset();
    useCollection.setState({
      workspace: "/ws",
      tree: {
        collections: [
          {
            id: "1",
            slug: "acme",
            name: "Acme API",
            folders: [],
            requests: [
              { id: "a", path: "a.toml", name: "a", method: "GET", kind: "http" },
            ],
          },
          {
            id: "2",
            slug: "billing",
            name: "Billing",
            folders: [],
            requests: [
              { id: "b", path: "b.toml", name: "b", method: "POST", kind: "http" },
            ],
          },
        ],
        skipped: [],
      },
    });
    useEnv.setState({
      envs: [
        { name: "staging", vars: {} },
        { name: "prod", vars: {} },
      ],
      selected: "staging",
      error: null,
    } as never);
    useToasts.setState({ items: [] });
  });

  afterEach(cleanup);

  it("writes only after the plan has been reviewed and a path chosen", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share") return Promise.resolve(null);
      if (cmd === "plan_export") return Promise.resolve(plan());
      if (cmd === "run_export") return Promise.resolve(RECEIPT);
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue("/out.json");
    render(<ExportDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText("Acme API — 1 requests")).toBeTruthy());
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("run_export");

    fireEvent.click(screen.getByText("Choose file and export…"));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_export",
        expect.objectContaining({
          workspace: "/ws",
          token: "plan-1",
          path: "/out.json",
          force: false,
          format: "native",
        }),
      ),
    );
    expect(useToasts.getState().items.map((t) => t.text)).toContain(
      "1 requests written to /out.json",
    );
  });

  it("blocks on scanner findings until they are acknowledged", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share") return Promise.resolve(null);
      if (cmd === "plan_export")
        return Promise.resolve(plan({ findings: [FINDING], blocked: true }));
      if (cmd === "run_export") return Promise.resolve({ ...RECEIPT, forced: true });
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue("/out.json");
    render(<ExportDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText("aws-access-key-id")).toBeTruthy());
    const button = screen.getByText("Export anyway…").closest("button");
    expect(button?.hasAttribute("disabled")).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /findings to the file anyway/i }));
    expect(button?.hasAttribute("disabled")).toBe(false);

    fireEvent.click(button!);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_export",
        expect.objectContaining({
          workspace: "/ws",
          token: "plan-1",
          path: "/out.json",
          force: true,
          format: "native",
        }),
      ),
    );
  });

  it("does not write when no path is chosen", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share") return Promise.resolve(null);
      if (cmd === "plan_export") return Promise.resolve(plan());
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue(null);
    render(<ExportDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText("Choose file and export…")).toBeTruthy());
    fireEvent.click(screen.getByText("Choose file and export…"));

    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain("run_export");
  });

  it("reports a stale plan instead of retrying", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "workspace_share") return Promise.resolve(null);
      if (cmd === "plan_export") return Promise.resolve(plan());
      if (cmd === "run_export")
        return Promise.reject("the workspace changed since this export was reviewed");
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue("/out.json");
    render(<ExportDialog onClose={() => {}} />);

    await waitFor(() => expect(screen.getByText("Choose file and export…")).toBeTruthy());
    fireEvent.click(screen.getByText("Choose file and export…"));

    await waitFor(() =>
      expect(screen.getByText(/the workspace changed/)).toBeTruthy(),
    );
  });

  it("lets the user pick one collection for Postman without exporting the whole workspace", async () => {
    invoke.mockImplementation((cmd: string, args?: { selection?: { collections?: { slug: string }[] }; format?: string }) => {
      if (cmd === "workspace_share")
        return Promise.resolve({ format: "postman", dir: null });
      if (cmd === "plan_export") {
        const slug = args?.selection?.collections?.[0]?.slug ?? "acme";
        return Promise.resolve(
          plan({
            format: "postman",
            included: {
              collections: [
                {
                  slug,
                  name: slug === "billing" ? "Billing" : "Acme API",
                  requests: [{ path: "a.toml", name: "a" }],
                },
              ],
              environments: [],
              requestCount: 1,
            },
          }),
        );
      }
      if (cmd === "run_export") return Promise.resolve(RECEIPT);
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue("/collection.json");
    render(<ExportDialog onClose={() => {}} />);

    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /Acme API/i })).toBeTruthy(),
    );
    expect(screen.getByRole("radio", { name: /Billing/i })).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: /Billing/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "plan_export",
        expect.objectContaining({
          format: "postman",
          selection: { collections: [{ slug: "billing" }], environments: [] },
        }),
      ),
    );

    await waitFor(() =>
      expect(screen.getByText("Choose file and export…")).toBeTruthy(),
    );
    fireEvent.click(screen.getByText("Choose file and export…"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_export",
        expect.objectContaining({
          format: "postman",
          selection: { collections: [{ slug: "billing" }], environments: [] },
        }),
      ),
    );
  });
});

describe("choosing what goes into the bundle", () => {
  beforeEach(() => {
    invoke.mockReset();
    save.mockReset();
    useCollection.setState({
      workspace: "/ws",
      tree: {
        collections: [
          {
            id: "1",
            slug: "acme",
            name: "Acme API",
            folders: [],
            requests: [
              { id: "a", path: "a.toml", name: "a", method: "GET", kind: "http" },
            ],
          },
        ],
        skipped: [],
      },
    });
    useEnv.setState({
      envs: [{ name: "staging", vars: {} }],
      selected: "staging",
      error: null,
    } as never);
    useToasts.setState({ items: [] });
  });

  afterEach(cleanup);

  it("re-plans with the selection and writes only what stayed ticked", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "workspace_share") return Promise.resolve(null);
      if (command === "plan_export") return Promise.resolve(plan());
      if (command === "run_export")
        return Promise.resolve({
          path: "/out.json",
          bytes: 512,
          requests: 1,
          collections: 1,
          environments: 0,
          forced: false,
        });
      return Promise.resolve(undefined);
    });
    save.mockResolvedValue("/out.json");
    render(<ExportDialog onClose={() => {}} />);

    const env = await screen.findByRole("checkbox", { name: /staging \(environment\)/i });
    fireEvent.click(env);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("plan_export", {
        workspace: "/ws",
        selection: { collections: [{ slug: "acme" }], environments: [] },
        format: "native",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: /export/i }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "run_export",
        expect.objectContaining({
          selection: { collections: [{ slug: "acme" }], environments: [] },
          format: "native",
        }),
      ),
    );
  });
});
