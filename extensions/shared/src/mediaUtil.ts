// Pure media classification helpers (unit tested).

import type { MediaSourceKind } from "@bindings/MediaSourceKind";

export { cookieHeader } from "./util";

const DIRECT_EXT = ["mp4", "m4v", "webm", "mov", "mkv", "ogv", "m4a", "mp3", "ogg", "oga", "opus", "flac", "wav", "aac"];
/** Streaming fragments: never offered on their own. */
const SEGMENT_EXT = ["ts", "m4s", "cmfv", "cmfa", "aac.ts", "mp4a"];
const MIN_DIRECT_SIZE = 64 * 1024;

function pathExt(url: string): string | null {
  try {
    const p = new URL(url).pathname.toLowerCase();
    const base = p.split("/").pop() ?? "";
    const dot = base.lastIndexOf(".");
    return dot > 0 ? base.slice(dot + 1) : null;
  } catch {
    return null;
  }
}

/**
 * Classify a network response as a downloadable media resource.
 * Returns null for anything that is not a manifest or a complete media file.
 */
export function classifyMedia(url: string, mime: string | null, size: number | null): MediaSourceKind | null {
  if (!/^https?:/i.test(url)) return null;
  const m = (mime ?? "").split(";")[0].trim().toLowerCase();
  const ext = pathExt(url);
  if (ext === "m3u8" || m === "application/vnd.apple.mpegurl" || m === "application/x-mpegurl" || m === "audio/mpegurl" || m === "audio/x-mpegurl") {
    return "hls";
  }
  if (ext === "mpd" || m === "application/dash+xml") return "dash";
  if (ext && SEGMENT_EXT.includes(ext)) return null;
  if (m === "video/mp2t" || m === "video/iso.segment" || m === "audio/iso.segment") return null;
  const mediaMime = m.startsWith("video/") || m.startsWith("audio/");
  const mediaExt = ext !== null && DIRECT_EXT.includes(ext);
  if (!mediaMime && !(mediaExt && (m === "" || m === "application/octet-stream" || m === "binary/octet-stream"))) return null;
  if (size !== null && size < MIN_DIRECT_SIZE) return null;
  return "direct";
}

export { buildOptions, type MediaOption } from "@shared/mediaOptions";
