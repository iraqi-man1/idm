import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { PendingCapture } from "@/bindings/PendingCapture";
import type { StartMode } from "@/bindings/StartMode";
import type { UrlInfo } from "@/bindings/UrlInfo";
import { FileIcon } from "@/components/FileIcon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAppearance } from "@/hooks/useAppearance";
import { api, errorMessage } from "@/lib/api";
import { formatBytes } from "@/lib/format";
import { useSettings } from "@/stores/settings";

/** IDM-style "Download File Info" dialog for downloads sent by the browser. */
export function CaptureWindow({ id }: { id: string }) {
  useAppearance();
  const { t } = useTranslation();
  const loadSettings = useSettings((s) => s.load);
  const [pending, setPending] = useState<PendingCapture | null>(null);
  const [info, setInfo] = useState<UrlInfo | null>(null);
  const [probing, setProbing] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [dir, setDir] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void loadSettings();
    api
      .getPendingCapture(id)
      .then((p) => {
        setPending(p);
        setName(p.request.file_name ?? "");
        return api.probeCapture(id).then((i) => {
          setInfo(i);
          setName((n) => n || i.file_name);
          setDir(i.suggested_dir);
        });
      })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setProbing(false));
    void api.defaultDownloadDir().then((d) => setDir((cur) => cur || d));
  }, [id, loadSettings]);

  const close = () => void getCurrentWindow().close();

  const submit = async (start: StartMode) => {
    if (!pending) return;
    setBusy(true);
    try {
      const nameChanged = name.trim() && name.trim() !== (info?.file_name ?? pending.request.file_name);
      await api.resolveCapture(id, {
        ...pending.request,
        file_name: nameChanged || pending.request.file_name ? name.trim() : null,
        save_dir: dir.trim() || null,
        start,
        expected_size: info?.total_size ?? pending.request.expected_size,
        mime: info?.mime ?? pending.request.mime,
      });
      close();
    } catch (e) {
      toast.error(t("capture.title"), { description: errorMessage(e) });
      setBusy(false);
    }
  };

  const cancel = async () => {
    await api.resolveCapture(id, null).catch(() => undefined);
    close();
  };

  if (!pending) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-[13px]">
        {error ? <p className="text-danger">{error}</p> : <Loader2 className="size-5 animate-spin text-muted-foreground" />}
        {error && <Button onClick={close}>{t("common.close")}</Button>}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex items-center gap-3 border-b border-border bg-surface px-5 py-4">
        <FileIcon category={info?.category ?? "other"} className="size-10" />
        <div className="min-w-0 flex-1">
          <h1 className="text-[14px] font-semibold">{t("capture.title")}</h1>
          <p className="truncate font-mono text-[11px] text-muted-foreground" dir="ltr" title={pending.request.url}>
            {pending.request.url}
          </p>
        </div>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
        {pending.duplicate_of && (
          <div className="flex items-center gap-2 rounded-md bg-warning-soft px-3 py-2 text-xs text-warning">
            <AlertTriangle className="size-4 shrink-0" /> {t("add.duplicate")}
          </div>
        )}
        <div className="flex flex-wrap gap-x-6 gap-y-1 rounded-lg border border-border bg-surface-2 px-3 py-2 text-[13px]">
          {probing ? (
            <span className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" /> {t("add.checking")}
            </span>
          ) : (
            <>
              <span>
                <span className="text-muted-foreground">{t("add.size")}: </span>
                <b className="tabular">
                  {formatBytes(info?.total_size ?? pending.request.expected_size ?? null)}
                </b>
              </span>
              {info && (
                <span>
                  <span className="text-muted-foreground">{t("add.resume")}: </span>
                  <b className={info.resumable ? "text-success" : "text-warning"}>{info.resumable ? t("common.yes") : t("common.no")}</b>
                </span>
              )}
              {error && <span className="text-xs text-danger">{t("add.probeFailed", { error })}</span>}
            </>
          )}
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cap-name">{t("add.fileName")}</Label>
          <Input id="cap-name" value={name} dir="auto" onChange={(e) => setName(e.target.value)} autoFocus />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cap-dir">{t("add.saveTo")}</Label>
          <div className="flex gap-2">
            <Input id="cap-dir" value={dir} dir="ltr" onChange={(e) => setDir(e.target.value)} />
            <Button
              variant="secondary"
              onClick={async () => {
                const d = await open({ directory: true, defaultPath: dir || undefined });
                if (typeof d === "string") setDir(d);
              }}
            >
              <FolderOpen /> {t("common.browse")}
            </Button>
          </div>
        </div>
      </div>
      <div className="flex items-center justify-end gap-2 border-t border-border bg-surface px-5 py-3">
        <Button variant="ghost" onClick={cancel} disabled={busy}>
          {t("common.cancel")}
        </Button>
        <Button variant="secondary" onClick={() => submit("paused")} disabled={busy}>
          {t("add.later")}
        </Button>
        <Button onClick={() => submit("now")} disabled={busy || !name.trim()}>
          {busy && <Loader2 className="animate-spin" />}
          {t("add.startNow")}
        </Button>
      </div>
    </div>
  );
}
