import { create } from "zustand";
import { uid } from "../lib/draft";

export type ToastTone = "success" | "error" | "info";

export interface Toast {
  id: string;
  tone: ToastTone;
  text: string;
}

interface ToastState {
  items: Toast[];
  push: (tone: ToastTone, text: string) => void;
  dismiss: (id: string) => void;
}

const LIFETIME_MS = 3200;

export const useToasts = create<ToastState>((set) => ({
  items: [],
  push: (tone, text) => {
    const id = uid();
    set((s) => ({ items: [...s.items.slice(-3), { id, tone, text }] }));
    setTimeout(() => {
      set((s) => ({ items: s.items.filter((t) => t.id !== id) }));
    }, LIFETIME_MS);
  },
  dismiss: (id) => set((s) => ({ items: s.items.filter((t) => t.id !== id) })),
}));

export function toast(tone: ToastTone, text: string): void {
  useToasts.getState().push(tone, text);
}
