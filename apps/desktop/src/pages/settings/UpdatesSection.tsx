import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { Loader2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppSettings } from "@/bindings/AppSettings";
import { Button } from "@/components/ui/button";
import { Field, Section } from "@/components/ui/field";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/api";
import { formatPercent } from "@/lib/format";
import { useSettings } from "@/stores/settings";

export function UpdatesSection({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const info = useSettings((x) => x.info);
  const save = useSettings((x) => x.save);
  const [state, setState] = useState<"idle" | "checking" | "none" | "available" | "installing">("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [progress, setProgress] = useState<number | null>(null);

  const doCheck = async () => {
    setState("checking");
    try {
      const u = await check();
      setUpdate(u);
      setState(u ? "available" : "none");
    } catch (e) {
      setState("idle");
      toast.error(t("settings.updates.check"), { description: errorMessage(e) });
    }
  };

  const install = async () => {
    if (!update) return;
    setState("installing");
    let total = 0;
    let done = 0;
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        if (ev.event === "Progress") {
          done += ev.data.chunkLength;
          setProgress(total ? done / total : null);
        }
      });
      await relaunch();
    } catch (e) {
      setState("available");
      toast.error(t("settings.updates.install"), { description: errorMessage(e) });
    }
  };

  return (
    <Section>
      <Field label={t("settings.updates.installed")}>
        <span className="tabular">{info?.version}</span>
      </Field>
      {info?.updater_configured ? (
        <>
          <Field label={t("settings.updates.autoCheck")}>
            <Switch
              checked={s.updates.auto_check}
              onCheckedChange={(v) => void save((x) => ({ ...x, updates: { auto_check: v } }))}
            />
          </Field>
          <Field
            label={t("settings.updates.check")}
            hint={
              state === "none"
                ? t("settings.updates.upToDate")
                : state === "available" && update
                  ? t("settings.updates.available", { version: update.version })
                  : state === "installing"
                    ? `${t("settings.updates.installing")} ${progress !== null ? formatPercent(progress) : ""}`
                    : undefined
            }
          >
            {state === "available" ? (
              <Button onClick={install}>{t("settings.updates.install")}</Button>
            ) : (
              <Button variant="secondary" disabled={state === "checking" || state === "installing"} onClick={doCheck}>
                {(state === "checking" || state === "installing") && <Loader2 className="animate-spin" />}
                {state === "checking" ? t("settings.updates.checking") : t("settings.updates.check")}
              </Button>
            )}
          </Field>
        </>
      ) : (
        <p className="py-3 text-[13px] text-muted-foreground">{t("settings.updates.notConfigured")}</p>
      )}
    </Section>
  );
}
