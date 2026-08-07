import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { newDraft } from "../lib/draft";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { CurlDialog } from "./CurlDialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("CurlDialog", () => {
  beforeEach(() => {
    invoke.mockReset();
    useCollection.setState({ workspace: "/ws" });
    useEnv.setState({ selected: "hosted", envs: [], workspace: "/ws", error: null });
  });

  afterEach(cleanup);

  it("renders the curl command and copies it", async () => {
    const command = "curl -X GET 'https://api.dev/users'";
    invoke.mockResolvedValue(command);
    render(
      <CurlDialog draft={newDraft("R", "http")} onClose={() => {}} />,
    );
    await waitFor(() => expect(screen.getByText(command)).toBeTruthy());
    expect(invoke).toHaveBeenCalledWith(
      "to_curl",
      expect.objectContaining({
        workspace: "/ws",
        env: "hosted",
      }),
    );

    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    fireEvent.click(screen.getByText("Copy"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(command));
  });

  it("refuses a stream request without calling the backend", async () => {
    render(
      <CurlDialog
        draft={newDraft("R", "websocket")}
        onClose={() => {}}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText(/websocket request cannot be written as a curl/i),
      ).toBeTruthy(),
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it("surfaces an unresolved-variable error from the engine", async () => {
    invoke.mockRejectedValue(new Error("unresolved variable: token"));
    render(<CurlDialog draft={newDraft("R", "http")} onClose={() => {}} />);
    await waitFor(() =>
      expect(screen.getByText(/unresolved variable: token/i)).toBeTruthy(),
    );
  });
});
