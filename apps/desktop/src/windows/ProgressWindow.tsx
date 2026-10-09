import { getCurrentWindow } from "@tauri-apps/api/window";
import { ExternalLink, FolderOpen, Pause, Play, Square, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { LogLine } from "@/bindings/LogLine";
import { FileIcon } from "@/components/FileIcon";
import { SegmentMap } from "@/components/SegmentMap";
import { SpeedGraph } from "@/components/SpeedGraph";
import { StatusLabel, statusTone } from "@/components/StatusLabel";
import { Button } from "@/components/ui/button";
import { ProgressBar } from "@/components/ui/progress-bar";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { capabilities } from "@/hooks/useActions";
import { useAppearance } from "@/hooks/useAppearance";
import { useEngineSync } from "@/hooks/useEngine";
import { api, errorMessage } from "@/lib/api";
import { formatBytes, formatDuration, formatPercent, formatSpeed } from "@/lib/format";
import { cn } from "@/lib/utils";
import { fraction, isActive, mergeRow } from "@/lib/view";
import { useDownloads } from "@/stores/downloads";

function Stat({ label, value, strong }: { label: string; value: React.ReactNode; strong?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1 text-[13px]">
      <span className="text-muted-foreground">{label}</span>
      <span className={cn("tabular text-end", strong && "font-semibold")}>{value}</span>
    </div>
  );
}

export function ProgressWindow({ id }: { id: string }) {
  useEngineSync();
  useAppearance();
  const { t } = useTranslation();
  const info = useDownloads((s) => s.items[id]);
  const prog = useDownloads((s) => s.progress[id]);
  const loaded = useDownloads((s) => s.loaded);
  const [history, setHistory] = useState<number[]>([]);
  const [log, setLog] = useState<LogLine[]>([]);
  const [tab, setTab] = useState("details");
  const lastSample = useRef(0);

  useEffect(() => {
    void api.speedHistory(id).then(setHistory).catch(() => undefined);
  }, [id]);

  // One sample per second from live progress.
  useEffect(() => {
    if (!prog) return;
    const now = Date.now();
    if (now - lastSample.current >= 950) {
      lastSample.current = now;
      setHistory((h) => [...h.slice(-299), prog.speed]);
    }
  }, [prog]);

  useEffect(() => {
    if (tab !== "log") return;
    const load = () => void api.log(id).then(setLog).catch(() => undefined);
    load();
    const timer = setInterval(load, 2000);
    return () => clearInterval(timer);
  }, [tab, id]);

  const row = useMemo(() => (info ? mergeRow(info, prog) : null), [info, prog]);

  useEffect(() => {
    if (row) {
      const f = fraction(row);
      const pct = f === null ? "" : `${Math.floor(f * 100)}% · `;
      void getCurrentWindow().setTitle(`${pct}${row.file_name}`);
    }
  }, [row]);

  if (!row) {
    return (
      <div className="flex h-full items-center justify-center text-[13px] text-muted-foreground">
        {loaded ? t("progress.notFound") : t("common.loading")}
      </div>
    );
  }

  const f = fraction(row);
  const active = isActive(row.status);
  const caps = capabilities(row);
  const act = (label: string, fn: () => Promise<unknown>) => fn().catch((e) => toast.error(label, { description: errorMessage(e) }));
  const segments = row.segments;

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex items-start gap-3 border-b border-border bg-surface p-4">
        <FileIcon category={row.category} className="size-10" />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-[14px] font-semibold" dir="auto" title={row.file_name}>
            {row.file_name}
          </h1>
          <p className="truncate font-mono text-[11px] text-muted-foreground" dir="ltr" title={row.url} data-selectable>
            {row.url}
          </p>
          <div className="mt-2 flex items-center gap-3">
            <ProgressBar value={f === null && active ? null : f ?? 0} tone={statusTone(row.status)} animated={row.status === "downloading"} className="h-2 flex-1" />
            <span className="w-14 text-end text-[13px] font-semibold tabular">{f === null ? "" : formatPercent(f)}</span>
          </div>
        </div>
      </div>

      <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
        <TabsList className="px-3">
          <TabsTrigger value="details">{t("progress.details")}</TabsTrigger>
          <TabsTrigger value="connections">
            {t("progress.segments")} {active && <span className="ms-1 tabular text-muted-foreground">({row.active_connections})</span>}
          </TabsTrigger>
          <TabsTrigger value="log">{t("progress.log")}</TabsTrigger>
        </TabsList>

        <TabsContent value="details" className="min-h-0 flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-2 gap-x-8">
            <div className="divide-y divide-border/60">
              <Stat label={t("progress.status")} value={<StatusLabel status={row.status} errorKind={row.error_kind} className="justify-end" />} />
              <Stat label={t("progress.fileSize")} value={formatBytes(row.total_size)} />
              <Stat label={t("progress.downloaded")} value={formatBytes(row.downloaded)} strong />
              <Stat label={t("progress.resumable")} value={row.resumable === null ? t("common.unknown") : row.resumable ? t("common.yes") : t("common.no")} />
            </div>
            <div className="divide-y divide-border/60">
              <Stat label={t("progress.rate")} value={active ? formatSpeed(row.speed) : "—"} strong />
              <Stat label={t("progress.average")} value={formatSpeed(row.avg_speed)} />
              <Stat label={t("progress.timeLeft")} value={active ? formatDuration(row.eta_secs) : "—"} />
              <Stat label={t("progress.elapsed")} value={formatDuration(row.elapsed_ms / 1000)} />
            </div>
          </div>
          {row.error && <p className="mt-3 rounded-md bg-danger-soft px-3 py-2 text-xs text-danger">{row.error}</p>}
          {row.stage && <p className="mt-3 text-xs text-muted-foreground">{row.stage}</p>}
          <div className="mt-4">
            <div className="mb-1.5 flex items-center justify-between text-xs text-muted-foreground">
              <span>{t("progress.segmentMap")}</span>
              <span className="tabular">{t("progress.connections")}: {row.active_connections}/{row.max_connections}</span>
            </div>
            {row.total_size ? (
              <SegmentMap total={row.total_size} segments={segments} />
            ) : (
              <p className="text-xs text-muted-foreground">{t("progress.unknownSize")}</p>
            )}
          </div>
          <div className="mt-4">
            <div className="mb-1.5 text-xs text-muted-foreground">{t("progress.graph")}</div>
            <div className="rounded-lg border border-border bg-surface p-2">
              <SpeedGraph samples={history} height={96} />
            </div>
          </div>
        </TabsContent>

        <TabsContent value="connections" className="min-h-0 flex-1 overflow-y-auto">
          {segments.length === 0 ? (
            <p className="p-4 text-[13px] text-muted-foreground">{t("progress.noConnections")}</p>
          ) : (
            <table className="w-full text-[12px]">
              <thead className="sticky top-0 bg-surface-2 text-muted-foreground">
                <tr className="text-start">
                  <th className="px-3 py-1.5 text-start font-medium">{t("progress.connection")}</th>
                  <th className="px-3 py-1.5 text-start font-medium">{t("progress.range")}</th>
                  <th className="px-3 py-1.5 text-end font-medium">{t("progress.received")}</th>
                  <th className="px-3 py-1.5 text-end font-medium">{t("progress.speed")}</th>
                  <th className="px-3 py-1.5 text-start font-medium">{t("progress.state")}</th>
                </tr>
              </thead>
              <tbody>
                {segments.map((s, i) => {
                  const len = s.end === null ? null : s.end - s.start;
                  const pct = len ? s.written / len : null;
                  return (
                    <tr key={`${s.index}-${s.start}`} className="border-b border-border/60">
                      <td className="px-3 py-1.5 tabular text-muted-foreground">{i + 1}</td>
                      <td className="px-3 py-1.5 tabular" dir="ltr">
                        {formatBytes(s.start)} – {s.end === null ? "…" : formatBytes(s.end)}
                      </td>
                      <td className="px-3 py-1.5 text-end tabular">
                        {formatBytes(s.written)}
                        {pct !== null && <span className="ms-1 text-muted-foreground">({formatPercent(pct)})</span>}
                      </td>
                      <td className="px-3 py-1.5 text-end tabular">{s.active ? formatSpeed(s.speed) : ""}</td>
                      <td className={cn("px-3 py-1.5", s.done ? "text-success" : s.active ? "text-primary" : "text-muted-foreground")}>
                        {s.done ? t("progress.done") : s.active ? t("progress.receiving") : t("progress.idle")}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </TabsContent>

        <TabsContent value="log" className="min-h-0 flex-1 overflow-y-auto p-3">
          <div className="font-mono text-[11px] leading-5" dir="ltr" data-selectable>
            {log.map((l, i) => (
              <div key={i} className={cn(l.level === "error" && "text-danger", l.level === "warn" && "text-warning")}>
                <span className="text-muted-foreground">{new Date(l.ts).toLocaleTimeString()} </span>
                {l.message}
              </div>
            ))}
          </div>
        </TabsContent>
      </Tabs>

      <div className="flex items-center gap-2 border-t border-border bg-surface px-4 py-3">
        {row.status === "completed" ? (
          <>
            <Button onClick={() => act(t("progress.openFile"), () => api.openFile(id))}>
              <ExternalLink /> {t("progress.openFile")}
            </Button>
            <Button variant="secondary" onClick={() => act(t("progress.showInFolder"), () => api.openFolder(id))}>
              <FolderOpen /> {t("progress.showInFolder")}
            </Button>
          </>
        ) : (
          <>
            {caps.canPause && (
              <Button variant="secondary" onClick={() => act(t("actions.pause"), () => api.pause(id))}>
                <Pause /> {t("actions.pause")}
              </Button>
            )}
            {caps.canResume && (
              <Button onClick={() => act(t("actions.resume"), () => api.start(id))}>
                <Play /> {t("actions.resume")}
              </Button>
            )}
            {caps.canCancel && (
              <Button variant="ghost" className="text-danger" onClick={() => act(t("actions.cancel"), () => api.cancel(id))}>
                <Square /> {t("actions.cancel")}
              </Button>
            )}
          </>
        )}
        <div className="flex-1" />
        <Button variant="ghost" onClick={() => void getCurrentWindow().close()}>
          <X /> {t("progress.hide")}
        </Button>
      </div>
    </div>
  );
}
