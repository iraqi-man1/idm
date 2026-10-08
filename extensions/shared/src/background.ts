// Background service worker (Chromium) / event page (Firefox).

import type { ExtBrowserDownload } from "@bindings/ExtBrowserDownload";
import type { ExtCaptureOrigin } from "@bindings/ExtCaptureOrigin";
import { initMedia, handleMediaMessage } from "./media";
import { BridgeError, NativeBridge } from "./native";
import { basename, cookieHeader, hostOf, RecentSet, shouldCapture, type CaptureConfig } from "./util";

const IS_FIREFOX = typeof (globalThis as { browser?: unknown }).browser !== "undefined" && !("onDeterminingFilename" in chrome.downloads);
const BROWSER_NAME = detectBrowser();

function detectBrowser(): string {
  const ua = navigator.userAgent;
  if (/Firefox\//.test(ua)) return "firefox";
  if (/Edg\//.test(ua)) return "edge";
  if (/OPR\//.test(ua)) return "opera";
  if ((navigator as { brave?: unknown }).brave) return "brave";
  if (/Vivaldi/.test(ua)) return "vivaldi";
  if (/Chrome\//.test(ua)) return "chrome";
  return "chromium";
}

export const bridge = new NativeBridge(BROWSER_NAME, chrome.runtime.getManifest().version);

interface LocalPrefs {
  captureEnabled: boolean;
  excludedSites: string[];
}

const DEFAULT_PREFS: LocalPrefs = { captureEnabled: true, excludedSites: [] };
let prefs: LocalPrefs = { ...DEFAULT_PREFS };
const bypass = new RecentSet(15_000);
const handled = new RecentSet(4_000);

async function loadPrefs() {
  const stored = await chrome.storage.local.get(["prefs"]);
  prefs = { ...DEFAULT_PREFS, ...(stored.prefs ?? {}) };
}

async function savePrefs() {
  await chrome.storage.local.set({ prefs });
}

function captureConfig(): CaptureConfig {
  const c = bridge.config;
  return {
    enabled: prefs.captureEnabled && (c?.capture_downloads ?? false),
    extensions: c?.capture_extensions ?? [],
    minSize: c?.min_capture_size ?? 0,
    excluded: [...(c?.excluded_sites ?? []), ...prefs.excludedSites],
  };
}

/** Publish state for the popup and content scripts. */
async function publishState() {
  await chrome.storage.local.set({
    bridgeState: bridge.state,
    appConfig: bridge.config,
  });
  const ok = bridge.state.kind === "connected" || bridge.state.kind === "app_not_running";
  await chrome.action.setBadgeText({ text: ok ? "" : "!" });
  await chrome.action.setBadgeBackgroundColor({ color: "#dc2626" });
}

async function cookiesFor(url: string): Promise<string | null> {
  try {
    const list = await chrome.cookies.getAll({ url });
    return cookieHeader(list);
  } catch {
    return null;
  }
}

/** Hand a download to Velox. Resolves true when Velox accepted it. */
export async function sendDownload(d: Omit<ExtBrowserDownload, "cookies" | "user_agent" | "headers"> & { headers?: ExtBrowserDownload["headers"] }): Promise<boolean> {
  const download: ExtBrowserDownload = {
    ...d,
    headers: d.headers ?? [],
    cookies: await cookiesFor(d.url),
    user_agent: navigator.userAgent,
  };
  try {
    const r = await bridge.request({ type: "add_download", download }, 30_000);
    return r.type === "added" && r.accepted;
  } catch (e) {
    console.warn("Velox: hand-over failed", (e as BridgeError).code, (e as Error).message);
    return false;
  }
}

async function handleBrowserDownload(item: chrome.downloads.DownloadItem): Promise<boolean> {
  const url = item.finalUrl || item.url;
  const size = item.fileSize > 0 ? item.fileSize : item.totalBytes > 0 ? item.totalBytes : null;
  const decision = shouldCapture(
    {
      url: item.url,
      finalUrl: item.finalUrl,
      filename: item.filename,
      mime: item.mime,
      fileSize: size,
      referrer: item.referrer,
      byExtensionId: item.byExtensionId,
      incognito: item.incognito,
      bypass: bypass.has(item.url) || bypass.has(url),
      recentlyHandled: handled.has(url),
    },
    captureConfig(),
  );
  console.debug("Velox: browser download", item.id, url, decision);
  if (!decision.capture) return false;
  handled.add(url);
  return sendDownload({
    url,
    referrer: item.referrer || null,
    page_url: item.referrer || null,
    file_name: basename(item.filename),
    mime: item.mime || null,
    file_size: size,
    interactive: true,
    origin: "capture",
  });
}

async function takeOver(id: number) {
  try {
    await chrome.downloads.cancel(id);
  } catch {
    /* already gone */
  }
  try {
    await chrome.downloads.erase({ id });
  } catch {
    /* ignore */
  }
}

if (!IS_FIREFOX && chrome.downloads.onDeterminingFilename) {
  // Chromium: decide before the file name / Save As dialog is shown.
  chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
    void handleBrowserDownload(item).then(async (accepted) => {
      if (accepted) await takeOver(item.id);
      else suggest();
    });
    return true; // suggest() is called asynchronously
  });
} else {
  // Firefox: pause, ask Velox, then cancel or resume.
  chrome.downloads.onCreated.addListener((item) => {
    if (item.state !== "in_progress") return;
    void (async () => {
      const cfg = captureConfig();
      if (!cfg.enabled) return;
      try {
        await chrome.downloads.pause(item.id);
      } catch {
        /* may already be finished */
      }
      const accepted = await handleBrowserDownload(item);
      if (accepted) await takeOver(item.id);
      else {
        try {
          await chrome.downloads.resume(item.id);
        } catch {
          /* ignore */
        }
      }
    })();
  });
}

// ----- context menus ---------------------------------------------------------

function createMenus() {
  chrome.contextMenus.removeAll(() => {
    const m = chrome.i18n.getMessage;
    chrome.contextMenus.create({ id: "velox-link", title: m("menuDownloadLink"), contexts: ["link"] });
    chrome.contextMenus.create({ id: "velox-media", title: m("menuDownloadMedia"), contexts: ["video", "audio"] });
    chrome.contextMenus.create({ id: "velox-image", title: m("menuDownloadImage"), contexts: ["image"] });
    chrome.contextMenus.create({ id: "velox-all", title: m("menuDownloadAll"), contexts: ["page", "selection"] });
  });
}

/** Runs in the page: collect link URLs (only the selection when there is one). */
function collectLinks(): string[] {
  const sel = window.getSelection();
  let anchors: HTMLAnchorElement[] = Array.from(document.querySelectorAll("a[href]"));
  if (sel && !sel.isCollapsed && sel.rangeCount > 0) {
    anchors = anchors.filter((a) => sel.containsNode(a, true));
  }
  const urls = anchors.map((a) => a.href).filter((h) => /^(https?|ftps?):\/\//i.test(h));
  return Array.from(new Set(urls));
}

chrome.contextMenus.onClicked.addListener((info, tab) => {
  void (async () => {
    const page = info.pageUrl ?? tab?.url ?? null;
    const send = (url: string, origin: ExtCaptureOrigin) =>
      sendDownload({ url, referrer: page, page_url: page, file_name: null, mime: null, file_size: null, interactive: true, origin });
    switch (info.menuItemId) {
      case "velox-link":
        if (info.linkUrl) await send(info.linkUrl, "context_menu");
        break;
      case "velox-image":
        if (info.srcUrl && !info.srcUrl.startsWith("data:")) await send(info.srcUrl, "context_menu");
        break;
      case "velox-media":
        if (info.srcUrl && /^https?:/.test(info.srcUrl)) await send(info.srcUrl, "video");
        else if (tab?.id !== undefined) await chrome.tabs.sendMessage(tab.id, { type: "velox:open-media-menu" }).catch(() => undefined);
        break;
      case "velox-all": {
        if (tab?.id === undefined) break;
        const [res] = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: collectLinks });
        const urls = (res?.result as string[] | undefined) ?? [];
        if (urls.length) {
          const items: ExtBrowserDownload[] = urls.slice(0, 2000).map((url) => ({
            url,
            referrer: page,
            page_url: page,
            file_name: null,
            mime: null,
            file_size: null,
            cookies: null,
            user_agent: null,
            headers: [],
            interactive: true,
            origin: "batch",
          }));
          await bridge.request({ type: "add_batch", items, page_url: page }).catch(() => undefined);
        }
        break;
      }
    }
  })();
});

// ----- messages from popup / content scripts ---------------------------------

type UiMessage =
  | { type: "velox:bypass"; url: string }
  | { type: "velox:status" }
  | { type: "velox:refresh" }
  | { type: "velox:set-capture"; enabled: boolean }
  | { type: "velox:set-site-excluded"; host: string; excluded: boolean }
  | { type: "velox:show-app" };

chrome.runtime.onMessage.addListener((msg: { type?: string }, sender, sendResponse) => {
  if (typeof msg?.type !== "string" || !msg.type.startsWith("velox:")) return false;
  if (msg.type.startsWith("velox:media")) {
    void handleMediaMessage(msg, sender).then(sendResponse, (e: Error) => sendResponse({ error: e.message }));
    return true;
  }
  const m = msg as UiMessage;
  void (async () => {
    switch (m.type) {
      case "velox:bypass":
        bypass.add(m.url);
        return { ok: true };
      case "velox:refresh":
        await bridge.hello();
        await publishState();
        return { state: bridge.state, config: bridge.config, prefs };
      case "velox:status":
        return { state: bridge.state, config: bridge.config, prefs };
      case "velox:set-capture":
        prefs.captureEnabled = m.enabled;
        await savePrefs();
        return { prefs };
      case "velox:set-site-excluded": {
        const host = m.host.toLowerCase();
        prefs.excludedSites = prefs.excludedSites.filter((h) => h !== host);
        if (m.excluded) prefs.excludedSites.push(host);
        await savePrefs();
        return { prefs };
      }
      case "velox:show-app":
        await bridge.request({ type: "show_app" }).catch(() => undefined);
        return { ok: true };
    }
  })().then(sendResponse, (e: Error) => sendResponse({ error: e.message }));
  return true;
});

// ----- lifecycle --------------------------------------------------------------

bridge.onChange(() => void publishState());

async function start() {
  await loadPrefs();
  await bridge.hello();
  await publishState();
}

chrome.runtime.onInstalled.addListener(() => createMenus());
chrome.runtime.onStartup.addListener(() => createMenus());
initMedia(bridge);
void start();

// Refresh the handshake periodically so the popup reflects app restarts.
setInterval(() => {
  if (bridge.state.kind !== "connected") void bridge.hello();
}, 30_000);

export { hostOf };
