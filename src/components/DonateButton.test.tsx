import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DONATE_URL } from "../lib/web/config";
import { DonateButton } from "./DonateButton";

const openUrl = vi.fn();

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (...args: unknown[]) => openUrl(...args),
}));

afterEach(() => {
  cleanup();
  openUrl.mockReset();
});

describe("DonateButton", () => {
  it("opens the GitHub Sponsors page", async () => {
    openUrl.mockResolvedValueOnce(undefined);
    render(<DonateButton />);
    fireEvent.click(screen.getByRole("button", { name: "Donate" }));
    await waitFor(() => expect(openUrl).toHaveBeenCalledWith(DONATE_URL));
  });
});
