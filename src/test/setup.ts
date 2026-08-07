import { beforeEach } from "vitest";

// The suite spans jsdom and node environments; a node test file has no storage.
beforeEach(() => {
  if (typeof sessionStorage !== "undefined") sessionStorage.clear();
});

class NoopResizeObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver ??= NoopResizeObserver as unknown as typeof ResizeObserver;

if (typeof Range !== "undefined" && !Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: [][Symbol.iterator],
  }) as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () =>
    ({ x: 0, y: 0, top: 0, left: 0, bottom: 0, right: 0, width: 0, height: 0 }) as DOMRect;
}

if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}
