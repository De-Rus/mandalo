import { beforeEach, describe, expect, it } from "vitest";
import {
  CENTER_MIN,
  clampSidebar,
  SIDEBAR_MAX,
  SIDEBAR_MIN,
  useLayout,
} from "./layout";

describe("clampSidebar", () => {
  it("keeps a sane width on a wide window", () => {
    expect(clampSidebar(262, 1280)).toBe(262);
  });

  it("clamps a width stored on a wider monitor down to the window", () => {
    expect(clampSidebar(SIDEBAR_MAX, 700)).toBe(700 - CENTER_MIN);
  });

  it("never lets the sidebar leave the workbench less than its minimum", () => {
    for (let width = 900; width <= 1600; width += 37) {
      const sidebar = clampSidebar(SIDEBAR_MAX, width);
      expect(width - sidebar).toBeGreaterThanOrEqual(CENTER_MIN);
    }
  });

  it("falls back to the sidebar minimum when the window cannot hold both", () => {
    expect(clampSidebar(SIDEBAR_MAX, 400)).toBe(SIDEBAR_MIN);
    expect(clampSidebar(50, 400)).toBe(SIDEBAR_MIN);
  });

  it("never exceeds the sidebar maximum however wide the window", () => {
    expect(clampSidebar(4000, 4000)).toBe(SIDEBAR_MAX);
  });
});

describe("useLayout", () => {
  beforeEach(() => {
    useLayout.setState({ viewportWidth: 1280, sidebarWidth: 262 });
  });

  it("re-clamps the stored width when the window shrinks", () => {
    useLayout.getState().setSidebarWidth(SIDEBAR_MAX);
    expect(useLayout.getState().sidebarWidth).toBe(SIDEBAR_MAX);

    useLayout.getState().setViewportWidth(700);
    expect(useLayout.getState().sidebarWidth).toBe(700 - CENTER_MIN);
  });

  it("clamps a drag past either end", () => {
    useLayout.getState().setSidebarWidth(-100);
    expect(useLayout.getState().sidebarWidth).toBe(SIDEBAR_MIN);
    useLayout.getState().setSidebarWidth(9999);
    expect(useLayout.getState().sidebarWidth).toBe(SIDEBAR_MAX);
  });
});
