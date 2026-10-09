import { BatteryLow, ListOrdered, Play, Plus, Square, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { PostAction } from "@/bindings/PostAction";
import type { QueueInfo } from "@/bindings/QueueInfo";
import type { QueueUpdate } from "@/bindings/QueueUpdate";
import type { Schedule } from "@/bindings/Schedule";
import { Button } from "@/components/ui/button";
import { Field, Section } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { LazyInput, NumberInput } from "@/components/ui/lazy-input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api, errorMessage } from "@/lib/api";
import { cn } from "@/lib/utils";
import { countByFilter, type ViewFilter } from "@/lib/view";
import { useDownloads } from "@/stores/downloads";
import { useQueues } from "@/stores/queues";
import { useUi } from "@/stores/ui";

const POST_ACTIONS: PostAction[] = ["none", "exit", "sleep", "hibernate", "shutdown"];
// 2026-10-04 is a Sunday: day i of the week is 2026-10-(04+i).
const weekday = (lang: string, i: number) =>
  new Intl.DateTimeFormat(lang, { weekday: "short" }).format(new Date(2026, 9, 4 + i, 12));

/** Time of day; saved as soon as a complete time (or nothing) is entered. */
function TimeInput({ value, onCommit, label }: { value: string | null; onCommit: (v: string | null) => void; label: string }) {
  return (
    <Input
      type="time"
      className="w-32"
      aria-label={label}
      value={value ?? ""}
      onChange={(e) => {
        const v = e.target.value;
        if (v === "" || /^\d{2}:\d{2}$/.test(v)) onCommit(v || null);
      }}
    />
  );
}

export function queueName(q: QueueInfo, t: (k: string) => string) {
  return q.built_in ? t("queues.main") : q.name;
}

function QueueEditor({ q }: { q: QueueInfo }) {
  const { t, i18n } = useTranslation();
  const items = useDownloads((s) => s.items);
  const setFilter = useDownloads((s) => s.setFilter);
  const setPage = useUi((s) => s.setPage);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const pending = useMemo(() => countByFilter(Object.values(items), [`queue:${q.id}` as ViewFilter])[`queue:${q.id}`] ?? 0, [items, q.id]);

  const run = (p: Promise<unknown>) => p.catch((e) => toast.error(t("common.error"), { description: errorMessage(e) }));
  const update = (u: Partial<QueueUpdate>) => run(api.updateQueue(q.id, u).then(useQueues.getState().upsert));
  const setSchedule = (patch: Partial<Schedule>) => {
    // Build on the latest state: edits can follow each other faster than
    // the change events arrive.
    const latest = useQueues.getState().queues.find((x) => x.id === q.id) ?? q;
    const next: Schedule = { ...latest.schedule, ...patch };
    // Enabling needs at least one time; offer a sensible default.
    if (next.enabled && !next.start_time && !next.stop_time) next.start_time = "02:00";
    void update({ schedule: next });
  };
  const s = q.schedule;

  return (
    <div className="space-y-4" data-testid="queue-editor">
      <div className="flex items-center gap-3">
        <h2 className="min-w-0 flex-1 truncate text-lg font-semibold">{queueName(q, t)}</h2>
        <span className={cn("rounded-full px-2 py-0.5 text-xs", q.running ? "bg-success-soft text-success" : "bg-muted text-muted-foreground")}>
          {q.running ? t("queues.running") : t("queues.stopped")}
        </span>
        {q.running ? (
          <Button variant="secondary" onClick={() => void run(api.stopQueue(q.id))}>
            <Square /> {t("queues.stop")}
          </Button>
        ) : (
          <Button onClick={() => void run(api.startQueue(q.id))}>
            <Play /> {t("queues.start")}
          </Button>
        )}
      </div>
      <Section>
        {!q.built_in && (
          <Field label={t("queues.name")}>
            <LazyInput className="w-56" dir="auto" value={q.name} onCommit={(v) => void update({ name: v })} />
          </Field>
        )}
        <Field label={t("queues.maxConcurrent")}>
          <NumberInput value={q.max_concurrent} min={1} max={32} onCommit={(v) => void update({ max_concurrent: v })} />
        </Field>
        <Field label={t("queues.retryFailed")}>
          <Switch checked={q.retry_failed} onCheckedChange={(v) => void update({ retry_failed: v })} />
        </Field>
        <Field label={t("queues.postAction")} hint={q.post_action !== "none" ? t("queues.postActionHint") : undefined}>
          <Select
            value={q.post_action}
            className="min-w-48"
            ariaLabel={t("queues.postAction")}
            onChange={(v) => void update({ post_action: v })}
            options={POST_ACTIONS.map((a) => ({ value: a, label: t(`queues.post_${a}`) }))}
          />
        </Field>
        <Field label={t("queues.downloads", { count: pending })}>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setFilter(`queue:${q.id}`);
              setPage("downloads");
            }}
          >
            <ListOrdered /> {t("queues.showDownloads")}
          </Button>
        </Field>
      </Section>
      <Section title={t("queues.schedule")}>
        <Field label={t("queues.scheduleEnabled")}>
          <Switch checked={s.enabled} onCheckedChange={(v) => setSchedule({ enabled: v })} aria-label={t("queues.scheduleEnabled")} />
        </Field>
        <Field label={t("queues.startTime")} hint={t("queues.timeHint")}>
          <TimeInput label={t("queues.startTime")} value={s.start_time} onCommit={(v) => setSchedule({ start_time: v })} />
        </Field>
        <Field label={t("queues.stopTime")} hint={t("queues.timeHint")}>
          <TimeInput label={t("queues.stopTime")} value={s.stop_time} onCommit={(v) => setSchedule({ stop_time: v })} />
        </Field>
        <Field label={t("queues.days")} hint={s.days.length === 0 ? t("queues.everyDay") : undefined}>
          <div className="flex gap-1" role="group" aria-label={t("queues.days")}>
            {[0, 1, 2, 3, 4, 5, 6].map((d) => {
              const on = s.days.includes(d);
              return (
                <button
                  key={d}
                  type="button"
                  aria-pressed={on}
                  onClick={() => setSchedule({ days: on ? s.days.filter((x) => x !== d) : [...s.days, d].sort() })}
                  className={cn(
                    "h-7 min-w-10 rounded-md border px-1.5 text-xs",
                    on ? "border-primary bg-primary text-primary-foreground" : "border-border-strong hover:bg-accent",
                  )}
                >
                  {weekday(i18n.language, d)}
                </button>
              );
            })}
          </div>
        </Field>
      </Section>
      {!q.built_in && (
        <div className="flex justify-end">
          <Button
            variant={confirmDelete ? "danger" : "ghost"}
            onClick={() => {
              if (!confirmDelete) setConfirmDelete(true);
              else void run(api.deleteQueue(q.id));
            }}
            onBlur={() => setConfirmDelete(false)}
          >
            <Trash2 /> {confirmDelete ? t("queues.confirmDelete") : t("queues.delete")}
          </Button>
        </div>
      )}
    </div>
  );
}

