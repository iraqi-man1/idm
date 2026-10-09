// Pure helpers for filtering and sorting the download list.

import type { Category } from "@/bindings/Category";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import type { DownloadStatus } from "@/bindings/DownloadStatus";
import type { ProgressSnapshot } from "@/bindings/ProgressSnapshot";

export type StatusFilter = "all" | "downloading" | "completed" | "paused" | "queued" | "scheduled" | "failed";
export type ViewFilter = StatusFilter | `cat:${Category}` | `queue:${string}`;

export type SortKey =
  | "name"
  | "size"
  | "downloaded"
  | "progress"
  | "speed"
  | "eta"
  | "connections"
  | "status"
  | "destination"
  | "date_added";

export interface SortSpec {
  key: SortKey;
  dir: "asc" | "desc";
}

export const ACTIVE_STATUSES: DownloadStatus[] = ["connecting", "downloading", "retrying", "processing"];

export function isActive(s: DownloadStatus): boolean {
  return ACTIVE_STATUSES.includes(s);
}

/** Row data: persistent info overlaid with the latest live progress. */
export interface Row extends DownloadInfo {
  segments: ProgressSnapshot["segments"];
  stage: string | null;
}

export function mergeRow(d: DownloadInfo, p: ProgressSnapshot | undefined): Row {
  if (!p) return { ...d, segments: [], stage: null };
  return {
    ...d,
    status: isActive(d.status) ? p.status : d.status,
    downloaded: Math.max(d.downloaded, p.downloaded),
    total_size: p.total_size ?? d.total_size,
    speed: p.speed,
    avg_speed: p.avg_speed,
    eta_secs: p.eta_secs,
    active_connections: p.active_connections,
    elapsed_ms: p.elapsed_ms,
    segments: p.segments,
    stage: p.stage,
  };
}

export function matchesFilter(d: DownloadInfo, f: ViewFilter): boolean {
  if (f.startsWith("cat:")) return d.category === f.slice(4);
  if (f.startsWith("queue:")) return d.queue_id === f.slice(6) && d.status !== "completed";
  switch (f) {
    case "all":
      return true;
    case "downloading":
      return isActive(d.status);
    case "completed":
      return d.status === "completed";
    case "paused":
      return d.status === "paused" || d.status === "cancelled";
    case "queued":
      return d.status === "queued" && d.scheduled_at === null;
    case "scheduled":
      return d.scheduled_at !== null && d.status !== "completed";
    case "failed":
      return d.status === "failed";
  }
  return true;
}

export function matchesSearch(d: DownloadInfo, q: string): boolean {
  if (!q) return true;
  const needle = q.toLocaleLowerCase();
  return (
    d.file_name.toLocaleLowerCase().includes(needle) ||
    d.url.toLocaleLowerCase().includes(needle) ||
    d.save_dir.toLocaleLowerCase().includes(needle)
  );
}

export function fraction(d: Pick<DownloadInfo, "downloaded" | "total_size" | "status">): number | null {
  if (d.status === "completed") return 1;
  if (d.total_size === null) return null;
  if (d.total_size === 0) return 1;
  return Math.min(1, d.downloaded / d.total_size);
}

const STATUS_ORDER: Record<DownloadStatus, number> = {
  downloading: 0,
  connecting: 1,
  retrying: 2,
  processing: 3,
  queued: 4,
  paused: 5,
  failed: 6,
  cancelled: 7,
  completed: 8,
};

function keyOf(d: Row, key: SortKey): number | string {
  switch (key) {
    case "name":
      return d.file_name.toLocaleLowerCase();
    case "size":
      return d.total_size ?? -1;
    case "downloaded":
      return d.downloaded;
    case "progress":
      return fraction(d) ?? -1;
    case "speed":
      return d.speed;
    case "eta":
      return d.eta_secs ?? Number.MAX_SAFE_INTEGER;
    case "connections":
      return d.active_connections;
    case "status":
      return STATUS_ORDER[d.status];
    case "destination":
      return d.save_dir.toLocaleLowerCase();
    case "date_added":
      return d.created_at;
  }
}

export function sortRows(rows: Row[], spec: SortSpec): Row[] {
  const dir = spec.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const ka = keyOf(a, spec.key);
    const kb = keyOf(b, spec.key);
    if (ka < kb) return -dir;
    if (ka > kb) return dir;
    return b.created_at - a.created_at;
  });
}

/** Counts shown in the sidebar. */
export function countByFilter(items: DownloadInfo[], filters: ViewFilter[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const f of filters) out[f] = 0;
  for (const d of items) for (const f of filters) if (matchesFilter(d, f)) out[f]++;
  return out;
}
