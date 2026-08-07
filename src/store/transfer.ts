import { create } from "zustand";
import type { LoadedDocument } from "../lib/importSource";

interface TransferState {
  importOpen: boolean;
  dropped: LoadedDocument | null;
  dropSeq: number;
  openImport: (dropped?: LoadedDocument) => void;
  closeImport: () => void;
}

export const useTransfer = create<TransferState>((set) => ({
  importOpen: false,
  dropped: null,
  dropSeq: 0,
  openImport: (dropped) =>
    set((s) => ({
      importOpen: true,
      dropped: dropped ?? null,
      dropSeq: s.dropSeq + 1,
    })),
  closeImport: () => set({ importOpen: false, dropped: null }),
}));
