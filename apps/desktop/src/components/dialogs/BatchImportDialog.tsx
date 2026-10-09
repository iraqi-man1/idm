import { open } from "@tauri-apps/plugin-dialog";
import { FileText, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input, Textarea } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, errorMessage } from "@/lib/api";
import { expandPattern, extractUrls } from "@/lib/utils";
import { useUi } from "@/stores/ui";

export function BatchImportDialog() {
  const { t } = useTranslation();
  const { batchDialog, setBatch } = useUi();
  const [text, setText] = useState("");
  const [dir, setDir] = useState("");
  const [startNow, setStartNow] = useState(false);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (batchDialog) {
      setText("");
      void api.defaultDownloadDir().then(setDir);
    }
  }, [batchDialog]);

  // Links dropped onto the window arrive through this event.
  useEffect(() => {
    const onText = (e: Event) => setText((e as CustomEvent<string>).detail);
    window.addEventListener("velox:batch-text", onText);
    return () => window.removeEventListener("velox:batch-text", onText);
  }, []);

  const urls = useMemo(() => {
    const out: string[] = [];
    for (const u of extractUrls(text)) out.push(...expandPattern(u));
    return Array.from(new Set(out)).slice(0, 5000);
  }, [text]);

  const importFile = (f: File) => {
    const reader = new FileReader();
    reader.onload = () => setText((cur) => (cur ? cur + "\n" : "") + String(reader.result ?? ""));
    reader.readAsText(f);
  };

  const submit = async () => {
    setBusy(true);
    try {
      const res = await api.addBatch(urls, dir || null, startNow ? "queue" : "paused");
      toast.success(t("batch.result", { added: res.added.length, failed: res.failed.length }), {
        description: res.failed.slice(0, 3).map((f) => `${f.url}: ${f.error}`).join("\n") || undefined,
      });
      setBatch(false);
    } catch (e) {
      toast.error(t("batch.title"), { description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={batchDialog} onOpenChange={setBatch}>
      <DialogContent className="w-[min(640px,calc(100vw-32px))]">
        <DialogHeader>
          <DialogTitle>{t("batch.title")}</DialogTitle>
          <DialogDescription>{t("batch.description")}</DialogDescription>
        </DialogHeader>
        <Textarea
          value={text}
          dir="ltr"
          rows={10}
          onChange={(e) => setText(e.target.value)}
          placeholder={t("batch.placeholder")}
          className="min-h-48 font-mono text-xs"
        />
        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>{t("batch.found", { count: urls.length })}</span>
          <Button variant="ghost" size="sm" onClick={() => fileRef.current?.click()}>
            <FileText /> {t("batch.importFile")}
          </Button>
          <input
            ref={fileRef}
            type="file"
            accept=".txt,.csv,.lst,text/plain"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) importFile(f);
              e.target.value = "";
            }}
          />
        </div>
        <div className="space-y-1.5">
          <Label>{t("add.saveTo")}</Label>
          <div className="flex gap-2">
            <Input value={dir} dir="ltr" onChange={(e) => setDir(e.target.value)} />
            <Button
              variant="secondary"
              onClick={async () => {
                const d = await open({ directory: true });
                if (typeof d === "string") setDir(d);
              }}
            >
              <FolderOpen /> {t("common.browse")}
            </Button>
          </div>
        </div>
        <label className="flex items-center gap-2 text-[13px]">
          <Checkbox checked={startNow} onCheckedChange={(v) => setStartNow(v === true)} />
          {t("batch.start")}
        </label>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setBatch(false)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={urls.length === 0 || busy} onClick={submit}>
            {busy && <Loader2 className="animate-spin" />}
            {t("batch.add", { count: urls.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
