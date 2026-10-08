import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ChevronDown, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AddDownloadRequest } from "@/bindings/AddDownloadRequest";
import type { ChecksumAlgorithm } from "@/bindings/ChecksumAlgorithm";
import type { StartMode } from "@/bindings/StartMode";
import type { UrlInfo } from "@/bindings/UrlInfo";
import { FileIcon } from "@/components/FileIcon";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input, Textarea } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { api, errorMessage } from "@/lib/api";
import { formatBytes, parseSize } from "@/lib/format";
import { cn, isProbablyUrl } from "@/lib/utils";
import { useSettings } from "@/stores/settings";
import { useUi } from "@/stores/ui";

const CONNECTION_CHOICES = ["1", "2", "4", "8", "16", "24", "32"] as const;
const HEX_LEN_ALGO: Record<number, ChecksumAlgorithm> = { 32: "md5", 40: "sha1", 64: "sha256", 128: "sha512" };

export function AddDownloadDialog() {
  const { t } = useTranslation();
  const { addDialog, closeAdd } = useUi();
  const settings = useSettings((s) => s.settings);

  const [url, setUrl] = useState("");
  const [info, setInfo] = useState<UrlInfo | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [dir, setDir] = useState("");
  const [dirEdited, setDirEdited] = useState(false);
  const [connections, setConnections] = useState("8");
  const [advanced, setAdvanced] = useState(false);
  const [referer, setReferer] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [cookies, setCookies] = useState("");
  const [checksum, setChecksum] = useState("");
  const [limit, setLimit] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const probeSeq = useRef(0);

  // Reset when opened; prefill from the request or the clipboard.
  useEffect(() => {
    if (!addDialog.open) return;
    setInfo(null);
    setProbeError(null);
    setName("");
    setNameEdited(false);
    setDir("");
    setDirEdited(false);
    setAdvanced(false);
    setReferer("");
    setUsername("");
    setPassword("");
    setCookies("");
    setChecksum("");
    setLimit("");
    setConnections(String(settings?.downloads.connections_per_download ?? 8));
    if (addDialog.url) {
      setUrl(addDialog.url);
    } else {
      setUrl("");
      readText()
        .then((txt) => {
          const candidate = txt?.trim() ?? "";
          if (isProbablyUrl(candidate)) setUrl(candidate);
        })
        .catch(() => undefined);
    }
    void api.defaultDownloadDir().then((d) => setDir((cur) => cur || d));
  }, [addDialog.open, addDialog.url, settings?.downloads.connections_per_download]);

  const credentials = username ? { username, password } : null;

  // Debounced probe of the address.
  useEffect(() => {
    if (!addDialog.open) return;
    const u = url.trim();
    if (!isProbablyUrl(u)) {
      setInfo(null);
      setProbeError(null);
      return;
    }
    const seq = ++probeSeq.current;
    const timer = setTimeout(() => {
      setProbing(true);
      setProbeError(null);
      const req: AddDownloadRequest = {
        ...emptyRequest(),
        url: u,
        referer: referer || null,
        credentials,
        cookies: cookies || null,
      };
      api
        .probeUrl(req)
        .then((i) => {
          if (seq !== probeSeq.current) return;
          setInfo(i);
          if (!nameEdited) setName(i.file_name);
          if (!dirEdited) setDir(i.suggested_dir);
        })
        .catch((e) => {
          if (seq !== probeSeq.current) return;
          setInfo(null);
          setProbeError(errorMessage(e));
        })
        .finally(() => {
          if (seq === probeSeq.current) setProbing(false);
        });
    }, 450);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [url, addDialog.open, referer, username, password, cookies]);

  const urlValid = isProbablyUrl(url.trim());
  const checksumAlgo = useMemo(() => {
    const c = checksum.trim().toLowerCase();
    return c && /^[0-9a-f]+$/.test(c) ? HEX_LEN_ALGO[c.length] ?? null : null;
  }, [checksum]);
  const checksumInvalid = checksum.trim() !== "" && !checksumAlgo;
  const limitBytes = limit.trim() ? parseSize(limit) : 0;
  const limitInvalid = limitBytes === null;

  const submit = async (start: StartMode) => {
    if (!urlValid || checksumInvalid || limitInvalid) return;
    setSubmitting(true);
    try {
      const req: AddDownloadRequest = {
        ...emptyRequest(),
        url: url.trim(),
        file_name: nameEdited && name.trim() ? name.trim() : null,
        save_dir: dir.trim() || null,
        referer: referer.trim() || null,
        cookies: cookies.trim() || null,
        credentials,
        connections: Number(connections),
        start,
        checksum: checksumAlgo ? { algorithm: checksumAlgo, expected: checksum.trim().toLowerCase() } : null,
        speed_limit: limitBytes || null,
        expected_size: info?.total_size ?? null,
        mime: info?.mime ?? null,
      };
      const d = await api.addDownload(req);
      closeAdd();
      if (start === "now" && settings?.general.show_progress_window) void api.openProgressWindow(d.id);
    } catch (e) {
      toast.error(t("add.title"), { description: errorMessage(e) });
    } finally {
      setSubmitting(false);
    }
  };

  const pickDir = async () => {
    const d = await open({ directory: true, multiple: false, defaultPath: dir || undefined });
    if (typeof d === "string") {
      setDir(d);
      setDirEdited(true);
    }
  };

  return (
    <Dialog open={addDialog.open} onOpenChange={(o) => !o && closeAdd()}>
      <DialogContent className="w-[min(620px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle>{t("add.title")}</DialogTitle>
          <DialogDescription className="sr-only">{t("add.urlPlaceholder")}</DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto pe-1">
          <div className="space-y-1.5">
            <Label htmlFor="add-url">{t("add.url")}</Label>
            <Textarea
              id="add-url"
              autoFocus
              rows={2}
              value={url}
              dir="ltr"
              onChange={(e) => setUrl(e.target.value.replace(/\s+/g, ""))}
              placeholder={t("add.urlPlaceholder")}
              className={cn("min-h-14 resize-none font-mono text-xs", url && !urlValid && "border-danger")}
            />
            {url && !urlValid && <p className="text-xs text-danger">{t("add.invalidUrl")}</p>}
          </div>

          <div className="flex items-center gap-3 rounded-lg border border-border bg-surface-2 p-3">
            {info ? <FileIcon category={info.category} className="size-9" /> : <div className="size-9 rounded-md bg-muted" />}
            <div className="min-w-0 flex-1 text-[13px]">
              {probing ? (
                <span className="flex items-center gap-2 text-muted-foreground">
                  <Loader2 className="size-4 animate-spin" /> {t("add.checking")}
                </span>
              ) : info ? (
                <div className="flex flex-wrap gap-x-5 gap-y-1">
                  <span>
                    <span className="text-muted-foreground">{t("add.size")}: </span>
                    <b className="tabular">{info.total_size !== null ? formatBytes(info.total_size) : t("common.unknown")}</b>
                  </span>
                  <span>
                    <span className="text-muted-foreground">{t("add.resume")}: </span>
                    <b className={info.resumable ? "text-success" : "text-warning"}>{info.resumable ? t("common.yes") : t("common.no")}</b>
                  </span>
                  {info.mime && (
                    <span className="truncate text-muted-foreground" dir="ltr">
                      {info.mime.split(";")[0]}
                    </span>
                  )}
                </div>
              ) : probeError ? (
                <div className="text-xs">
                  <p className="text-danger">{t("add.probeFailed", { error: probeError })}</p>
                  <p className="text-muted-foreground">{t("add.probeFailedHint")}</p>
                </div>
              ) : (
                <span className="text-muted-foreground">—</span>
              )}
            </div>
          </div>

          {info?.duplicate_of && (
            <div className="flex items-center gap-2 rounded-md bg-warning-soft px-3 py-2 text-xs text-warning">
              <AlertTriangle className="size-4 shrink-0" /> {t("add.duplicate")}
            </div>
          )}
          {info && (info.kind === "hls" || info.kind === "dash") && (
            <div className="rounded-md bg-primary-soft px-3 py-2 text-xs text-primary">{t("add.media")}</div>
          )}

          <div className="grid grid-cols-[1fr_auto] gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="add-name">{t("add.fileName")}</Label>
              <Input
                id="add-name"
                value={name}
                dir="auto"
                onChange={(e) => {
                  setName(e.target.value);
                  setNameEdited(true);
                }}
              />
            </div>
            <div className="space-y-1.5">
              <Label>{t("add.connections")}</Label>
              <Select
                value={connections}
                onChange={setConnections}
                className="min-w-20"
                options={CONNECTION_CHOICES.map((c) => ({ value: c, label: c }))}
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="add-dir">{t("add.saveTo")}</Label>
            <div className="flex gap-2">
              <Input
                id="add-dir"
                value={dir}
                dir="ltr"
                onChange={(e) => {
                  setDir(e.target.value);
                  setDirEdited(true);
                }}
              />
              <Button variant="secondary" onClick={pickDir} type="button">
                <FolderOpen />
                {t("common.browse")}
              </Button>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setAdvanced((a) => !a)}
            className="flex items-center gap-1 self-start text-[13px] font-medium text-muted-foreground hover:text-foreground"
          >
            <ChevronDown className={cn("size-4 transition-transform", !advanced && "-rotate-90 rtl:rotate-90")} />
            {t("add.advanced")}
          </button>
          {advanced && (
            <div className="grid grid-cols-2 gap-3 rounded-lg border border-border p-3">
              <div className="col-span-2 space-y-1.5">
                <Label>{t("add.referer")}</Label>
                <Input value={referer} dir="ltr" onChange={(e) => setReferer(e.target.value)} />
              </div>
              <div className="space-y-1.5">
                <Label>{t("add.username")}</Label>
                <Input value={username} autoComplete="off" onChange={(e) => setUsername(e.target.value)} />
              </div>
              <div className="space-y-1.5">
                <Label>{t("add.password")}</Label>
                <Input type="password" value={password} autoComplete="off" onChange={(e) => setPassword(e.target.value)} />
              </div>
              <div className="col-span-2 space-y-1.5">
                <Label>{t("add.cookies")}</Label>
                <Input value={cookies} dir="ltr" placeholder="name=value; other=value" onChange={(e) => setCookies(e.target.value)} />
              </div>
              <div className="col-span-2 space-y-1.5">
                <Label>{t("add.checksum")}</Label>
                <Input
                  value={checksum}
                  dir="ltr"
                  className={cn("font-mono text-xs", checksumInvalid && "border-danger")}
                  onChange={(e) => setChecksum(e.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  {checksumAlgo ? checksumAlgo.toUpperCase() : t("add.checksumHint")}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label>{t("add.speedLimit")}</Label>
                <Input
                  value={limit}
                  dir="ltr"
                  placeholder={t("add.unlimited")}
                  className={cn(limitInvalid && "border-danger")}
                  onChange={(e) => setLimit(e.target.value)}
                />
              </div>
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={closeAdd}>
            {t("common.cancel")}
          </Button>
          <Button variant="secondary" disabled={!urlValid || submitting} onClick={() => submit("queue")}>
            {t("add.addToQueue")}
          </Button>
          <Button variant="secondary" disabled={!urlValid || submitting} onClick={() => submit("paused")}>
            {t("add.later")}
          </Button>
          <Button disabled={!urlValid || submitting || checksumInvalid || limitInvalid} onClick={() => submit("now")}>
            {submitting && <Loader2 className="animate-spin" />}
            {t("add.startNow")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function emptyRequest(): AddDownloadRequest {
  return {
    url: "",
    file_name: null,
    save_dir: null,
    referer: null,
    user_agent: null,
    headers: [],
    cookies: null,
    credentials: null,
    connections: null,
    queue_id: null,
    start: "now",
    scheduled_at: null,
    checksum: null,
    media: null,
    source: "user",
    expected_size: null,
    mime: null,
    priority: null,
    speed_limit: null,
    conflict: null,
    page_url: null,
  };
}
