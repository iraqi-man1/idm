// Typed wrappers around the Tauri commands exposed by src-tauri.
// Payload types are generated from the Rust crate `velox-types`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AddDownloadRequest } from "@/bindings/AddDownloadRequest";
import type { AppInfo } from "@/bindings/AppInfo";
import type { AppSettings } from "@/bindings/AppSettings";
import type { BrowserIntegrationStatus } from "@/bindings/BrowserIntegrationStatus";
import type { BatchAddResult } from "@/bindings/BatchAddResult";
import type { ChecksumAlgorithm } from "@/bindings/ChecksumAlgorithm";
import type { ChecksumResult } from "@/bindings/ChecksumResult";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import type { EngineEvent } from "@/bindings/EngineEvent";
import type { LogLine } from "@/bindings/LogLine";
import type { MediaProbeResult } from "@/bindings/MediaProbeResult";
import type { MediaSourceKind } from "@/bindings/MediaSourceKind";
import type { PendingCapture } from "@/bindings/PendingCapture";
import type { PostAction } from "@/bindings/PostAction";
import type { PowerHold } from "@/bindings/PowerHold";
import type { ProgressSnapshot } from "@/bindings/ProgressSnapshot";
import type { QueueInfo } from "@/bindings/QueueInfo";
import type { QueueUpdate } from "@/bindings/QueueUpdate";
import type { SchedulerEvent } from "@/bindings/SchedulerEvent";
import type { StartMode } from "@/bindings/StartMode";
import type { StatsSummary } from "@/bindings/StatsSummary";
import type { ToolStatus } from "@/bindings/ToolStatus";
import type { UrlInfo } from "@/bindings/UrlInfo";

/** Error object returned by every command (see src-tauri/src/error.rs). */
export interface CommandError {
  code: string;
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as CommandError).message);
  return String(e);
}

