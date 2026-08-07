import { afterEach, describe, expect, it, vi } from "vitest";
import { locksSupported, withWriteLock } from "./locks";

function fakeLocks() {
  const held = new Map<string, Promise<unknown>>();
  return {
    request: (name: string, run: () => Promise<unknown>) => {
      const previous = held.get(name) ?? Promise.resolve();
      const next = previous.then(run, run);
      held.set(
        name,
        next.catch(() => undefined),
      );
      return next;
    },
  };
}

function withLockManager(value: unknown) {
  Object.defineProperty(navigator, "locks", {
    value,
    configurable: true,
  });
}

afterEach(() => {
  Object.defineProperty(navigator, "locks", {
    value: undefined,
    configurable: true,
  });
});

async function racingWriters(): Promise<string[]> {
  const file = { text: "" };
  const append = (mark: string) => async () => {
    const seen = file.text;
    await new Promise((r) => setTimeout(r, 5));
    file.text = `${seen}${mark}`;
  };
  await Promise.all([
    withWriteLock("browser", append("A")),
    withWriteLock("browser", append("B")),
  ]);
  return [file.text];
}

describe("serialising writes across tabs", () => {
  it("makes the second writer wait, so neither write is lost", async () => {
    withLockManager(fakeLocks());
    expect(locksSupported()).toBe(true);

    const [text] = await racingWriters();

    expect(text.length).toBe(2);
    expect([...text].sort().join("")).toBe("AB");
  });

  it("keeps a workspace's writes off another workspace's lock", async () => {
    withLockManager(fakeLocks());
    const order: string[] = [];

    await Promise.all([
      withWriteLock("one", async () => {
        await new Promise((r) => setTimeout(r, 10));
        order.push("one");
      }),
      withWriteLock("two", async () => {
        order.push("two");
      }),
    ]);

    expect(order).toEqual(["two", "one"]);
  });

  it("still serialises this tab's own writes when the Locks API is missing", async () => {
    withLockManager(undefined);
    expect(locksSupported()).toBe(false);

    const [text] = await racingWriters();

    expect(text.length).toBe(2);
  });

  it("does not wedge the queue when a write fails", async () => {
    withLockManager(undefined);
    const failed = withWriteLock("browser", () => Promise.reject(new Error("disk")));

    await expect(failed).rejects.toThrow("disk");
    await expect(withWriteLock("browser", async () => "next")).resolves.toBe("next");
  });

  it("uses the real Locks API when the browser has one", async () => {
    const request = vi.fn((_: string, run: () => Promise<unknown>) => run());
    withLockManager({ request });

    await withWriteLock("browser", async () => "done");

    expect(request).toHaveBeenCalledWith("mandalo.write.browser", expect.any(Function));
  });
});
