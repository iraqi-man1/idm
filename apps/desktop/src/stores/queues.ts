import { create } from "zustand";
import type { PowerHold } from "@/bindings/PowerHold";
import type { QueueInfo } from "@/bindings/QueueInfo";
import type { SchedulerEvent } from "@/bindings/SchedulerEvent";
import { api } from "@/lib/api";

interface QueuesState {
  queues: QueueInfo[];
  hold: PowerHold | null;
  load: () => Promise<void>;
  apply: (e: SchedulerEvent) => void;
  /** Apply a queue returned by a command (before the change event arrives). */
  upsert: (q: QueueInfo) => void;
}

export const useQueues = create<QueuesState>((set) => ({
  queues: [],
  hold: null,
  load: async () => {
    const [queues, hold] = await Promise.all([api.queues(), api.powerHold()]);
    set({ queues, hold });
  },
  apply: (e) => {
    if (e.type === "queues_changed") set({ queues: e.queues });
    if (e.type === "power_hold") set({ hold: e.hold });
  },
  upsert: (q) => set((s) => ({ queues: s.queues.map((x) => (x.id === q.id ? q : x)) })),
}));
