import { useEffect } from "react";
import { events } from "@/lib/api";
import { useDownloads } from "@/stores/downloads";
import { useSettings } from "@/stores/settings";

/** Load state and keep it in sync with backend events (call once per window). */
export function useEngineSync() {
  const load = useDownloads((s) => s.load);
  const applyEvent = useDownloads((s) => s.applyEvent);
  const loadSettings = useSettings((s) => s.load);
  const replaceSettings = useSettings((s) => s.replace);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    // Subscribe first so no event between load and listen is lost.
    void events.engine(applyEvent).then((u) => (disposed ? u() : unlisteners.push(u)));
    void events.settingsChanged(replaceSettings).then((u) => (disposed ? u() : unlisteners.push(u)));
    void load();
    void loadSettings();
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [load, applyEvent, loadSettings, replaceSettings]);
}
