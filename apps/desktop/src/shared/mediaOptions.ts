// Media quality options shared by the desktop app (Add Download dialog)
// and the browser extension (floating "Download This Video" menu).
// Pure functions; type imports are relative so both builds resolve them.

import type { MediaFormat } from "../bindings/MediaFormat";
import type { MediaProbeResult } from "../bindings/MediaProbeResult";
import type { OutputContainer } from "../bindings/OutputContainer";

/** A choice shown in the "Download This Video" menu. */
export interface MediaOption {
  label: string;
  detail: string;
  formatId: string | null;
  audioFormatId: string | null;
  container: OutputContainer;
  size: number | null;
  height: number | null;
}

function humanSize(bytes: number | null): string {
  if (!bytes) return "";
  const u = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1000 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u[i]}`;
}

function qualityLabel(f: MediaFormat): string {
  if (f.height) return `${f.height}p${f.fps && f.fps > 30 ? Math.round(f.fps) : ""}`;
  if (f.bitrate) return `${Math.round(f.bitrate / 1000)} kbps`;
  return f.label || "Default";
}

function codecShort(c: string | null): string | null {
  if (!c) return null;
  const l = c.toLowerCase();
  if (l.startsWith("avc") || l.startsWith("h264")) return "H.264";
  if (l.startsWith("hvc") || l.startsWith("hev") || l.startsWith("h265")) return "HEVC";
  if (l.startsWith("av01") || l === "av1") return "AV1";
  if (l.startsWith("vp9") || l.startsWith("vp09")) return "VP9";
  if (l.startsWith("mp4a") || l === "aac") return "AAC";
  if (l.startsWith("opus")) return "Opus";
  return null;
}

/**
 * Turn probe results into a short, sorted list of options:
 * every muxed rendition, one option per height for video-only renditions
 * (paired with the best audio), and one audio-only option.
 */
export function buildOptions(r: MediaProbeResult, audioOnlyLabel = "Audio only"): MediaOption[] {
  const formats = r.formats;
  const audio = formats.filter((f) => f.has_audio && !f.has_video).sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0));
  const bestAudio = audio[0] ?? null;
  const options: MediaOption[] = [];
  const seenHeights = new Set<string>();

  const muxed = formats.filter((f) => f.has_video && f.has_audio);
  const videoOnly = formats.filter((f) => f.has_video && !f.has_audio);

  for (const f of [...muxed].sort((a, b) => (b.height ?? 0) - (a.height ?? 0) || (b.bitrate ?? 0) - (a.bitrate ?? 0))) {
    const key = `m${f.height ?? f.id}`;
    if (seenHeights.has(key)) continue;
    seenHeights.add(key);
    const parts = [f.ext?.toUpperCase(), codecShort(f.vcodec), humanSize(f.filesize)].filter(Boolean);
    options.push({ label: qualityLabel(f), detail: parts.join(" · "), formatId: f.id, audioFormatId: null, container: "mp4", size: f.filesize, height: f.height });
  }
  // Best rendition per height for video-only formats.
  const byHeight = new Map<number, MediaFormat>();
  for (const f of videoOnly) {
    const h = f.height ?? 0;
    const cur = byHeight.get(h);
    if (!cur || (f.bitrate ?? 0) > (cur.bitrate ?? 0)) byHeight.set(h, f);
  }
  for (const f of [...byHeight.values()].sort((a, b) => (b.height ?? 0) - (a.height ?? 0))) {
    const key = `m${f.height ?? f.id}`;
    if (seenHeights.has(key)) continue;
    seenHeights.add(key);
    const size = f.filesize && bestAudio?.filesize ? f.filesize + bestAudio.filesize : f.filesize;
    const parts = [codecShort(f.vcodec), humanSize(size)].filter(Boolean);
    options.push({
      label: qualityLabel(f),
      detail: parts.join(" · "),
      formatId: f.id,
      audioFormatId: bestAudio?.id ?? null,
      container: "mp4",
      size,
      height: f.height,
    });
  }
  if (bestAudio) {
    const parts = [codecShort(bestAudio.acodec), bestAudio.bitrate ? `${Math.round(bestAudio.bitrate / 1000)} kbps` : null, humanSize(bestAudio.filesize)].filter(Boolean);
    options.push({ label: audioOnlyLabel, detail: parts.join(" · "), formatId: bestAudio.id, audioFormatId: null, container: "m4a", size: bestAudio.filesize, height: null });
  }
  if (options.length === 0 && formats.length > 0) {
    const f = formats[0];
    options.push({ label: qualityLabel(f), detail: humanSize(f.filesize), formatId: f.id, audioFormatId: null, container: f.has_video ? "mp4" : "m4a", size: f.filesize, height: f.height });
  }
  return options;
}
