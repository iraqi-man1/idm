import { FolderOpen, Gauge, ListPlus, Pause, Play, Plus, Search, Square, Trash2, X } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Tooltip } from "@/components/ui/tooltip";
import { capabilities, useActions } from "@/hooks/useActions";
import { api } from "@/lib/api";
import { useDownloads } from "@/stores/downloads";
import { useUi } from "@/stores/ui";

function ToolButton({
  icon: Icon,
  label,
  onClick,
  disabled,
  shortcut,
}: {
  icon: typeof Plus;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  shortcut?: string;
}) {
  return (
    <Tooltip content={shortcut ? `${label} (${shortcut})` : label}>
      <Button variant="ghost" size="sm" onClick={onClick} disabled={disabled} className="h-8 gap-1.5 px-2">
        <Icon />
        <span className="hidden 2xl:inline">{label}</span>
      </Button>
    </Tooltip>
  );
}

export function Toolbar() {
  const { t } = useTranslation();
  const ui = useUi();
  const actions = useActions();
  const selected = useDownloads((s) => s.selected);
  const items = useDownloads((s) => s.items);
  const search = useDownloads((s) => s.search);
  const setSearch = useDownloads((s) => s.setSearch);

  const sel = useMemo(() => selected.map((id) => items[id]).filter(Boolean), [selected, items]);
  const caps = sel.map(capabilities);
  const any = (k: keyof ReturnType<typeof capabilities>) => caps.some((c) => c[k]);
  const ids = (k: keyof ReturnType<typeof capabilities>) => sel.filter((_, i) => caps[i][k]).map((d) => d.id);
  const single = sel.length === 1 ? sel[0] : null;

  return (
    <div className="flex h-12 shrink-0 items-center gap-1 border-b border-border bg-surface px-3" data-tauri-drag-region>
      <Button size="sm" className="h-8" onClick={() => ui.openAdd()}>
        <Plus />
        {t("toolbar.add")}
      </Button>
      <ToolButton icon={ListPlus} label={t("toolbar.batch")} onClick={() => ui.setBatch(true)} />
      <div className="mx-1 h-5 w-px bg-border" />
      <ToolButton icon={Play} label={t("toolbar.resume")} disabled={!any("canResume")} onClick={() => actions.resume(ids("canResume"))} />
      <ToolButton icon={Pause} label={t("toolbar.pause")} disabled={!any("canPause")} onClick={() => actions.pause(ids("canPause"))} />
      <ToolButton icon={Square} label={t("toolbar.stop")} disabled={!any("canCancel")} onClick={() => actions.cancel(ids("canCancel"))} />
      <ToolButton icon={Trash2} label={t("toolbar.delete")} disabled={sel.length === 0} shortcut="Del" onClick={() => actions.remove(selected)} />
      <div className="mx-1 h-5 w-px bg-border" />
      <ToolButton icon={Gauge} label={t("toolbar.showProgress")} disabled={!single} onClick={() => single && actions.showProgress(single.id)} />
      <ToolButton icon={FolderOpen} label={t("toolbar.openFolder")} disabled={!single} onClick={() => single && actions.openFolder(single.id)} />
      <div className="mx-1 h-5 w-px bg-border" />
      <ToolButton icon={Play} label={t("toolbar.resumeAll")} onClick={() => void api.resumeAll()} />
      <ToolButton icon={Pause} label={t("toolbar.pauseAll")} onClick={() => void api.pauseAll()} />
      <div className="flex-1" data-tauri-drag-region />
      <div className="relative w-56 min-w-40 shrink">
        <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("toolbar.search")}
          className="h-8 w-full rounded-md border border-border-strong bg-surface-2 ps-8 pe-7 text-[13px] placeholder:text-muted-foreground/70 focus-visible:border-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        {search && (
          <button onClick={() => setSearch("")} className="absolute end-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:bg-accent">
            <X className="size-3.5" />
          </button>
        )}
      </div>
    </div>
  );
}
