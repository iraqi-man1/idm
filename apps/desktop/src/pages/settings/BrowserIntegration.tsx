import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckCircle2, CircleDashed, Copy, FolderOpen, Puzzle, RefreshCw, Wrench, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { BrowserInfo } from "@/bindings/BrowserInfo";
import type { BrowserIntegrationStatus } from "@/bindings/BrowserIntegrationStatus";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, errorMessage, events } from "@/lib/api";
import { formatDate } from "@/lib/format";

const EXT_PAGES: Record<string, string> = {
  chrome: "chrome://extensions",
  chromium: "chrome://extensions",
  edge: "edge://extensions",
  brave: "brave://extensions",
  vivaldi: "vivaldi://extensions",
  opera: "opera://extensions",
  firefox: "about:debugging#/runtime/this-firefox",
};

function CopyLine({ text }: { text: string }) {
  const { t } = useTranslation();
  return (
    <div className="mt-1 flex items-center gap-2">
      <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs" dir="ltr" data-selectable title={text}>
        {text}
      </code>
      <Button
        size="sm"
        variant="secondary"
        onClick={() => void writeText(text).then(() => toast.success(t("common.copied")))}
      >
        <Copy /> {t("settings.browser.copy")}
      </Button>
    </div>
  );
}

function InstallDialog({ browser, status, onClose }: { browser: BrowserInfo | null; status: BrowserIntegrationStatus; onClose: () => void }) {
  const { t } = useTranslation();
  if (!browser) return null;
  const firefox = browser.family === "firefox";
  const dir = firefox ? status.firefox_extension_dir : status.chromium_extension_dir;
  const page = EXT_PAGES[browser.id] ?? "chrome://extensions";
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="w-[min(620px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle>{t("settings.browser.installTitle", { browser: browser.name })}</DialogTitle>
          <DialogDescription>{t("settings.browser.consent")}</DialogDescription>
        </DialogHeader>
        {browser.store_url ? (
          <div className="space-y-3 text-[13px]">
            <p>{t("settings.browser.storeIntro")}</p>
            <Button onClick={() => void openUrl(browser.store_url!)}>{t("settings.browser.openStore")}</Button>
          </div>
        ) : !dir ? (
          <p className="text-[13px] text-warning">{t("settings.browser.noBundle")}</p>
        ) : (
          <ol className="list-decimal space-y-3 ps-5 text-[13px]">
            <p className="-ms-5 text-muted-foreground">{t("settings.browser.devIntro")}</p>
            {firefox ? (
              <>
                <li>
                  {t("settings.browser.firefoxStep1")}
                  <CopyLine text={page} />
                </li>
                <li>
                  {t("settings.browser.firefoxStep2")}
                  <CopyLine text={dir} />
                  <Button size="sm" variant="ghost" className="mt-1" onClick={() => void api.openExtensionFolder("firefox")}>
                    <FolderOpen /> {t("settings.browser.openFolder")}
                  </Button>
                </li>
                <p className="-ms-5 text-xs text-muted-foreground">{t("settings.browser.firefoxNote")}</p>
              </>
            ) : (
              <>
                <li>
                  {t("settings.browser.chromiumStep1", { browser: browser.name })}
                  <CopyLine text={page} />
                </li>
                <li>{t("settings.browser.chromiumStep2")}</li>
                <li>
                  {t("settings.browser.chromiumStep3")}
                  <CopyLine text={dir} />
                  <Button size="sm" variant="ghost" className="mt-1" onClick={() => void api.openExtensionFolder("chromium")}>
                    <FolderOpen /> {t("settings.browser.openFolder")}
                  </Button>
                </li>
                <li>{t("settings.browser.chromiumStep4")}</li>
              </>
            )}
          </ol>
        )}
        <DialogFooter>
          <Button onClick={onClose}>{t("common.close")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function BrowserIntegration() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<BrowserIntegrationStatus | null>(null);
  const [installFor, setInstallFor] = useState<BrowserInfo | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => void api.browserStatus().then(setStatus).catch(() => undefined), []);
  useEffect(() => {
    refresh();
    const un = events.browserStatusChanged(refresh);
    const timer = setInterval(refresh, 5000);
    return () => {
      clearInterval(timer);
      void un.then((u) => u());
    };
  }, [refresh]);

  const repair = async () => {
    setBusy(true);
    try {
      setStatus(await api.repairBrowserIntegration());
      toast.success(t("settings.browser.repaired"));
    } catch (e) {
      toast.error(t("settings.browser.repair"), { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  if (!status) return null;
  const ext = status.last_extension;
  const installed = status.browsers.filter((b) => b.installed);
  const others = status.browsers.filter((b) => !b.installed);
  const registeredCount = status.browsers.filter((b) => b.registered).length;

  return (
    <section className="rounded-xl border border-border bg-surface shadow-card">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <div className="flex size-9 items-center justify-center rounded-lg bg-primary-soft text-primary">
          <Puzzle className="size-5" />
        </div>
        <div className="min-w-0 flex-1 text-[13px]">
          <p className="font-semibold">{t("settings.browser.status")}</p>
          {status.connected > 0 && ext ? (
            <p className={ext.compatible ? "text-success" : "text-warning"}>
              {ext.compatible
                ? t("settings.browser.connected", { browser: ext.browser, version: ext.version })
                : t("settings.browser.incompatible", { version: ext.version })}
            </p>
          ) : ext ? (
            <p className="text-muted-foreground">{t("settings.browser.lastSeen", { browser: ext.browser, date: formatDate(ext.last_seen) })}</p>
          ) : (
            <p className="text-muted-foreground">{t("settings.browser.neverSeen")}</p>
          )}
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.browser.host")}:{" "}
            {status.host_path ? t("settings.browser.registeredFor", { count: registeredCount }) : <span className="text-danger">{t("settings.browser.hostMissing")}</span>}
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={repair} disabled={busy}>
          {busy ? <RefreshCw className="animate-spin" /> : <Wrench />}
          {t("settings.browser.repair")}
        </Button>
      </div>
      <div className="divide-y divide-border">
        {[...installed, ...others].map((b) => (
          <div key={b.id} className="flex items-center gap-3 px-4 py-2.5 text-[13px]">
            <span className="w-40 font-medium">{b.name}</span>
            {!b.installed ? (
              <Badge>
                <CircleDashed className="size-3" /> {t("settings.browser.notInstalled")}
              </Badge>
            ) : b.registered ? (
              <Badge tone="success">
                <CheckCircle2 className="size-3" /> {t("settings.browser.registered")}
              </Badge>
            ) : (
              <Badge tone="warning">
                <XCircle className="size-3" /> {t("settings.browser.notRegistered")}
              </Badge>
            )}
            {b.error && <span className="truncate text-xs text-danger" title={b.error}>{b.error}</span>}
            <div className="flex-1" />
            {b.installed && (
              <Button size="sm" variant={b.registered ? "default" : "secondary"} onClick={() => setInstallFor(b)}>
                <Puzzle /> {t("settings.browser.install")}
              </Button>
            )}
          </div>
        ))}
      </div>
      <InstallDialog browser={installFor} status={status} onClose={() => setInstallFor(null)} />
    </section>
  );
}
