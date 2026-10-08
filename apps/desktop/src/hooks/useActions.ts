import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import { api, errorMessage } from "@/lib/api";
import { isActive } from "@/lib/view";
import { useDownloads } from "@/stores/downloads";
import { useUi } from "@/stores/ui";

/** Which actions make sense for a download in its current state. */
export function capabilities(d: DownloadInfo) {
  const active = isActive(d.status);
  return {
    canPause: active || d.status === "queued",
    canResume: ["paused", "failed", "cancelled", "queued"].includes(d.status),
    canCancel: active || d.status === "paused" || d.status === "queued",
    canRestart: !active,
    canOpen: d.status === "completed",
    canMove: d.status === "completed",
    canVerify: d.status === "completed",
    canRename: !active,
    canRefreshUrl: !active && d.status !== "completed",
    canShowProgress: true,
  };
}

async function run(label: string, fn: () => Promise<unknown>) {
  try {
    await fn();
  } catch (e) {
    toast.error(label, { description: errorMessage(e) });
  }
}

export function useActions() {
  const { t } = useTranslation();
  const ui = useUi();

  const selectedInfos = useCallback((): DownloadInfo[] => {
    const { items, selected } = useDownloads.getState();
    return selected.map((id) => items[id]).filter(Boolean);
  }, []);

  return {
    selectedInfos,
    resume: (ids: string[]) => run(t("actions.resume"), () => api.startMany(ids)),
    pause: (ids: string[]) => run(t("actions.pause"), () => api.pauseMany(ids)),
    cancel: (ids: string[]) =>
      run(t("actions.cancel"), async () => {
        for (const id of ids) await api.cancel(id);
      }),
    restart: (ids: string[]) =>
      run(t("actions.restart"), async () => {
        for (const id of ids) await api.restart(id);
      }),
    remove: (ids: string[]) => ui.openDelete(ids),
    open: (id: string) => run(t("actions.open"), () => api.openFile(id)),
    openFolder: (id: string) => run(t("actions.openFolder"), () => api.openFolder(id)),
    showProgress: (id: string) => run(t("actions.showProgress"), () => api.openProgressWindow(id)),
    copyUrl: (d: DownloadInfo) =>
      run(t("actions.copyUrl"), async () => {
        await writeText(d.url);
        toast.success(t("common.copied"));
      }),
    moveTo: (id: string) =>
      run(t("actions.moveTo"), async () => {
        const dir = await open({ directory: true, multiple: false });
        if (typeof dir === "string") await api.move(id, dir);
      }),
    setLimit: (id: string, bytes: number) => run(t("actions.speedLimit"), () => api.setSpeedLimit(id, bytes)),
    properties: (id: string) => ui.setProperties(id),
    checksum: (id: string) => ui.setChecksum(id),
    refreshUrl: (id: string) => ui.setRefreshUrl(id),
  };
}
