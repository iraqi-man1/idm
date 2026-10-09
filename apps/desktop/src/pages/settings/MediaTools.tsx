import { CheckCircle2, Loader2, RefreshCw, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ToolStatus } from "@/bindings/ToolStatus";
import { Button } from "@/components/ui/button";
import { Section } from "@/components/ui/field";
import { api, errorMessage } from "@/lib/api";

/** Availability of the bundled FFmpeg, ffprobe and yt-dlp. */
export function MediaTools() {
  const { t } = useTranslation();
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    api
      .mediaTools()
      .then(setTools)
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false));
  }, []);
  useEffect(load, [load]);

  return (
    <Section
      title={
        <span className="flex items-center justify-between">
          {t("settings.media.tools")}
          <Button variant="ghost" size="sm" onClick={load} disabled={loading} aria-label={t("settings.media.toolsRefresh")}>
            {loading ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            {t("settings.media.toolsRefresh")}
          </Button>
        </span>
      }
    >
      <p className="py-2.5 text-xs text-muted-foreground">{t("settings.media.toolsHint")}</p>
      {error && <p className="py-2.5 text-xs text-danger">{error}</p>}
      {tools?.map((tool) => (
        <div key={tool.name} className="flex items-center gap-3 py-2.5 text-[13px]" data-testid={`tool-${tool.name}`}>
          {tool.available ? (
            <CheckCircle2 className="size-4 shrink-0 text-success" />
          ) : (
            <XCircle className="size-4 shrink-0 text-danger" />
          )}
          <div className="min-w-0 flex-1">
            <div className="font-medium">{tool.name}</div>
            <div className="truncate text-xs text-muted-foreground" dir="ltr" title={tool.path ?? undefined}>
              {tool.available ? tool.version : tool.error}
            </div>
          </div>
          <span className="shrink-0 text-xs text-muted-foreground">
            {!tool.available ? t("settings.media.toolMissing") : tool.bundled ? t("settings.media.toolBundled") : t("settings.media.toolSystem")}
          </span>
        </div>
      ))}
    </Section>
  );
}
