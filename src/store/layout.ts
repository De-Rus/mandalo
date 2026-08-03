import { create } from "zustand";

const KEY = "mandalo.layout.v1";

export const SIDEBAR_MIN = 180;
export const SIDEBAR_MAX = 460;
export const RESPONSE_MIN = 0.2;
export const RESPONSE_MAX = 0.8;

interface LayoutState {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  responseRatio: number;
  setSidebarWidth: (width: number) => void;
  toggleSidebar: () => void;
  setResponseRatio: (ratio: number) => void;
}

interface Stored {
  sidebarWidth?: number;
  sidebarCollapsed?: boolean;
  responseRatio?: number;
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

const stored = read();

export const useLayout = create<LayoutState>((set, get) => ({
  sidebarWidth: clamp(stored.sidebarWidth ?? 262, SIDEBAR_MIN, SIDEBAR_MAX),
  sidebarCollapsed: stored.sidebarCollapsed ?? false,
  responseRatio: clamp(stored.responseRatio ?? 0.45, RESPONSE_MIN, RESPONSE_MAX),
  setSidebarWidth: (width) => {
    set({ sidebarWidth: clamp(width, SIDEBAR_MIN, SIDEBAR_MAX) });
    persist(get());
  },
  toggleSidebar: () => {
    set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
    persist(get());
  },
  setResponseRatio: (ratio) => {
    set({ responseRatio: clamp(ratio, RESPONSE_MIN, RESPONSE_MAX) });
    persist(get());
  },
}));

function persist(state: LayoutState): void {
  localStorage.setItem(
    KEY,
    JSON.stringify({
      sidebarWidth: state.sidebarWidth,
      sidebarCollapsed: state.sidebarCollapsed,
      responseRatio: state.responseRatio,
    }),
  );
}
