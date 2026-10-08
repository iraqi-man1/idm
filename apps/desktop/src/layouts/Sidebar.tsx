import {
  AlertCircle,
  BarChart3,
  CalendarClock,
  CheckCircle2,
  Download,
  Inbox,
  ListOrdered,
  PauseCircle,
  Settings,
} from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { Category } from "@/bindings/Category";
import { categoryIcon } from "@/components/FileIcon";
import { cn } from "@/lib/utils";
import { countByFilter, type ViewFilter } from "@/lib/view";
import { useDownloads } from "@/stores/downloads";
import { type Page, useUi } from "@/stores/ui";

const STATUS_ITEMS: Array<{ f: ViewFilter; key: string; icon: typeof Inbox }> = [
  { f: "all", key: "nav.all", icon: Inbox },
  { f: "downloading", key: "nav.downloading", icon: Download },
  { f: "completed", key: "nav.completed", icon: CheckCircle2 },
  { f: "paused", key: "nav.paused", icon: PauseCircle },
  { f: "queued", key: "nav.queued", icon: ListOrdered },
  { f: "scheduled", key: "nav.scheduled", icon: CalendarClock },
  { f: "failed", key: "nav.failed", icon: AlertCircle },
];

const CATEGORIES: Category[] = ["video", "document", "music", "archive", "program", "image", "other"];

function NavButton({
  active,
  icon: Icon,
  label,
  count,
  onClick,
  tone,
}: {
  active: boolean;
  icon: typeof Inbox;
  label: string;
  count?: number;
  onClick: () => void;
  tone?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "group relative flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors",
        active ? "bg-surface font-medium text-foreground shadow-card" : "text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      {active && <span className="absolute inset-y-1.5 start-0 w-[3px] rounded-full bg-primary" />}
      <Icon className={cn("size-4 shrink-0", active ? "text-primary" : tone)} />
      <span className="truncate">{label}</span>
      {count !== undefined && count > 0 && (
        <span className="ms-auto rounded-full bg-muted px-1.5 text-[11px] tabular text-muted-foreground">{count}</span>
      )}
    </button>
  );
}

export function Sidebar() {
  const { t } = useTranslation();
  const items = useDownloads((s) => s.items);
  const filter = useDownloads((s) => s.filter);
  const setFilter = useDownloads((s) => s.setFilter);
  const page = useUi((s) => s.page);
  const setPage = useUi((s) => s.setPage);

  const counts = useMemo(() => {
    const filters: ViewFilter[] = [...STATUS_ITEMS.map((s) => s.f), ...CATEGORIES.map((c) => `cat:${c}` as ViewFilter)];
    return countByFilter(Object.values(items), filters);
  }, [items]);

  const go = (f: ViewFilter) => {
    setFilter(f);
    setPage("downloads");
  };
  const pageBtn = (p: Page, icon: typeof Inbox, label: string) => (
    <NavButton active={page === p} icon={icon} label={label} onClick={() => setPage(p)} />
  );

  return (
    <aside className="flex w-56 shrink-0 flex-col border-e border-border bg-sidebar">
      <div className="flex h-12 items-center gap-2 px-4" data-tauri-drag-region>
        <img src="/app-icon.svg" alt="" className="size-6" draggable={false} />
        <span className="text-[14px] font-semibold tracking-tight">{t("app.short")}</span>
      </div>
      <nav className="flex-1 space-y-4 overflow-y-auto px-2.5 pb-3">
        <div className="space-y-0.5">
          {STATUS_ITEMS.map(({ f, key, icon }) => (
            <NavButton
              key={f}
              active={page === "downloads" && filter === f}
              icon={icon}
              label={t(key)}
              count={f === "all" || f === "completed" ? undefined : counts[f]}
              onClick={() => go(f)}
            />
          ))}
        </div>
        <div>
          <div className="px-2.5 pb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground/80">
            {t("nav.categories")}
          </div>
          <div className="space-y-0.5">
            {CATEGORIES.map((c) => (
              <NavButton
                key={c}
                active={page === "downloads" && filter === `cat:${c}`}
                icon={categoryIcon(c)}
                label={t(`nav.${c}`)}
                count={counts[`cat:${c}`]}
                onClick={() => go(`cat:${c}`)}
              />
            ))}
          </div>
        </div>
      </nav>
      <div className="space-y-0.5 border-t border-border p-2.5">
        {pageBtn("statistics", BarChart3, t("nav.statistics"))}
        {pageBtn("settings", Settings, t("nav.settings"))}
      </div>
    </aside>
  );
}
