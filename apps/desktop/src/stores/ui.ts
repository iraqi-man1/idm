import { create } from "zustand";

export type Page = "downloads" | "settings" | "statistics" | "queues";

interface UiState {
  page: Page;
  settingsSection: string;
  addDialog: { open: boolean; url: string };
  batchDialog: boolean;
  deleteDialog: { open: boolean; ids: string[] };
  propertiesId: string | null;
  checksumId: string | null;
  refreshUrlId: string | null;
  renameId: string | null;
  setPage: (p: Page, section?: string) => void;
  openAdd: (url?: string) => void;
  closeAdd: () => void;
  setBatch: (open: boolean) => void;
  openDelete: (ids: string[]) => void;
  closeDelete: () => void;
  setProperties: (id: string | null) => void;
  setChecksum: (id: string | null) => void;
  setRefreshUrl: (id: string | null) => void;
  setRename: (id: string | null) => void;
}

export const useUi = create<UiState>((set) => ({
  page: "downloads",
  settingsSection: "general",
  addDialog: { open: false, url: "" },
  batchDialog: false,
  deleteDialog: { open: false, ids: [] },
  propertiesId: null,
  checksumId: null,
  refreshUrlId: null,
  renameId: null,
  setPage: (page, section) => set((s) => ({ page, settingsSection: section ?? s.settingsSection })),
  openAdd: (url = "") => set({ addDialog: { open: true, url } }),
  closeAdd: () => set({ addDialog: { open: false, url: "" } }),
  setBatch: (batchDialog) => set({ batchDialog }),
  openDelete: (ids) => set({ deleteDialog: { open: true, ids } }),
  closeDelete: () => set({ deleteDialog: { open: false, ids: [] } }),
  setProperties: (propertiesId) => set({ propertiesId }),
  setChecksum: (checksumId) => set({ checksumId }),
  setRefreshUrl: (refreshUrlId) => set({ refreshUrlId }),
  setRename: (renameId) => set({ renameId }),
}));
