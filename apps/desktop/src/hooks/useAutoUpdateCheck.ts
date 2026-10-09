import { check } from "@tauri-apps/plugin-updater";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { AUTO_CHECK_DELAY_MS, autoCheckDue } from "@/lib/updates";
import { useSettings } from "@/stores/settings";
import { useUi } from "@/stores/ui";

/**
 * "Check for updates automatically": checks shortly after start and then
 * daily, and offers a found update in a notification. Installing stays a
 * user action in Settings → Updates.
 */
export function useAutoUpdateCheck() {
  const { t } = useTranslation();
  const configured = useSettings((x) => x.info?.updater_configured ?? false);
  const enabled = useSettings((x) => x.settings?.updates.auto_check ?? false);
  const setPage = useUi((x) => x.setPage);
  const lastCheck = useRef<number | null>(null);
  const announced = useRef<string | null>(null);

  useEffect(() => {
    if (!configured || !enabled) return;
    let active = true;
    const run = async () => {
      if (!autoCheckDue({ configured, enabled, lastCheck: lastCheck.current, now: Date.now() })) return;
      lastCheck.current = Date.now();
      try {
        const update = await check();
        if (!active || !update || announced.current === update.version) return;
        announced.current = update.version;
        toast(t("settings.updates.available", { version: update.version }), {
          duration: Infinity,
          action: { label: t("settings.updates.open"), onClick: () => setPage("settings", "updates") },
        });
      } catch (e) {
        // Background checks stay quiet; the manual check reports errors.
        console.warn("automatic update check failed", e);
      }
    };
    const first = setTimeout(run, AUTO_CHECK_DELAY_MS);
    // Hourly wake-ups; autoCheckDue keeps the actual checks a day apart.
    const hourly = setInterval(run, 60 * 60 * 1000);
    return () => {
      active = false;
      clearTimeout(first);
      clearInterval(hourly);
    };
  }, [configured, enabled, setPage, t]);
}
