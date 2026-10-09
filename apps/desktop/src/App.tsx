import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Toaster } from "sonner";
import { DownloadTable } from "@/components/DownloadTable";
import { AddDownloadDialog } from "@/components/dialogs/AddDownloadDialog";
import { BatchImportDialog } from "@/components/dialogs/BatchImportDialog";
import { PostActionDialog } from "@/components/dialogs/PostActionDialog";
import { ChecksumDialog, DeleteDialog, PropertiesDialog, RefreshUrlDialog, RenameDialog } from "@/components/dialogs/SmallDialogs";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAppearance } from "@/hooks/useAppearance";
import { useAutoUpdateCheck } from "@/hooks/useAutoUpdateCheck";
import { useEngineSync } from "@/hooks/useEngine";
import { Sidebar } from "@/layouts/Sidebar";
import { StatusBar } from "@/layouts/StatusBar";
import { Toolbar } from "@/layouts/Toolbar";
import { events } from "@/lib/api";
import { extractUrls } from "@/lib/utils";
import { matchesFilter, matchesSearch, mergeRow, sortRows } from "@/lib/view";
import { QueuesPage } from "@/pages/QueuesPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { StatisticsPage } from "@/pages/StatisticsPage";
import { useDownloads } from "@/stores/downloads";
import { useQueues } from "@/stores/queues";
import { useUi } from "@/stores/ui";

function DownloadsView() {
  const items = useDownloads((s) => s.items);
  const progress = useDownloads((s) => s.progress);
  const filter = useDownloads((s) => s.filter);
  const search = useDownloads((s) => s.search);
  const sort = useDownloads((s) => s.sort);
  const rows = useMemo(() => {
    const list = Object.values(items)
      .filter((d) => matchesFilter(d, filter) && matchesSearch(d, search))
      .map((d) => mergeRow(d, progress[d.id]));
    return sortRows(list, sort);
  }, [items, progress, filter, search, sort]);
  return (
    <>
      <Toolbar />
      <DownloadTable rows={rows} />
      <StatusBar visibleCount={rows.length} />
    </>
  );
}

export default function App() {
  useEngineSync();
  useAppearance();
  useAutoUpdateCheck();
  const { i18n } = useTranslation();
  const page = useUi((s) => s.page);
  const openAdd = useUi((s) => s.openAdd);
  const setBatch = useUi((s) => s.setBatch);
  const [dragging, setDragging] = useState(false);

  // Queue definitions, running state and power holds.
  useEffect(() => {
    const { load, apply } = useQueues.getState();
    void load();
    const un = events.scheduler(apply);
    return () => void un.then((u) => u());
  }, []);

  // URLs passed on the command line / second instance.
  useEffect(() => {
    const un = events.addUrl((url) => openAdd(url));
    return () => void un.then((u) => u());
  }, [openAdd]);

  // "Download all links" from the browser extension.
  useEffect(() => {
    const un = events.batchUrls(({ urls }) => {
      setBatch(true);
      setTimeout(() => window.dispatchEvent(new CustomEvent("velox:batch-text", { detail: urls.join("\n") })), 50);
    });
    return () => void un.then((u) => u());
  }, [setBatch]);

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    const text = e.dataTransfer.getData("text/uri-list") || e.dataTransfer.getData("text/plain");
    const urls = extractUrls(text);
    if (urls.length === 1) openAdd(urls[0]);
    else if (urls.length > 1) {
      setBatch(true);
      // The batch dialog reads the clipboard-free text via a custom event.
      setTimeout(() => window.dispatchEvent(new CustomEvent("velox:batch-text", { detail: urls.join("\n") })), 50);
    }
  };

  return (
    <TooltipProvider>
      <div
        className="flex h-full"
        onDragOver={(e) => {
          if (e.dataTransfer.types.some((t) => t === "text/uri-list" || t === "text/plain")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => {
          if (e.currentTarget === e.target) setDragging(false);
        }}
        onDrop={onDrop}
      >
        <Sidebar />
        <main className="relative flex min-w-0 flex-1 flex-col">
          {page === "downloads" && <DownloadsView />}
          {page === "settings" && <SettingsPage />}
          {page === "statistics" && <StatisticsPage />}
          {page === "queues" && <QueuesPage />}
          {dragging && <div className="pointer-events-none absolute inset-2 rounded-xl border-2 border-dashed border-primary bg-primary-soft/40" />}
        </main>
      </div>
      <AddDownloadDialog />
      <BatchImportDialog />
      <DeleteDialog />
      <PropertiesDialog />
      <ChecksumDialog />
      <RefreshUrlDialog />
      <RenameDialog />
      <PostActionDialog />
      <Toaster position="bottom-right" richColors closeButton dir={i18n.dir()} theme="system" />
    </TooltipProvider>
  );
}
