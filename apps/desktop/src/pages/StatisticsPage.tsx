import { Activity, CalendarDays, CheckCircle2, Gauge, HardDriveDownload, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { StatsSummary } from "@/bindings/StatsSummary";
import { FileIcon } from "@/components/FileIcon";
import { api } from "@/lib/api";
import { formatBytes, formatNumber, formatSpeed } from "@/lib/format";

function StatCard({ icon: Icon, label, value, tone = "text-primary" }: { icon: typeof Activity; label: string; value: string; tone?: string }) {
  return (
    <div className="rounded-xl border border-border bg-surface p-4 shadow-card">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className={`size-4 ${tone}`} />
        {label}
      </div>
      <div className="mt-2 text-xl font-semibold tabular">{value}</div>
    </div>
  );
}

function Bars({ data, labelEvery = 1 }: { data: Array<{ label: string; value: number; title: string }>; labelEvery?: number }) {
  const max = Math.max(1, ...data.map((d) => d.value));
  return (
    <div className="flex h-44 items-end gap-[3px]" dir="ltr">
      {data.map((d, i) => (
        <div key={d.label + i} className="group flex h-full flex-1 flex-col items-center justify-end gap-1" title={d.title}>
          <div
            className="w-full rounded-t-[3px] bg-primary/80 transition-colors group-hover:bg-primary"
            style={{ height: `${Math.max(d.value > 0 ? 2 : 0, (d.value / max) * 100)}%` }}
          />
          <span className="h-3 text-[10px] leading-3 text-muted-foreground">{i % labelEvery === 0 ? d.label : ""}</span>
        </div>
      ))}
    </div>
  );
}

export function StatisticsPage() {
  const { t } = useTranslation();
  const [stats, setStats] = useState<StatsSummary | null>(null);
  useEffect(() => {
    const load = () => void api.stats().then(setStats).catch(() => undefined);
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  }, []);
  if (!stats) return <div className="p-6 text-muted-foreground">{t("common.loading")}</div>;
  const totalCat = Math.max(1, ...stats.by_category.map((c) => c.bytes));
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-5xl space-y-5 p-6">
        <h1 className="text-lg font-semibold">{t("stats.title")}</h1>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-3 xl:grid-cols-6">
          <StatCard icon={Activity} label={t("stats.today")} value={formatBytes(stats.today_bytes)} />
          <StatCard icon={CalendarDays} label={t("stats.month")} value={formatBytes(stats.month_bytes)} />
          <StatCard icon={HardDriveDownload} label={t("stats.total")} value={formatBytes(stats.total_bytes)} />
          <StatCard icon={CheckCircle2} label={t("stats.completed")} value={formatNumber(stats.total_completed)} tone="text-success" />
          <StatCard icon={XCircle} label={t("stats.failed")} value={formatNumber(stats.total_failed)} tone="text-danger" />
          <StatCard icon={Gauge} label={t("stats.bestSpeed")} value={formatSpeed(stats.best_avg_speed)} />
        </div>
        <div className="grid gap-4 lg:grid-cols-2">
          <section className="rounded-xl border border-border bg-surface p-4 shadow-card">
            <h2 className="mb-3 text-[13px] font-semibold">{t("stats.daily")}</h2>
            <Bars
              labelEvery={5}
              data={stats.daily.map((d) => ({ label: d.day.slice(8), value: d.bytes, title: `${d.day}: ${formatBytes(d.bytes)} · ${t("stats.files", { count: d.files })}` }))}
            />
          </section>
          <section className="rounded-xl border border-border bg-surface p-4 shadow-card">
            <h2 className="mb-3 text-[13px] font-semibold">{t("stats.monthly")}</h2>
            <Bars
              data={stats.monthly.map((m) => ({ label: m.month.slice(5), value: m.bytes, title: `${m.month}: ${formatBytes(m.bytes)} · ${t("stats.files", { count: m.files })}` }))}
            />
          </section>
        </div>
        <section className="rounded-xl border border-border bg-surface p-4 shadow-card">
          <h2 className="mb-3 text-[13px] font-semibold">{t("stats.byCategory")}</h2>
          <div className="space-y-2.5">
            {stats.by_category.length === 0 && <p className="text-[13px] text-muted-foreground">—</p>}
            {[...stats.by_category]
              .sort((a, b) => b.bytes - a.bytes)
              .map((c) => (
                <div key={c.category} className="flex items-center gap-3 text-[13px]">
                  <FileIcon category={c.category} />
                  <span className="w-28">{t(`nav.${c.category}`)}</span>
                  <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
                    <div className="h-full rounded-full bg-primary" style={{ width: `${(c.bytes / totalCat) * 100}%` }} />
                  </div>
                  <span className="w-40 text-end tabular text-muted-foreground">
                    {formatBytes(c.bytes)} · {t("stats.files", { count: c.files })}
                  </span>
                </div>
              ))}
          </div>
        </section>
      </div>
    </div>
  );
}
