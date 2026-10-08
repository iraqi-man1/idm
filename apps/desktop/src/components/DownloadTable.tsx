import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowDown,
  ArrowUp,
  CheckSquare,
  Copy,
  FileCheck2,
  FolderInput,
  FolderOpen,
  Gauge,
  Info,
  Link2,
  Pause,
  Pencil,
  Play,
  RotateCcw,
  Square,
  Trash2,
  ExternalLink,
  DownloadCloud,
  SearchX,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import { FileIcon } from "@/components/FileIcon";
import { StatusLabel, statusTone } from "@/components/StatusLabel";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/menu";
import { ProgressBar } from "@/components/ui/progress-bar";
import { capabilities, useActions } from "@/hooks/useActions";
import { api, errorMessage } from "@/lib/api";
import { formatBytes, formatDate, formatDuration, formatPercent, formatSpeed } from "@/lib/format";
import { cn } from "@/lib/utils";
import { fraction, isActive, type Row, type SortKey } from "@/lib/view";
import { useDownloads } from "@/stores/downloads";
import { useSettings } from "@/stores/settings";
import { useUi } from "@/stores/ui";

export type ColumnId =
  | "name"
  | "size"
  | "downloaded"
  | "progress"
  | "speed"
  | "eta"
  | "connections"
  | "status"
  | "destination"
  | "date_added";

const COLUMNS: Record<ColumnId, { track: string; sort: SortKey; align?: "end" | "center" }> = {
  name: { track: "minmax(220px,3fr)", sort: "name" },
  size: { track: "84px", sort: "size", align: "end" },
  downloaded: { track: "92px", sort: "downloaded", align: "end" },
  progress: { track: "minmax(130px,1.3fr)", sort: "progress" },
  speed: { track: "92px", sort: "speed", align: "end" },
  eta: { track: "78px", sort: "eta", align: "end" },
  connections: { track: "86px", sort: "connections", align: "center" },
  status: { track: "minmax(110px,0.9fr)", sort: "status" },
  destination: { track: "minmax(120px,1fr)", sort: "destination" },
  date_added: { track: "122px", sort: "date_added" },
};
export const ALL_COLUMNS = Object.keys(COLUMNS) as ColumnId[];

const ROW_HEIGHT = 38;
const SPEED_LIMITS = [0, 128 * 1024, 512 * 1024, 1024 ** 2, 2 * 1024 ** 2, 5 * 1024 ** 2];

function Cell({ row, col }: { row: Row; col: ColumnId }) {
  const { t } = useTranslation();
  switch (col) {
    case "name":
      return (
        <div className="flex min-w-0 items-center gap-2.5" title={`${row.save_dir}\n${row.url}`}>
          <FileIcon category={row.category} />
          <span className="truncate font-medium" dir="auto">
            {row.file_name}
          </span>
        </div>
      );
    case "size":
      return <span className="tabular">{formatBytes(row.total_size)}</span>;
    case "downloaded":
      return <span className="tabular">{row.downloaded > 0 ? formatBytes(row.downloaded) : "—"}</span>;
    case "progress": {
      const f = fraction(row);
      const active = isActive(row.status);
      return (
        <div className="flex w-full items-center gap-2">
          <ProgressBar
            value={f === null && !active ? 0 : f}
            tone={statusTone(row.status)}
            animated={row.status === "downloading"}
            className="flex-1"
          />
          <span className="w-12 shrink-0 text-end text-xs tabular text-muted-foreground">
            {f === null ? (active ? "…" : "") : formatPercent(f)}
          </span>
        </div>
      );
    }
    case "speed":
      return <span className="tabular">{isActive(row.status) && row.speed > 0 ? formatSpeed(row.speed) : ""}</span>;
    case "eta":
      return <span className="tabular">{isActive(row.status) ? formatDuration(row.eta_secs) : ""}</span>;
    case "connections":
      return (
        <span className="tabular text-muted-foreground">
          {isActive(row.status) ? `${row.active_connections}/${row.max_connections}` : row.max_connections}
        </span>
      );
    case "status":
      return (
        <span title={row.error ?? row.stage ?? undefined} className="min-w-0">
          <StatusLabel status={row.status} errorKind={row.error_kind} scheduled={row.scheduled_at !== null} />
        </span>
      );
    case "destination":
      return (
        <span className="truncate text-muted-foreground" dir="auto" title={row.save_dir}>
          {row.save_dir}
        </span>
      );
    case "date_added":
      return <span className="tabular text-muted-foreground">{formatDate(row.created_at)}</span>;
  }
  return t("common.unknown");
}

