import { ArrowDown, BatteryLow, Gauge, Lock } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/menu";
import { errorMessage } from "@/lib/api";
import { formatSpeed } from "@/lib/format";
import { useDownloads } from "@/stores/downloads";
import { useQueues } from "@/stores/queues";
import { useSettings } from "@/stores/settings";

const LIMITS = [0, 256 * 1024, 512 * 1024, 1024 ** 2, 2 * 1024 ** 2, 5 * 1024 ** 2, 10 * 1024 ** 2];

export function StatusBar({ visibleCount }: { visibleCount: number }) {
  const { t } = useTranslation();
  const progress = useDownloads((s) => s.progress);
  const selected = useDownloads((s) => s.selected.length);
  const settings = useSettings((s) => s.settings);
  const info = useSettings((s) => s.info);
  const save = useSettings((s) => s.save);
  const hold = useQueues((s) => s.hold);

  const { active, speed } = useMemo(() => {
    const list = Object.values(progress);
    return { active: list.length, speed: list.reduce((a, p) => a + p.speed, 0) };
  }, [progress]);
  const limit = settings?.downloads.speed_limit ?? 0;

  const setLimit = (bytes: number) =>
    save((s) => {
      s.downloads.speed_limit = bytes;
      return s;
    }).catch((e) => toast.error(t("common.error"), { description: errorMessage(e) }));

  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-border bg-sidebar px-3 text-xs text-muted-foreground">
      <span className="tabular">
        {t("statusbar.items", { count: visibleCount })}
        {selected > 0 && ` · ${t("statusbar.selected", { count: selected })}`}
      </span>
      <span className="flex items-center gap-1 tabular">
        <ArrowDown className="size-3.5 text-primary" />
        {t("statusbar.active", { count: active })} · {formatSpeed(speed)}
      </span>
      <div className="flex-1" />
      {hold && (
        <span className="flex items-center gap-1 text-warning" role="status">
          <BatteryLow className="size-3.5" />
          {t(hold === "low_battery" ? "queues.holdBattery" : "queues.holdMetered")}
        </span>
      )}
      {info && !info.secure_storage && (
        <span className="flex items-center gap-1 text-warning" title={t("settings.network.noSecureStorage")}>
          <Lock className="size-3.5" />
          {t("settings.about.unavailable")}
        </span>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger className="flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-accent hover:text-foreground">
          <Gauge className="size-3.5" />
          {t("statusbar.limit")}: {limit ? formatSpeed(limit) : t("statusbar.noLimit")}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" side="top">
          {LIMITS.map((l) => (
            <DropdownMenuCheckboxItem key={l} checked={l === limit} onCheckedChange={() => void setLimit(l)}>
              {l ? formatSpeed(l) : t("statusbar.noLimit")}
            </DropdownMenuCheckboxItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </footer>
  );
}
