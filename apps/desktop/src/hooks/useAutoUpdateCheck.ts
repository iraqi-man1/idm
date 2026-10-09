import { check } from "@tauri-apps/plugin-updater";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { AUTO_CHECK_DELAY_MS, AUTO_CHECK_WAKE_MS, createAutoChecker, type FoundUpdate } from "@/lib/updates";
import { useSettings } from "@/stores/settings";
import { useUi } from "@/stores/ui";

/**
 * "Check for updates automatically": checks shortly after start and then
 * daily (failed checks are retried hourly), and offers a found update in a
 * notification. Installing stays a user action in Settings → Updates.
 */
export function useAutoUpdateCheck() {
  const { t } = useTranslation();
  const configured = useSettings((x) => x.info?.updater_configured ?? false);
  const enabled = useSettings((x) => x.settings?.updates.auto_check ?? false);
  const setPage = useUi((x) => x.setPage);

  const notify = useRef<(u: FoundUpdate) => void>(() => {});
  notify.current = (u) =>
    toast(t("settings.updates.available", { version: u.version }), {
      duration: Infinity,
      action: { label: t("settings.updates.open"), onClick: () => setPage("settings", "updates") },
    });
  const checker = useRef<ReturnType<typeof createAutoChecker> | null>(null);
  if (!checker.current) {
    checker.current = createAutoChecker({
      check,
      notify: (u) => notify.current(u),
      // Background checks stay quiet; the manual check reports errors.
      onError: (e) => console.warn("automatic update check failed", e),
      now: () => Date.now(),
    });
  }

  useEffect(() => {
    if (!configured || !enabled) return;
    const tick = () => void checker.current?.({ configured, enabled });
    const first = setTimeout(tick, AUTO_CHECK_DELAY_MS);
    const wake = setInterval(tick, AUTO_CHECK_WAKE_MS);
    return () => {
      clearTimeout(first);
      clearInterval(wake);
    };
  }, [configured, enabled]);
}
