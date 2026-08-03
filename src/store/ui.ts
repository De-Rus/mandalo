import { useEffect } from "react";
import { create } from "zustand";

interface UiState {
  modalCount: number;
  modalOpened: () => void;
  modalClosed: () => void;
}

export const useUi = create<UiState>((set) => ({
  modalCount: 0,
  modalOpened: () => set((s) => ({ modalCount: s.modalCount + 1 })),
  modalClosed: () => set((s) => ({ modalCount: Math.max(0, s.modalCount - 1) })),
}));

export function useModalGuard(): void {
  const opened = useUi((s) => s.modalOpened);
  const closed = useUi((s) => s.modalClosed);
  useEffect(() => {
    opened();
    return closed;
  }, [opened, closed]);
}

export function anyModalOpen(): boolean {
  return useUi.getState().modalCount > 0;
}
