import { AlertTriangle, Film, Loader2, Music, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MediaProbeResult } from "@/bindings/MediaProbeResult";
import type { MediaRequest } from "@/bindings/MediaRequest";
import type { MediaSourceKind } from "@/bindings/MediaSourceKind";
import type { OutputContainer } from "@/bindings/OutputContainer";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { api, errorMessage } from "@/lib/api";
import { cn } from "@/lib/utils";
import { buildOptions } from "@/shared/mediaOptions";
import { useSettings } from "@/stores/settings";

const AUDIO: OutputContainer[] = ["m4a", "mp3"];
const VIDEO: OutputContainer[] = ["mp4", "mkv", "original"];

export interface MediaChoice {
  /** The selection, or null while nothing downloadable is chosen. */
  request: MediaRequest | null;
  title: string | null;
  /** DRM or a live stream: the address cannot be downloaded. */
  blocked: boolean;
  /** Estimated size of the chosen rendition. */
  size: number | null;
}

/**
 * Quality / format / subtitle picker for a streaming manifest or a media
 * page in the Add Download dialog. Manifests are probed immediately; pages
 * only when the user asks (yt-dlp is slower and the page may be a plain
 * file the user wants as is).
 */
export function MediaPicker({
  url,
  kind,
  referer,
  cookies,
  onChange,
}: {
  url: string;
  kind: MediaSourceKind;
  referer: string | null;
  cookies: string | null;
  onChange: (c: MediaChoice) => void;
}) {
  const { t } = useTranslation();
  const media = useSettings((s) => s.settings?.media);
  const [requested, setRequested] = useState(kind !== "page");
  const [result, setResult] = useState<MediaProbeResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [choice, setChoice] = useState(0);
  const [container, setContainer] = useState<OutputContainer>("mp4");
  const [subs, setSubs] = useState<string[]>([]);
  const [embed, setEmbed] = useState(true);
  /** Explicit audio track; null = the option's default (best) track. */
  const [audioId, setAudioId] = useState<string | null>(null);

  useEffect(() => {
    setRequested(kind !== "page");
    setResult(null);
    setError(null);
  }, [url, kind]);

  useEffect(() => {
    if (!requested) return;
    let alive = true;
    setLoading(true);
    setError(null);
    api
      .probeMedia(url, kind, referer, cookies)
      .then((r) => {
        if (!alive) return;
        setResult(r);
        setChoice(0);
        setAudioId(null);
        const wanted = new Set(media?.download_subtitles ? media.subtitle_languages.map((l) => l.toLowerCase()) : []);
        setSubs(r.subtitles.filter((s) => wanted.has(s.language.toLowerCase())).map((s) => s.language));
        setEmbed(media?.embed_subtitles ?? true);
      })
      .catch((e) => alive && setError(errorMessage(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requested, url, kind, referer, cookies]);

  const options = useMemo(() => (result ? buildOptions(result, t("add.audioOnly")) : []), [result, t]);
  const option = options[choice] ?? null;
  const audioOnly = option?.container === "m4a";
  const blocked = !!result && (result.drm_protected || result.is_live);
  const audioTracks = useMemo(() => result?.formats.filter((f) => f.has_audio && !f.has_video) ?? [], [result]);
  // A track choice applies to audio-only options and to video paired with separate audio.
  const canPickAudio = audioTracks.length > 1 && !!option && (audioOnly || option.audioFormatId !== null);
  const trackId = canPickAudio ? audioId ?? (audioOnly ? option!.formatId : option!.audioFormatId) : null;

  // Default output format for the chosen option.
  useEffect(() => {
    if (!option) return;
    const preferred = media?.output_container ?? "mp4";
    if (audioOnly) setContainer(AUDIO.includes(preferred) ? preferred : "m4a");
    else setContainer(preferred);
  }, [option, audioOnly, media?.output_container]);

  useEffect(() => {
    if (!result || !option || blocked) {
      onChange({ request: null, title: result?.title ?? null, blocked, size: null });
      return;
    }
    onChange({
      request: {
        kind,
        url: result.url || url,
        format_id: audioOnly && trackId ? trackId : option.formatId,
        audio_format_id: !audioOnly && trackId ? trackId : option.audioFormatId,
        subtitle_languages: AUDIO.includes(container) ? [] : subs,
        embed_subtitles: embed && (container === "mp4" || container === "mkv"),
        container,
        title: result.title,
        max_height: option.height,
      },
      title: result.title,
      blocked: false,
      size: option.size,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [result, option, container, subs, embed, blocked, trackId]);

  if (!requested) {
    return (
      <div className="flex items-center gap-3 rounded-lg border border-border bg-surface-2 p-3 text-[13px]">
        <Film className="size-5 shrink-0 text-muted-foreground" />
        <p className="flex-1 text-muted-foreground">{t("add.mediaPage")}</p>
        <Button variant="secondary" size="sm" onClick={() => setRequested(true)}>
          <Search />
          {t("add.findVideos")}
        </Button>
      </div>
    );
  }
  if (loading) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 p-3 text-[13px] text-muted-foreground">
        <Loader2 className="size-4 animate-spin" /> {t("add.mediaLoading")}
      </div>
    );
  }
  if (error || blocked || (result && options.length === 0)) {
    const text = error
      ? t("add.mediaFailed", { error })
      : result?.drm_protected
        ? t("add.mediaDrm")
        : result?.is_live
          ? t("add.mediaLive")
          : t("add.mediaNone");
    return (
      <div
        role="alert"
        className={cn(
          "flex items-start gap-2 rounded-md px-3 py-2 text-xs",
          error || blocked ? "bg-danger-soft text-danger" : "bg-warning-soft text-warning",
        )}
      >
        <AlertTriangle className="mt-px size-4 shrink-0" /> {text}
      </div>
    );
  }
  if (!result) return null;

  return (
    <div className="space-y-3 rounded-lg border border-border p-3" data-testid="media-picker">
      <div className="space-y-1.5">
        <Label>{t("add.mediaQuality")}</Label>
        <div role="radiogroup" aria-label={t("add.mediaQuality")} className="grid max-h-44 gap-1 overflow-y-auto">
          {options.map((o, i) => (
            <button
              key={`${o.formatId}-${o.audioFormatId}-${i}`}
              type="button"
              role="radio"
              aria-checked={i === choice}
              onClick={() => setChoice(i)}
              className={cn(
                "flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-start text-[13px]",
                i === choice ? "border-primary bg-primary-soft" : "border-border hover:bg-accent",
              )}
            >
              {o.container === "m4a" ? <Music className="size-4 text-muted-foreground" /> : <Film className="size-4 text-muted-foreground" />}
              <b className="tabular">{o.label}</b>
              <span className="truncate text-xs text-muted-foreground" dir="ltr">
                {o.detail}
              </span>
            </button>
          ))}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
        {canPickAudio && trackId && (
          <div className="flex items-center gap-2">
            <Label>{t("add.mediaAudioTrack")}</Label>
            <Select
              value={trackId}
              onChange={setAudioId}
              className="min-w-36"
              ariaLabel={t("add.mediaAudioTrack")}
              options={audioTracks.map((f) => ({ value: f.id, label: f.language ? `${f.label} (${f.language})` : f.label }))}
            />
          </div>
        )}
        <div className="flex items-center gap-2">
          <Label>{t("add.mediaSaveAs")}</Label>
          <Select
            value={container}
            onChange={setContainer}
            className="min-w-28"
            ariaLabel={t("add.mediaSaveAs")}
            options={[...(audioOnly ? [] : VIDEO), ...AUDIO].map((c) => ({
              value: c,
              label: c === "original" ? t("add.keepOriginal") : AUDIO.includes(c) && !audioOnly ? `${c.toUpperCase()} (${t("add.audioOnly")})` : c.toUpperCase(),
            }))}
          />
        </div>
      </div>
      {result.subtitles.length > 0 && !AUDIO.includes(container) && (
        <div className="space-y-1.5">
          <Label>{t("add.mediaSubtitles")}</Label>
          <div className="flex flex-wrap gap-x-4 gap-y-1.5">
            {result.subtitles.slice(0, 24).map((s) => (
              <label key={s.language} className="flex items-center gap-1.5 text-[13px]">
                <Checkbox
                  checked={subs.includes(s.language)}
                  onCheckedChange={(v) =>
                    setSubs((cur) => (v === true ? [...cur, s.language] : cur.filter((x) => x !== s.language)))
                  }
                />
                <span dir="auto">{s.name ? `${s.name} (${s.language})` : s.language}</span>
              </label>
            ))}
          </div>
          {subs.length > 0 && (container === "mp4" || container === "mkv") && (
            <label className="flex items-center gap-1.5 text-[13px]">
              <Checkbox checked={embed} onCheckedChange={(v) => setEmbed(v === true)} />
              {t("add.mediaEmbed")}
            </label>
          )}
        </div>
      )}
    </div>
  );
}
