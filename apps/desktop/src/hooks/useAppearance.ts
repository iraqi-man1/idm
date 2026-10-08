import { useEffect } from "react";
import { applyLanguage } from "@/i18n";
import { useSettings } from "@/stores/settings";

/** Apply theme (light/dark/system) and language/direction from settings. */
export function useAppearance() {
  const theme = useSettings((s) => s.settings?.appearance.theme ?? "system");
  const language = useSettings((s) => s.settings?.appearance.language ?? "system");

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && mq.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme]);

  useEffect(() => {
    applyLanguage(language);
  }, [language]);
}
