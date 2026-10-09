import { create } from "zustand";
import type { AppInfo } from "@/bindings/AppInfo";
import type { AppSettings } from "@/bindings/AppSettings";
import { api } from "@/lib/api";

interface SettingsState {
  settings: AppSettings | null;
  info: AppInfo | null;
  load: () => Promise<void>;
  /** Persist a modified copy; the backend returns the normalized document. */
  save: (update: (s: AppSettings) => AppSettings) => Promise<AppSettings>;
  replace: (s: AppSettings) => void;
}

export const useSettings = create<SettingsState>((set, get) => ({
  settings: null,
  info: null,
  load: async () => {
    const [settings, info] = await Promise.all([api.getSettings(), api.appInfo()]);
    set({ settings, info });
  },
  save: async (update) => {
    const current = get().settings;
    if (!current) throw new Error("settings not loaded");
    const next = update(structuredClone(current));
    const saved = await api.updateSettings(next);
    set({ settings: saved });
    return saved;
  },
  replace: (settings) => set({ settings }),
}));
