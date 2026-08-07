import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Outgoing, SavedMessage, StreamKind } from "../../lib/stream";
import { Composer } from "./Composer";

function show(kind: StreamKind, patch: Partial<Parameters<typeof Composer>[0]> = {}) {
  const onSend = vi.fn();
  const onSaveMessage = vi.fn();
  const onForgetMessage = vi.fn();
  render(
    <Composer
      kind={kind}
      connected
      sending={false}
      subscriptions={[]}
      saved={[]}
      onSend={onSend}
      onSaveMessage={onSaveMessage}
      onForgetMessage={onForgetMessage}
      {...patch}
    />,
  );
  return { onSend, onSaveMessage, onForgetMessage };
}

const sent = (fn: ReturnType<typeof vi.fn>): Outgoing => fn.mock.calls[0][0];

afterEach(cleanup);

describe("the mqtt composer", () => {
  it("publishes a topic, a payload, a QoS and a retain flag", async () => {
    const user = userEvent.setup();
    const { onSend } = show("mqtt");
    await user.type(screen.getByLabelText("Publish topic"), "sensors/kitchen");
    await user.type(screen.getByLabelText("Message body"), '{{"c":21}');
    await user.selectOptions(screen.getByLabelText("QoS"), "1");
    await user.click(screen.getByLabelText("Retain"));
    await user.click(screen.getByRole("button", { name: /publish/i }));
    expect(sent(onSend)).toEqual({
      kind: "publish",
      topic: "sensors/kitchen",
      payload: '{"c":21}',
      qos: 1,
      retain: true,
    });
  });

  it("trims the topic and defaults QoS to 0 and retain to off", async () => {
    const user = userEvent.setup();
    const { onSend } = show("mqtt");
    await user.type(screen.getByLabelText("Publish topic"), "  a/b  ");
    await user.type(screen.getByLabelText("Message body"), "ping");
    await user.click(screen.getByRole("button", { name: /publish/i }));
    expect(sent(onSend)).toEqual({
      kind: "publish",
      topic: "a/b",
      payload: "ping",
      qos: 0,
      retain: false,
    });
  });

  it("will not publish without a topic", async () => {
    const user = userEvent.setup();
    show("mqtt");
    await user.type(screen.getByLabelText("Message body"), "orphan");
    expect(
      (screen.getByRole("button", { name: /publish/i }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("subscribes to a topic filter with its QoS", async () => {
    const user = userEvent.setup();
    const { onSend } = show("mqtt");
    await user.type(screen.getByLabelText("Subscribe topic"), "sensors/#");
    await user.selectOptions(screen.getByLabelText("Subscribe QoS"), "1");
    await user.click(screen.getByRole("button", { name: "Subscribe" }));
    expect(sent(onSend)).toEqual({ kind: "subscribe", topic: "sensors/#", qos: 1 });
  });

  it("unsubscribes from a live subscription", async () => {
    const user = userEvent.setup();
    const { onSend } = show("mqtt", { subscriptions: ["sensors/#"] });
    await user.click(screen.getByLabelText("Unsubscribe from sensors/#"));
    expect(sent(onSend)).toEqual({ kind: "unsubscribe", topic: "sensors/#" });
  });
});

describe("the websocket composer", () => {
  it("sends free text", async () => {
    const user = userEvent.setup();
    const { onSend } = show("websocket");
    await user.type(screen.getByLabelText("Message body"), "hello");
    await user.click(screen.getByRole("button", { name: /^send$/i }));
    expect(sent(onSend)).toEqual({ kind: "text", text: "hello" });
  });

  it("sends on ⌘⏎", async () => {
    const user = userEvent.setup();
    const { onSend } = show("websocket");
    const body = screen.getByLabelText("Message body");
    await user.type(body, "hello");
    await user.type(body, "{Meta>}{Enter}{/Meta}");
    expect(sent(onSend)).toEqual({ kind: "text", text: "hello" });
  });

  it("cannot send while the connection is down", () => {
    show("websocket", { connected: false });
    expect(
      (screen.getByRole("button", { name: /^send$/i }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("has no topic fields", () => {
    show("websocket");
    expect(screen.queryByLabelText("Publish topic")).toBeNull();
  });
});

describe("sse", () => {
  it("says there is nothing to send rather than offering a dead box", () => {
    show("sse");
    expect(screen.queryByLabelText("Message body")).toBeNull();
    expect(screen.getByText(/nothing to send on this stream/i)).toBeTruthy();
  });
});

describe("saved messages", () => {
  const SAVED: SavedMessage[] = [
    { id: "m1", name: "Ping", message: { kind: "text", text: "ping" } },
  ];

  it("sends one with a single click", async () => {
    const user = userEvent.setup();
    const { onSend } = show("websocket", { saved: SAVED });
    await user.click(screen.getByRole("button", { name: "Ping" }));
    expect(sent(onSend)).toEqual({ kind: "text", text: "ping" });
  });

  it("saves what is in the box under a name", async () => {
    const user = userEvent.setup();
    const { onSaveMessage } = show("websocket");
    await user.type(screen.getByLabelText("Message body"), "hi there");
    await user.click(screen.getByRole("button", { name: /^save$/i }));
    const name = screen.getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Greeting");
    const modal = within(document.querySelector(".modal") as HTMLElement);
    await user.click(modal.getByRole("button", { name: "Save" }));
    expect(onSaveMessage.mock.calls[0][0]).toMatchObject({
      name: "Greeting",
      message: { kind: "text", text: "hi there" },
    });
  });

  it("forgets one", async () => {
    const user = userEvent.setup();
    const { onForgetMessage } = show("websocket", { saved: SAVED });
    await user.click(screen.getByLabelText("Forget Ping"));
    expect(onForgetMessage).toHaveBeenCalledWith("m1");
  });
});
