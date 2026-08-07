import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConflictDialog } from "./ConflictDialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("ConflictDialog", () => {
  beforeEach(() => invoke.mockReset());
  afterEach(cleanup);

  it("lets you keep both request versions and a config side", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "apply_conflict_choices") return Promise.resolve({});
      return Promise.resolve(undefined);
    });
    const onResolved = vi.fn();
    render(
      <ConflictDialog
        workspace="/ws"
        files={[
          "collections/a/api.http",
          "environments/local.toml",
        ]}
        items={[
          {
            path: "collections/a/api.http",
            ours: {
              exists: true,
              text: "### Login\n\nPOST {{baseUrl}}/login\n",
            },
            theirs: {
              exists: true,
              text: "### Login\n\nPOST {{baseUrl}}/auth/login\n",
            },
          },
          {
            path: "environments/local.toml",
            ours: {
              exists: true,
              text: 'value = "bob"\n',
            },
            theirs: {
              exists: true,
              text: 'value = "alice"\n',
            },
          },
        ]}
        onClose={() => {}}
        onResolved={onResolved}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Both" }));
    expect(screen.getByText(/POST \{\{baseUrl\}\}\/login/)).toBeTruthy();
    expect(screen.getByText(/POST \{\{baseUrl\}\}\/auth\/login/)).toBeTruthy();

    const yoursDiffs = screen.getAllByLabelText("Yours diff");
    fireEvent.click(yoursDiffs[yoursDiffs.length - 1]);

    fireEvent.click(screen.getByRole("button", { name: "Keep these" }));

    await waitFor(() => expect(invoke).toHaveBeenCalled());
    const call = invoke.mock.calls.find((c) => c[0] === "apply_conflict_choices");
    expect(call?.[1].decisions).toEqual([
      {
        path: "collections/a/api.http",
        choice: "ours",
        content: expect.stringContaining("### Login (remote)"),
      },
      {
        path: "environments/local.toml",
        choice: "ours",
      },
    ]);
    expect(call?.[1].decisions[0].content).toContain("POST {{baseUrl}}/login");
    expect(call?.[1].decisions[0].content).toContain(
      "POST {{baseUrl}}/auth/login",
    );
    expect(onResolved).toHaveBeenCalled();
  });
});
