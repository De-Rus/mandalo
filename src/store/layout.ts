import { create } from "zustand";

const KEY = "mandalo.layout.v1";

export const SIDEBAR_MIN = 180;
export const SIDEBAR_MAX = 460;
export const CENTER_MIN = 340;
export const RESPONSE_MIN = 0.2;
export const RESPONSE_MAX = 0.8;
export const CONTEXT_WIDTH = 320;



interface LayoutState {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  collectionsOpen: boolean;
  environmentsOpen: boolean;
  responseRatio: number;
  /**
   * A stream's log is the screen, not a footnote to it, so the two layouts are
   * remembered apart: dragging the splitter on a WebSocket must not shrink the
   * next HTTP response.
   */
  streamRatio: number;
  viewportWidth: number;
  contextOpen: boolean;
  setSidebarWidth: (width: number) => void;
  toggleSidebar: () => void;
  setCollectionsOpen: (open: boolean) => void;
  setEnvironmentsOpen: (open: boolean) => void;
  setResponseRatio: (ratio: number) => void;
  setStreamRatio: (ratio: number) => void;
  setViewportWidth: (width: number) => void;
  toggleContext: () => void;
  setContextOpen: (open: boolean) => void;
}

interface Stored {
  sidebarWidth?: number;
  sidebarCollapsed?: boolean;
  collectionsOpen?: boolean;
  environmentsOpen?: boolean;
  responseRatio?: number;
  streamRatio?: number;
  contextOpen?: boolean;
}

function read(): Stored {
  const raw = localStorage.getItem(KEY);
  if (!raw) return {};
  try {
    return JSON.parse(raw) as Stored;
  } catch {
    return {};
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * A width stored on a wide monitor must not survive into a narrow window: the
 * sidebar never shrinks, so an unclamped value eats the workbench. The centre
 * keeps `CENTER_MIN` until the window itself is too small to give it, and only
 * then does the sidebar fall back to its own minimum.
 */
export function clampSidebar(width: number, viewport: number): number {
  const roomForSidebar = viewport - CENTER_MIN;
  const max = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, roomForSidebar));
  return Math.round(clamp(width, SIDEBAR_MIN, max));
}

function viewport(): number {
  return typeof window === "undefined" ? SIDEBAR_MAX + CENTER_MIN : window.innerWidth;
}

const stored = read();

export const useLayout = create<LayoutState>((set, get) => ({
  sidebarWidth: clampSidebar(stored.sidebarWidth ?? 262, viewport()),
  sidebarCollapsed: stored.sidebarCollapsed ?? false,
  collectionsOpen: stored.collectionsOpen ?? true,
  environmentsOpen: stored.environmentsOpen ?? false,
  responseRatio: clamp(stored.responseRatio ?? 0.45, RESPONSE_MIN, RESPONSE_MAX),
  streamRatio: clamp(stored.streamRatio ?? 0.68, RESPONSE_MIN, RESPONSE_MAX),
  viewportWidth: viewport(),
  contextOpen: stored.contextOpen ?? true,
  setSidebarWidth: (width) => {
    set((s) => ({ sidebarWidth: clampSidebar(width, s.viewportWidth) }));
    persist(get());
  },
  toggleSidebar: () => {
    set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
    persist(get());
  },
  setCollectionsOpen: (open) => {
    set({ collectionsOpen: open });
    persist(get());
  },
  setEnvironmentsOpen: (open) => {
    set({ environmentsOpen: open });
    persist(get());
  },
  toggleContext: () => {
    set((s) => ({ contextOpen: !s.contextOpen }));
    persist(get());
  },
  setContextOpen: (open) => {
    set({ contextOpen: open });
    persist(get());
  },
  setResponseRatio: (ratio) => {
    set({ responseRatio: clamp(ratio, RESPONSE_MIN, RESPONSE_MAX) });
    persist(get());
  },
  setStreamRatio: (ratio) => {
    set({ streamRatio: clamp(ratio, RESPONSE_MIN, RESPONSE_MAX) });
    persist(get());
  },
  setViewportWidth: (width) =>
    set((s) => ({
      viewportWidth: width,
      sidebarWidth: clampSidebar(s.sidebarWidth, width),
    })),
}));

function persist(state: LayoutState): void {
  localStorage.setItem(
    KEY,
    JSON.stringify({
      sidebarWidth: state.sidebarWidth,
      sidebarCollapsed: state.sidebarCollapsed,
      collectionsOpen: state.collectionsOpen,
      environmentsOpen: state.environmentsOpen,
      responseRatio: state.responseRatio,
      streamRatio: state.streamRatio,
      contextOpen: state.contextOpen,
    }),
  );
}
