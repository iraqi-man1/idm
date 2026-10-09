import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { ChecksumResult } from "@/bindings/ChecksumResult";
import type { LogLine } from "@/bindings/LogLine";
import { FileIcon } from "@/components/FileIcon";
import { StatusLabel } from "@/components/StatusLabel";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, errorMessage } from "@/lib/api";
import { formatBytes, formatDate, formatDuration, formatSpeed } from "@/lib/format";
import { cn, isProbablyUrl } from "@/lib/utils";
import { useDownloads } from "@/stores/downloads";
import { useUi } from "@/stores/ui";

export function DeleteDialog() {
  const { t } = useTranslation();
  const { deleteDialog, closeDelete } = useUi();
  const items = useDownloads((s) => s.items);
  const [alsoFiles, setAlsoFiles] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (deleteDialog.open) setAlsoFiles(false);
  }, [deleteDialog.open]);
  const list = deleteDialog.ids.map((id) => items[id]).filter(Boolean);
  const anyCompleted = list.some((d) => d.status === "completed");

  const confirm = async () => {
    setBusy(true);
    try {
      await api.remove(deleteDialog.ids, alsoFiles);
      closeDelete();
    } catch (e) {
      toast.error(t("delete.title"), { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={deleteDialog.open} onOpenChange={(o) => !o && closeDelete()}>
      <DialogContent className="w-[min(440px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle>{t("delete.title")}</DialogTitle>
          <DialogDescription>
            {list.length === 1
              ? t("delete.message_one", { name: list[0].file_name })
              : t("delete.message_other", { count: list.length })}
          </DialogDescription>
        </DialogHeader>
        {anyCompleted && (
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox checked={alsoFiles} onCheckedChange={(v) => setAlsoFiles(v === true)} />
            {t("delete.alsoFiles")}
          </label>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={closeDelete}>
            {t("common.cancel")}
          </Button>
          <Button variant="danger" disabled={busy} onClick={confirm} autoFocus>
            {t("delete.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function RefreshUrlDialog() {
  const { t } = useTranslation();
  const { refreshUrlId, setRefreshUrl } = useUi();
  const d = useDownloads((s) => (refreshUrlId ? s.items[refreshUrlId] : undefined));
  const [url, setUrl] = useState("");
  useEffect(() => setUrl(""), [refreshUrlId]);
  const submit = async () => {
    if (!d) return;
    try {
      await api.updateUrl(d.id, url.trim());
      await api.start(d.id);
      setRefreshUrl(null);
    } catch (e) {
      toast.error(t("refresh.title"), { description: errorMessage(e) });
    }
  };
  return (
    <Dialog open={!!refreshUrlId} onOpenChange={(o) => !o && setRefreshUrl(null)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("refresh.title")}</DialogTitle>
          <DialogDescription>{t("refresh.message")}</DialogDescription>
        </DialogHeader>
        {d && (
          <p className="truncate rounded-md bg-muted px-2 py-1 font-mono text-xs text-muted-foreground" dir="ltr" title={d.url}>
            {d.url}
          </p>
        )}
        <div className="space-y-1.5">
          <Label>{t("refresh.newUrl")}</Label>
          <Input autoFocus value={url} dir="ltr" onChange={(e) => setUrl(e.target.value)} />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setRefreshUrl(null)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={!isProbablyUrl(url)} onClick={submit}>
            {t("actions.resume")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function RenameDialog() {
  const { t } = useTranslation();
  const { renameId, setRename } = useUi();
  const d = useDownloads((s) => (renameId ? s.items[renameId] : undefined));
  const [name, setName] = useState("");
  useEffect(() => setName(d?.file_name ?? ""), [renameId, d?.file_name]);
  const submit = async () => {
    if (!d) return;
    try {
      await api.rename(d.id, name.trim());
      setRename(null);
    } catch (e) {
      toast.error(t("rename.title"), { description: errorMessage(e) });
    }
  };
  return (
    <Dialog open={!!renameId} onOpenChange={(o) => !o && setRename(null)}>
      <DialogContent className="w-[min(460px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle>{t("rename.title")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-1.5">
          <Label>{t("rename.name")}</Label>
          <Input autoFocus value={name} dir="auto" onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setRename(null)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={!name.trim()} onClick={submit}>
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ChecksumDialog() {
  const { t } = useTranslation();
  const { checksumId, setChecksum } = useUi();
  const d = useDownloads((s) => (checksumId ? s.items[checksumId] : undefined));
  const [expected, setExpected] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ChecksumResult | null>(null);
  useEffect(() => {
    setExpected(d?.checksum?.expected ?? "");
    setResult(null);
  }, [checksumId, d?.checksum?.expected]);
  const verify = async () => {
    if (!d) return;
    setBusy(true);
    setResult(null);
    try {
      setResult(await api.verifyChecksum(d.id, expected.trim()));
    } catch (e) {
      toast.error(t("checksum.title"), { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={!!checksumId} onOpenChange={(o) => !o && setChecksum(null)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("checksum.title")}</DialogTitle>
          <DialogDescription>{d?.file_name}</DialogDescription>
        </DialogHeader>
        <div className="space-y-1.5">
          <Label>{t("checksum.expected")}</Label>
          <Input autoFocus value={expected} dir="ltr" className="font-mono text-xs" onChange={(e) => setExpected(e.target.value)} />
          <p className="text-xs text-muted-foreground">{t("add.checksumHint")}</p>
        </div>
        {busy && (
          <p className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> {t("checksum.working")}
          </p>
        )}
        {result && (
          <div className={cn("rounded-md px-3 py-2 text-[13px]", result.ok ? "bg-success-soft text-success" : "bg-danger-soft text-danger")}>
            <p className="flex items-center gap-2 font-medium">
              {result.ok ? <CheckCircle2 className="size-4" /> : <XCircle className="size-4" />}
              {result.ok ? t("checksum.match") : t("checksum.mismatch")}
            </p>
            <p className="mt-1 break-all font-mono text-xs opacity-80" dir="ltr" data-selectable>
              {t("checksum.computed", { value: result.actual })}
            </p>
          </div>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={() => setChecksum(null)}>
            {t("common.close")}
          </Button>
          <Button disabled={!expected.trim() || busy} onClick={verify}>
            {t("checksum.verify")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Prop({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[150px_1fr] gap-3 py-1.5 text-[13px]">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all" data-selectable>
        {children}
      </span>
    </div>
  );
}

export function PropertiesDialog() {
  const { t } = useTranslation();
  const { propertiesId, setProperties } = useUi();
  const d = useDownloads((s) => (propertiesId ? s.items[propertiesId] : undefined));
  const [log, setLog] = useState<LogLine[]>([]);
  useEffect(() => {
    if (propertiesId) void api.log(propertiesId).then(setLog).catch(() => setLog([]));
  }, [propertiesId]);
  if (!d) return null;
  const sep = d.save_dir.includes("\\") ? "\\" : "/";
  const avg = d.elapsed_ms > 0 ? (d.downloaded * 1000) / d.elapsed_ms : 0;
  return (
    <Dialog open={!!propertiesId} onOpenChange={(o) => !o && setProperties(null)}>
      <DialogContent className="w-[min(680px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileIcon category={d.category} />
            <span className="truncate" dir="auto">
              {d.file_name}
            </span>
          </DialogTitle>
          <DialogDescription className="sr-only">{t("properties.title")}</DialogDescription>
        </DialogHeader>
        <div className="min-h-0 overflow-y-auto">
          <div className="divide-y divide-border">
            <Prop label={t("columns.status")}>
              <StatusLabel status={d.status} errorKind={d.error_kind} />
              {d.error && <span className="mt-1 block text-xs text-danger">{d.error}</span>}
            </Prop>
            <Prop label={t("properties.url")}>
              <span dir="ltr" className="font-mono text-xs">{d.url}</span>
            </Prop>
            {d.final_url && (
              <Prop label={t("properties.finalUrl")}>
                <span dir="ltr" className="font-mono text-xs">{d.final_url}</span>
              </Prop>
            )}
            {d.page_url && (
              <Prop label={t("properties.page")}>
                <span dir="ltr" className="font-mono text-xs">{d.page_url}</span>
              </Prop>
            )}
            <Prop label={t("properties.path")}>
              <span dir="ltr">{`${d.save_dir}${sep}${d.file_name}`}</span>
            </Prop>
            <Prop label={t("columns.size")}>{formatBytes(d.total_size)}</Prop>
            <Prop label={t("columns.downloaded")}>{formatBytes(d.downloaded)}</Prop>
            <Prop label={t("progress.resumable")}>{d.resumable === null ? t("common.unknown") : d.resumable ? t("common.yes") : t("common.no")}</Prop>
            {d.mime && <Prop label={t("properties.mime")}>{d.mime}</Prop>}
            <Prop label={t("properties.kind")}>{d.kind.toUpperCase()}</Prop>
            <Prop label={t("properties.added")}>{formatDate(d.created_at)}</Prop>
            {d.started_at && <Prop label={t("properties.started")}>{formatDate(d.started_at)}</Prop>}
            {d.completed_at && <Prop label={t("properties.completedAt")}>{formatDate(d.completed_at)}</Prop>}
            <Prop label={t("properties.elapsed")}>{formatDuration(d.elapsed_ms / 1000)}</Prop>
            <Prop label={t("properties.averageSpeed")}>{formatSpeed(avg)}</Prop>
            {d.checksum && (
              <Prop label={t("properties.checksum")}>
                <span dir="ltr" className="font-mono text-xs">
                  {d.checksum.algorithm.toUpperCase()} {d.checksum.expected}
                </span>{" "}
                {d.checksum_ok !== null && (
                  <span className={d.checksum_ok ? "text-success" : "text-danger"}>
                    ({d.checksum_ok ? t("properties.checksumVerified") : t("properties.checksumFailed")})
                  </span>
                )}
              </Prop>
            )}
            {d.has_secrets && <Prop label="🔒">{t("properties.secrets")}</Prop>}
          </div>
          {log.length > 0 && (
            <>
              <h4 className="mb-1 mt-4 text-[13px] font-semibold">{t("properties.log")}</h4>
              <div className="max-h-48 overflow-y-auto rounded-md border border-border bg-surface-2 p-2 font-mono text-[11px] leading-5" dir="ltr" data-selectable>
                {log.map((l, i) => (
                  <div key={i} className={cn(l.level === "error" && "text-danger", l.level === "warn" && "text-warning")}>
                    <span className="text-muted-foreground">{new Date(l.ts).toLocaleTimeString()} </span>
                    {l.message}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
        <DialogFooter>
          <Button onClick={() => setProperties(null)}>{t("common.close")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
