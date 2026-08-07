import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollection } from "../../store/collection";
import { LAPSED_MARKER } from "./mounts";
import { Notices } from "./Notices";

const reconnect = vi.hoisted(() => vi.fn(() => Promise.resolve(true)));
vi.mock("./mounts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./mounts")>()),
  reconnectActive: reconnect,
}));

afterEach(cleanup);

beforeEach(() => {
  reconnect.mockClear();
  useCollection.setState({
    saveError: null,
    conflicts: [],
    vanished: [],
    activeId: "r1",
  });
});

describe("telling the user their work did not land", () => {
  it("says nothing when every save succeeded", () => {
    const view = render(<Notices />);

    expect(view.container.textContent).toBe("");
  });

  it("shows a failed autosave loudly instead of swallowing it", () => {
    useCollection.setState({ saveError: "QuotaExceededError" });

    render(<Notices />);

    expect(screen.getByRole("alert").textContent).toContain(
      "Your last change was not saved.",
    );
    expect(screen.getByRole("alert").textContent).toContain("QuotaExceededError");
  });

  it("offers to reconnect the folder when that is what broke", async () => {
    useCollection.setState({
      saveError: `Mándalo ${LAPSED_MARKER} “api”. Reconnect the folder to keep saving.`,
    });

    render(<Notices />);
    await userEvent.click(screen.getByRole("button", { name: "Reconnect folder" }));

    expect(reconnect).toHaveBeenCalled();
  });

  it("offers a plain retry for any other failure", () => {
    useCollection.setState({ saveError: "the disk went away" });

    render(<Notices />);

    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

describe("telling the user another tab changed the same request", () => {
  it("asks instead of picking a winner", () => {
    useCollection.setState({ conflicts: ["r1"] });

    render(<Notices />);

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("This request changed in another tab.");
    expect(screen.getByRole("button", { name: "Load theirs" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Keep mine" })).toBeTruthy();
  });

  it("stays quiet about a conflict on a request the user is not looking at", () => {
    useCollection.setState({ conflicts: ["r2"] });

    const view = render(<Notices />);

    expect(view.container.textContent).toBe("");
  });

  it("tells the user when the open request was deleted elsewhere", () => {
    useCollection.setState({ vanished: ["r1"] });

    render(<Notices />);

    expect(screen.getByRole("alert").textContent).toContain(
      "This request was deleted in another tab.",
    );
  });
});