export const api = {
  listDownloads: () => invoke<DownloadInfo[]>("list_downloads"),
  getDownload: (id: string) => invoke<DownloadInfo>("get_download", { id }),
  probeUrl: (request: AddDownloadRequest) => invoke<UrlInfo>("probe_url", { request }),
  addDownload: (request: AddDownloadRequest) => invoke<DownloadInfo>("add_download", { request }),
  addBatch: (
    urls: string[],
    saveDir: string | null,
    start: StartMode,
    queueId: string | null = null,
    referer: string | null = null,
  ) => invoke<BatchAddResult>("add_batch", { urls, saveDir, start, queueId, referer }),
  start: (id: string) => invoke<void>("start_download", { id }),
  pause: (id: string) => invoke<void>("pause_download", { id }),
  cancel: (id: string) => invoke<void>("cancel_download", { id }),
  restart: (id: string) => invoke<void>("restart_download", { id }),
  remove: (ids: string[], deleteFiles: boolean) => invoke<void>("remove_downloads", { ids, deleteFiles }),
  startMany: (ids: string[]) => invoke<void>("start_many", { ids }),
  pauseMany: (ids: string[]) => invoke<void>("pause_many", { ids }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  openFile: (id: string) => invoke<void>("open_file", { id }),
  openFolder: (id: string) => invoke<void>("open_folder", { id }),
  move: (id: string, directory: string) => invoke<DownloadInfo>("move_download", { id, directory }),
  rename: (id: string, name: string) => invoke<DownloadInfo>("rename_download", { id, name }),
  updateUrl: (id: string, url: string) => invoke<void>("update_download_url", { id, url }),
  setSpeedLimit: (id: string, limit: number) => invoke<void>("set_download_speed_limit", { id, limit }),
  setConnections: (id: string, connections: number) =>
    invoke<DownloadInfo>("set_download_connections", { id, connections }),
  verifyChecksum: (id: string, expected: string, algorithm: ChecksumAlgorithm | null = null) =>
    invoke<ChecksumResult>("verify_checksum", { id, algorithm, expected }),
  log: (id: string) => invoke<LogLine[]>("get_download_log", { id }),
  speedHistory: (id: string) => invoke<number[]>("get_speed_history", { id }),
  progress: () => invoke<ProgressSnapshot[]>("get_progress"),
  findDuplicates: (url: string) => invoke<string[]>("find_duplicates", { url }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) => invoke<AppSettings>("update_settings", { settings }),
  setProxyPassword: (password: string | null) => invoke<AppSettings>("set_proxy_password", { password }),
  appInfo: () => invoke<AppInfo>("get_app_info"),
  stats: () => invoke<StatsSummary>("get_stats"),
  defaultDownloadDir: () => invoke<string>("default_download_dir"),
  openProgressWindow: (id: string) => invoke<void>("open_progress_window", { id }),
  showMain: () => invoke<void>("show_main"),
  browserStatus: () => invoke<BrowserIntegrationStatus>("browser_integration_status"),
  repairBrowserIntegration: () => invoke<BrowserIntegrationStatus>("repair_browser_integration"),
  openExtensionFolder: (family: "chromium" | "firefox") => invoke<void>("open_extension_folder", { family }),
  getPendingCapture: (id: string) => invoke<PendingCapture>("get_pending_capture", { id }),
  probeCapture: (id: string) => invoke<UrlInfo>("probe_capture", { id }),
  resolveCapture: (id: string, request: AddDownloadRequest | null) =>
    invoke<DownloadInfo | null>("resolve_capture", { id, request }),
  mediaTools: () => invoke<ToolStatus[]>("get_media_tools"),
  probeMedia: (url: string, kind: MediaSourceKind, referer: string | null = null, cookies: string | null = null) =>
    invoke<MediaProbeResult>("probe_media", { url, kind, referer, cookies }),
  queues: () => invoke<QueueInfo[]>("list_queues"),
  createQueue: (name: string) => invoke<QueueInfo>("create_queue", { name }),
  updateQueue: (id: string, update: Partial<QueueUpdate>) =>
    invoke<QueueInfo>("update_queue", { id, update: { ...EMPTY_QUEUE_UPDATE, ...update } }),
  deleteQueue: (id: string) => invoke<void>("delete_queue", { id }),
  startQueue: (id: string) => invoke<void>("start_queue", { id }),
  stopQueue: (id: string) => invoke<void>("stop_queue", { id }),
  setDownloadQueue: (ids: string[], queueId: string) => invoke<void>("set_download_queue", { ids, queueId }),
  powerHold: () => invoke<PowerHold | null>("get_power_hold"),
  cancelPostAction: () => invoke<void>("cancel_post_action"),
  runPostActionNow: () => invoke<void>("run_post_action_now"),
  quit: () => invoke<void>("quit_app"),
};

const EMPTY_QUEUE_UPDATE: QueueUpdate = { name: null, max_concurrent: null, schedule: null, post_action: null, retry_failed: null };

export interface PostActionNotice {
  action: PostAction;
  queue: string;
  seconds: number;
}

export const events = {
  engine: (cb: (e: EngineEvent) => void): Promise<UnlistenFn> =>
    listen<EngineEvent>("engine://event", (ev) => cb(ev.payload)),
  settingsChanged: (cb: (s: AppSettings) => void): Promise<UnlistenFn> =>
    listen<AppSettings>("app://settings-changed", (ev) => cb(ev.payload)),
  addUrl: (cb: (url: string) => void): Promise<UnlistenFn> =>
    listen<string>("app://add-url", (ev) => cb(ev.payload)),
  batchUrls: (cb: (p: { urls: string[]; referer: string | null }) => void): Promise<UnlistenFn> =>
    listen<{ urls: string[]; referer: string | null }>("app://batch-urls", (ev) => cb(ev.payload)),
  browserStatusChanged: (cb: () => void): Promise<UnlistenFn> => listen("app://browser-status-changed", () => cb()),
  scheduler: (cb: (e: SchedulerEvent) => void): Promise<UnlistenFn> =>
    listen<SchedulerEvent>("app://scheduler", (ev) => cb(ev.payload)),
  postAction: (cb: (n: PostActionNotice) => void): Promise<UnlistenFn> =>
    listen<PostActionNotice>("app://post-action", (ev) => cb(ev.payload)),
  postActionCancelled: (cb: () => void): Promise<UnlistenFn> => listen("app://post-action-cancelled", () => cb()),
  postActionFailed: (cb: (error: string) => void): Promise<UnlistenFn> =>
    listen<string>("app://post-action-failed", (ev) => cb(ev.payload)),
};
