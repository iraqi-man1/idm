import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppSettings } from "@/bindings/AppSettings";
import { Field, Section } from "@/components/ui/field";
import { Textarea } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/api";
import { formatBytes } from "@/lib/format";
import { useSettings } from "@/stores/settings";

function LazyTextarea({ value, onCommit, rows = 3 }: { value: string; onCommit: (v: string) => void; rows?: number }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <Textarea
      dir="ltr"
      rows={rows}
      value={v}
      className="font-mono text-xs"
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
    />
  );
}

export function BrowserSection({ s }: { s: AppSettings }) {
  const { t } = useTranslation();
  const save = useSettings((x) => x.save);
  const b = s.browser;
  const set = <K extends keyof AppSettings["browser"]>(k: K, v: AppSettings["browser"][K]) =>
    save((x) => ({ ...x, browser: { ...x.browser, [k]: v } })).catch((e) =>
      toast.error(t("common.error"), { description: errorMessage(e) }),
    );
  return (
    <div className="space-y-4">
      <Section>
        <Field label={t("settings.browser.capture")}>
          <Switch checked={b.capture_downloads} onCheckedChange={(v) => set("capture_downloads", v)} />
        </Field>
        <Field label={t("settings.browser.extensions")} hint={t("settings.browser.extensionsHint")} stacked>
          <LazyTextarea
            value={b.capture_extensions.join(" ")}
            onCommit={(v) => set("capture_extensions", v.split(/[\s,;]+/).filter(Boolean))}
          />
        </Field>
        <Field label={t("settings.browser.minSize")}>
          <Select
            value={String(b.min_capture_size)}
            onChange={(v) => set("min_capture_size", Number(v))}
            options={[0, 64 * 1024, 512 * 1024, 1024 ** 2, 5 * 1024 ** 2, 20 * 1024 ** 2].map((n) => ({
              value: String(n),
              label: n ? formatBytes(n) : t("common.none"),
            }))}
          />
        </Field>
        <Field label={t("settings.browser.excluded")} hint={t("settings.browser.excludedHint")} stacked>
          <LazyTextarea
            value={b.excluded_sites.join("\n")}
            onCommit={(v) => set("excluded_sites", v.split(/[\s,]+/).map((x) => x.trim().toLowerCase()).filter(Boolean))}
          />
        </Field>
      </Section>
      <Section>
        <Field label={t("settings.browser.videoDetection")}>
          <Switch checked={b.video_detection} onCheckedChange={(v) => set("video_detection", v)} />
        </Field>
        <Field label={t("settings.browser.floatingButton")}>
          <Switch checked={b.floating_button} disabled={!b.video_detection} onCheckedChange={(v) => set("floating_button", v)} />
        </Field>
      </Section>
    </div>
  );
}