function RowMenu({ rows }: { rows: DownloadInfo[] }) {
  const { t } = useTranslation();
  const actions = useActions();
  if (rows.length === 0) return null;
  const caps = rows.map(capabilities);
  const ids = (k: keyof ReturnType<typeof capabilities>) => rows.filter((_, i) => caps[i][k]).map((d) => d.id);
  const single = rows.length === 1 ? rows[0] : null;
  const c = single ? caps[0] : null;
  return (
    <>
      {single && c?.canOpen && (
        <ContextMenuItem onSelect={() => actions.open(single.id)}>
          <ExternalLink /> {t("actions.open")}
        </ContextMenuItem>
      )}
      {single && (
        <ContextMenuItem onSelect={() => actions.openFolder(single.id)}>
          <FolderOpen /> {t("actions.openFolder")}
        </ContextMenuItem>
      )}
      {single && !c?.canOpen && (
        <ContextMenuItem onSelect={() => actions.showProgress(single.id)}>
          <Gauge /> {t("actions.showProgress")}
        </ContextMenuItem>
      )}
      {(single?.status === "completed" || single) && <ContextMenuSeparator />}
      {ids("canResume").length > 0 && (
        <ContextMenuItem onSelect={() => actions.resume(ids("canResume"))}>
          <Play /> {t("actions.resume")}
        </ContextMenuItem>
      )}
      {ids("canPause").length > 0 && (
        <ContextMenuItem onSelect={() => actions.pause(ids("canPause"))}>
          <Pause /> {t("actions.pause")}
        </ContextMenuItem>
      )}
      {ids("canCancel").length > 0 && (
        <ContextMenuItem onSelect={() => actions.cancel(ids("canCancel"))}>
          <Square /> {t("actions.cancel")}
        </ContextMenuItem>
      )}
      {ids("canRestart").length > 0 && (
        <ContextMenuItem onSelect={() => actions.restart(ids("canRestart"))}>
          <RotateCcw /> {t("actions.restart")}
        </ContextMenuItem>
      )}
      {single && c?.canRefreshUrl && (
        <ContextMenuItem onSelect={() => actions.refreshUrl(single.id)}>
          <Link2 /> {t("actions.refreshAddress")}
        </ContextMenuItem>
      )}
      {single && isActive(single.status) && (
        <ContextMenuSub>
          <ContextMenuSubTrigger>
            <Gauge /> {t("actions.speedLimit")}
          </ContextMenuSubTrigger>
          <ContextMenuSubContent>
            {SPEED_LIMITS.map((l) => (
              <ContextMenuCheckboxItem key={l} checked={single.speed_limit === l} onSelect={() => actions.setLimit(single.id, l)}>
                {l ? formatSpeed(l) : t("add.unlimited")}
              </ContextMenuCheckboxItem>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
      )}
      <ContextMenuSeparator />
      {single && (
        <ContextMenuItem onSelect={() => actions.copyUrl(single)}>
          <Copy /> {t("actions.copyUrl")}
        </ContextMenuItem>
      )}
      {single && c?.canMove && (
        <ContextMenuItem onSelect={() => actions.moveTo(single.id)}>
          <FolderInput /> {t("actions.moveTo")}
        </ContextMenuItem>
      )}
      {single && c?.canRename && (
        <ContextMenuItem onSelect={() => useUi.getState().setRename(single.id)}>
          <Pencil /> {t("actions.rename")}
        </ContextMenuItem>
      )}
      {single && c?.canVerify && (
        <ContextMenuItem onSelect={() => actions.checksum(single.id)}>
          <FileCheck2 /> {t("actions.verifyChecksum")}
        </ContextMenuItem>
      )}
      {single && (
        <ContextMenuItem onSelect={() => actions.properties(single.id)}>
          <Info /> {t("actions.properties")}
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem danger onSelect={() => actions.remove(rows.map((r) => r.id))}>
        <Trash2 /> {t("actions.deleteFromList")}
      </ContextMenuItem>
    </>
  );
}

export function DownloadTable({ rows }: { rows: Row[] }) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const selected = useDownloads((s) => s.selected);
  const anchor = useDownloads((s) => s.anchor);
  const setSelected = useDownloads((s) => s.setSelected);
  const sort = useDownloads((s) => s.sort);
  const setSort = useDownloads((s) => s.setSort);
  const search = useDownloads((s) => s.search);
  const items = useDownloads((s) => s.items);
  const visible = useSettings((s) => s.settings?.appearance.visible_columns);
  const saveSettings = useSettings((s) => s.save);
  const actions = useActions();
  const ui = useUi();

  const columns = useMemo<ColumnId[]>(() => {
    const v = (visible ?? []).filter((c): c is ColumnId => c in COLUMNS);
    return ["name", ...ALL_COLUMNS.filter((c) => c !== "name" && v.includes(c))];
  }, [visible]);
  const template = columns.map((c) => COLUMNS[c].track).join(" ");
  const selectedSet = useMemo(() => new Set(selected), [selected]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  const onRowClick = useCallback(
    (e: React.MouseEvent, id: string, index: number) => {
      if (e.shiftKey && anchor) {
        const a = rows.findIndex((r) => r.id === anchor);
        if (a >= 0) {
          const [lo, hi] = a < index ? [a, index] : [index, a];
          setSelected(rows.slice(lo, hi + 1).map((r) => r.id));
          return;
        }
      }
      if (e.ctrlKey || e.metaKey) {
        setSelected(selectedSet.has(id) ? selected.filter((x) => x !== id) : [...selected, id], id);
        return;
      }
      setSelected([id], id);
    },
    [anchor, rows, selected, selectedSet, setSelected],
  );

  const onDoubleClick = (row: Row) => {
    if (row.status === "completed") actions.open(row.id);
    else actions.showProgress(row.id);
  };

  // Keyboard shortcuts on the table.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest("input,textarea,[role=dialog],[role=menu]")) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        setSelected(rows.map((r) => r.id));
      } else if (e.key === "Delete" && selected.length) {
        e.preventDefault();
        ui.openDelete(selected);
      } else if (e.key === "Enter" && selected.length === 1) {
        const r = rows.find((x) => x.id === selected[0]);
        if (r) onDoubleClick(r);
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
        e.preventDefault();
        ui.openAdd();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const toggleColumn = (c: ColumnId) =>
    saveSettings((s) => {
      const set = new Set(s.appearance.visible_columns);
      if (set.has(c)) set.delete(c);
      else set.add(c);
      s.appearance.visible_columns = ALL_COLUMNS.filter((x) => set.has(x));
      return s;
    }).catch((e) => toast.error(t("common.error"), { description: errorMessage(e) }));

  const menuRows = selected.map((id) => items[id]).filter(Boolean);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            className="grid h-8 shrink-0 items-center gap-3 border-b border-border bg-surface-2 px-4 text-xs font-medium text-muted-foreground"
            style={{ gridTemplateColumns: template }}
          >
            {columns.map((c) => {
              const active = sort.key === COLUMNS[c].sort;
              return (
                <button
                  key={c}
                  onClick={() =>
                    setSort({ key: COLUMNS[c].sort, dir: active && sort.dir === "desc" ? "asc" : active ? "desc" : c === "name" ? "asc" : "desc" })
                  }
                  className={cn(
                    "flex min-w-0 items-center gap-1 hover:text-foreground",
                    COLUMNS[c].align === "end" && "justify-end",
                    COLUMNS[c].align === "center" && "justify-center",
                    active && "text-foreground",
                  )}
                >
                  <span className="truncate">{t(`columns.${c}`)}</span>
                  {active && (sort.dir === "asc" ? <ArrowUp className="size-3" /> : <ArrowDown className="size-3" />)}
                </button>
              );
            })}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent>
          {ALL_COLUMNS.filter((c) => c !== "name").map((c) => (
            <ContextMenuCheckboxItem key={c} checked={columns.includes(c)} onSelect={(e) => { e.preventDefault(); void toggleColumn(c); }}>
              {t(`columns.${c}`)}
            </ContextMenuCheckboxItem>
          ))}
        </ContextMenuContent>
      </ContextMenu>

      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            ref={parentRef}
            className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden bg-surface outline-none"
            tabIndex={0}
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) setSelected([], null);
            }}
          >
            {rows.length === 0 ? (
              <EmptyState searching={search.length > 0} />
            ) : (
              <div style={{ height: virtualizer.getTotalSize() }} className="relative w-full">
                {virtualizer.getVirtualItems().map((v) => {
                  const row = rows[v.index];
                  const isSel = selectedSet.has(row.id);
                  return (
                    <div
                      key={row.id}
                      data-index={v.index}
                      onMouseDown={(e) => {
                        if (e.button === 0) onRowClick(e, row.id, v.index);
                      }}
                      onContextMenu={() => {
                        if (!isSel) setSelected([row.id], row.id);
                      }}
                      onDoubleClick={() => onDoubleClick(row)}
                      className={cn(
                        "absolute inset-x-0 grid items-center gap-3 border-b border-border/60 px-4 text-[13px]",
                        isSel ? "bg-primary-soft" : "hover:bg-accent/60",
                      )}
                      style={{ height: ROW_HEIGHT, transform: `translateY(${v.start}px)`, gridTemplateColumns: template }}
                    >
                      {columns.map((c) => (
                        <div
                          key={c}
                          className={cn(
                            "flex min-w-0 items-center overflow-hidden",
                            COLUMNS[c].align === "end" && "justify-end",
                            COLUMNS[c].align === "center" && "justify-center",
                          )}
                        >
                          <Cell row={row} col={c} />
                        </div>
                      ))}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent>
          {menuRows.length > 0 ? (
            <RowMenu rows={menuRows} />
          ) : (
            <>
              <ContextMenuItem onSelect={() => ui.openAdd()}>
                <DownloadCloud /> {t("toolbar.add")}
              </ContextMenuItem>
              <ContextMenuItem onSelect={() => setSelected(rows.map((r) => r.id))}>
                <CheckSquare /> {t("actions.selectAll")}
              </ContextMenuItem>
              <ContextMenuItem onSelect={() => void api.resumeAll()}>
                <Play /> {t("toolbar.resumeAll")}
              </ContextMenuItem>
            </>
          )}
        </ContextMenuContent>
      </ContextMenu>
    </div>
  );
}

function EmptyState({ searching }: { searching: boolean }) {
  const { t } = useTranslation();
  const openAdd = useUi((s) => s.openAdd);
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <div className="flex size-14 items-center justify-center rounded-2xl bg-primary-soft text-primary">
        {searching ? <SearchX className="size-7" /> : <DownloadCloud className="size-7" />}
      </div>
      <div>
        <p className="text-sm font-semibold">{searching ? t("empty.search") : t("empty.title")}</p>
        {!searching && <p className="mt-1 max-w-sm text-[13px] text-muted-foreground">{t("empty.subtitle")}</p>}
      </div>
      {!searching && (
        <button onClick={() => openAdd()} className="text-[13px] font-medium text-primary hover:underline">
          {t("toolbar.add")}
        </button>
      )}
    </div>
  );
}
