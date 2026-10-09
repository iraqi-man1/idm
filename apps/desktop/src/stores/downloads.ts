import { create } from "zustand";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import type { EngineEvent } from "@/bindings/EngineEvent";
import type { ProgressSnapshot } from "@/bindings/ProgressSnapshot";
import { api } from "@/lib/api";
import type { SortSpec, ViewFilter } from "@/lib/view";

interface DownloadsState {
  items: Record<string, DownloadInfo>;
  progress: Record<string, ProgressSnapshot>;
  loaded: boolean;
  filter: ViewFilter;
  search: string;
  sort: SortSpec;
  selected: string[];
  /** Anchor for shift-click range selection. */
  anchor: string | null;
  load: () => Promise<void>;
  applyEvent: (e: EngineEvent) => void;
  setFilter: (f: ViewFilter) => void;
  setSearch: (q: string) => void;
  setSort: (s: SortSpec) => void;
  setSelected: (ids: string[], anchor?: string | null) => void;
}

export const useDownloads = create<DownloadsState>((set) => ({
  items: {},
  progress: {},
  loaded: false,
  filter: "all",
  search: "",
  sort: { key: "date_added", dir: "desc" },
  selected: [],
  anchor: null,

  load: async () => {
    const [list, progress] = await Promise.all([api.listDownloads(), api.progress()]);
    const items: Record<string, DownloadInfo> = {};
    for (const d of list) items[d.id] = d;
    const prog: Record<string, ProgressSnapshot> = {};
    for (const p of progress) prog[p.id] = p;
    set({ items, progress: prog, loaded: true });
  },

  applyEvent: (e) =>
    set((s) => {
      switch (e.type) {
        case "added":
        case "updated":
        case "completed":
        case "failed": {
          const d = e.download;
          const items = { ...s.items, [d.id]: d };
          let progress = s.progress;
          if (!["connecting", "downloading", "retrying", "processing"].includes(d.status) && progress[d.id]) {
            progress = { ...progress };
            delete progress[d.id];
          }
          return { items, progress };
        }
        case "removed": {
          const items = { ...s.items };
          delete items[e.id];
          const progress = { ...s.progress };
          delete progress[e.id];
          return { items, progress, selected: s.selected.filter((x) => x !== e.id) };
        }
        case "progress": {
          const progress: Record<string, ProgressSnapshot> = {};
          for (const p of e.items) progress[p.id] = p;
          return { progress };
        }
      }
      return {};
    }),

  setFilter: (filter) => set({ filter, selected: [], anchor: null }),
  setSearch: (search) => set({ search }),
  setSort: (sort) => set({ sort }),
  setSelected: (selected, anchor) => set((s) => ({ selected, anchor: anchor === undefined ? s.anchor : anchor })),
}));
