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
import type { PendingCapture } from "@/bindings/PendingCapture";
import type { ProgressSnapshot } from "@/bindings/ProgressSnapshot";
import type { StartMode } from "@/bindings/StartMode";
import type { StatsSummary } from "@/bindings/StatsSummary";
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
  quit: () => invoke<void>("quit_app"),
};

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
};