export function QueuesPage() {
  const { t } = useTranslation();
  const queues = useQueues((s) => s.queues);
  const hold = useQueues((s) => s.hold);
  const [selected, setSelected] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const current = queues.find((q) => q.id === selected) ?? queues[0];

  const create = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      const q = await api.createQueue(name);
      setNewName("");
      setSelected(q.id);
    } catch (e) {
      toast.error(t("common.error"), { description: errorMessage(e) });
    }
  };

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex w-56 shrink-0 flex-col border-e border-border bg-surface-2">
        <h1 className="px-4 pb-2 pt-4 text-[15px] font-semibold">{t("queues.title")}</h1>
        <div className="flex-1 space-y-0.5 overflow-y-auto px-2.5">
          {queues.map((q) => (
            <button
              key={q.id}
              onClick={() => setSelected(q.id)}
              className={cn(
                "flex h-8 w-full items-center gap-2 rounded-md px-2.5 text-start text-[13px]",
                q.id === current?.id ? "bg-primary-soft font-medium text-primary" : "hover:bg-accent",
              )}
            >
              <span className={cn("size-2 shrink-0 rounded-full", q.running ? "bg-success" : "bg-muted-foreground/40")} />
              <span className="truncate">{queueName(q, t)}</span>
            </button>
          ))}
        </div>
        <form
          className="flex gap-1.5 border-t border-border p-2.5"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <Input value={newName} dir="auto" placeholder={t("queues.newPlaceholder")} onChange={(e) => setNewName(e.target.value)} />
          <Button type="submit" size="icon" variant="secondary" aria-label={t("queues.create")} disabled={!newName.trim()}>
            <Plus />
          </Button>
        </form>
      </div>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl space-y-4 p-6">
          {hold && (
            <div className="flex items-center gap-2 rounded-md bg-warning-soft px-3 py-2 text-xs text-warning" role="status">
              <BatteryLow className="size-4" />
              {t(hold === "low_battery" ? "queues.holdBattery" : "queues.holdMetered")}
            </div>
          )}
          {current && <QueueEditor key={current.id} q={current} />}
        </div>
      </div>
    </div>
  );
}
