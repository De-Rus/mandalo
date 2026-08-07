import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { StreamEvent } from "../../lib/stream";
import { toRow, type LogRow } from "../../lib/streamLog";
import { MessageLog } from "./MessageLog";

afterEach(cleanup);

const AT = 1_764_000_000_000;

function message(seq: number, text: string, direction: "incoming" | "outgoing" = "incoming"): LogRow {
  const event: StreamEvent = {
    type: "message",
    at: AT,
    direction,
    payload: { kind: "text", text },
    meta: {},
  };
  return toRow(event, seq);
}

function lifecycle(seq: number): LogRow {
  return toRow({ type: "connected", at: AT, info: { url: "wss://x.dev" } }, seq);
}

function show(rows: LogRow[], overflow = 0) {
  const onClear = vi.fn();
  const view = render(<MessageLog rows={rows} overflow={overflow} onClear={onClear} />);
  return { onClear, ...view };
}

/** jsdom has no layout, so the scroll geometry is stated rather than measured. */
function scrollTo(top: number, height = 1000, client = 300) {
  const el = screen.getByRole("log");
  Object.defineProperty(el, "scrollHeight", { value: height, configurable: true });
  Object.defineProperty(el, "clientHeight", { value: client, configurable: true });
  Object.defineProperty(el, "scrollTop", { value: top, writable: true, configurable: true });
  fireEvent.scroll(el);
  return el;
}

describe("the message log", () => {
  it("draws a row per event, newest last", () => {
    show([message(0, "first"), message(1, "second")]);
    const summaries = screen.getAllByText(/first|second/).map((n) => n.textContent);
    expect(summaries).toEqual(["first", "second"]);
  });

  it("says so when the cap has thrown rows away", () => {
    show([message(0, "kept")], 120);
    expect(screen.getByRole("status").textContent).toContain("Showing the last 2,000");
    expect(screen.getByRole("status").textContent).toContain("120");
  });

  it("says nothing about the cap while nothing was dropped", () => {
    show([message(0, "kept")]);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("tells the user when there is nothing yet", () => {
    show([]);
    expect(screen.getByText(/Connect, and every frame lands here/)).toBeTruthy();
  });
});

describe("autoscroll", () => {
  it("follows by default", () => {
    show([message(0, "a")]);
    expect(
      screen.getByRole("button", { name: "Follow" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("pauses the moment the user scrolls up", () => {
    show([message(0, "a"), message(1, "b")]);
    scrollTo(100);
    expect(
      screen.getByRole("button", { name: "Follow" }).getAttribute("aria-pressed"),
    ).toBe("false");
    expect(screen.getByRole("button", { name: /jump to the newest/i })).toBeTruthy();
  });

  it("resumes when the user scrolls back to the bottom", () => {
    show([message(0, "a")]);
    scrollTo(100);
    scrollTo(700);
    expect(
      screen.getByRole("button", { name: "Follow" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(screen.queryByRole("button", { name: /jump to the newest/i })).toBeNull();
  });

  it("jumps back to the newest on demand", async () => {
    const user = userEvent.setup();
    show([message(0, "a")]);
    scrollTo(100);
    await user.click(screen.getByRole("button", { name: /jump to the newest/i }));
    expect(
      screen.getByRole("button", { name: "Follow" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("does not resume on its own when new rows arrive", () => {
    const { rerender, onClear } = show([message(0, "a")]);
    scrollTo(100);
    rerender(
      <MessageLog rows={[message(0, "a"), message(1, "b")]} overflow={0} onClear={onClear} />,
    );
    expect(
      screen.getByRole("button", { name: "Follow" }).getAttribute("aria-pressed"),
    ).toBe("false");
  });
});

describe("filtering", () => {
  const rows = [message(0, "in one"), message(1, "out one", "outgoing"), lifecycle(2)];

  it("narrows to one direction", async () => {
    const user = userEvent.setup();
    show(rows);
    await user.click(screen.getByRole("button", { name: "Outgoing" }));
    expect(screen.queryByText("in one")).toBeNull();
    expect(screen.getByText("out one")).toBeTruthy();
  });

  it("narrows to the connection's own story", async () => {
    const user = userEvent.setup();
    show(rows);
    await user.click(screen.getByRole("button", { name: "Connection" }));
    expect(screen.queryByText("in one")).toBeNull();
    expect(screen.getByText(/Connected to wss:\/\/x.dev/)).toBeTruthy();
  });

  it("searches the text", async () => {
    const user = userEvent.setup();
    show(rows);
    await user.type(screen.getByLabelText("Filter the log"), "out");
    expect(screen.queryByText("in one")).toBeNull();
    expect(screen.getByText("out one")).toBeTruthy();
  });

  it("says when a filter matches nothing", async () => {
    const user = userEvent.setup();
    show(rows);
    await user.type(screen.getByLabelText("Filter the log"), "nothing at all");
    expect(screen.getByText("No message matches this filter.")).toBeTruthy();
  });
});

describe("housekeeping", () => {
  it("clears on demand", async () => {
    const user = userEvent.setup();
    const { onClear } = show([message(0, "a")]);
    await user.click(screen.getByLabelText("Clear the log"));
    expect(onClear).toHaveBeenCalled();
  });

  it("lets go of its resize observer when it unmounts", () => {
    const disconnect = vi.fn();
    const original = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      observe(): void {}
      unobserve(): void {}
      disconnect = disconnect;
    } as unknown as typeof ResizeObserver;
    const { unmount } = show([message(0, "a")]);
    unmount();
    globalThis.ResizeObserver = original;
    expect(disconnect).toHaveBeenCalled();
  });

  it("only puts the rows in view into the DOM", () => {
    const rows = Array.from({ length: 500 }, (_, i) => message(i, `m${i}`));
    const { container } = show(rows);
    const drawn = container.querySelectorAll(".log-row").length;
    expect(drawn).toBeGreaterThan(0);
    expect(drawn).toBeLessThan(rows.length);
  });
});
