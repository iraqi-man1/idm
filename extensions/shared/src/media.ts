// Media detection: observes network responses (read-only webRequest) to
// find video/audio files and HLS/DASH manifests per tab, and serves the
// floating "Download This Video" button and the popup.

import type { ExtMediaContext } from "@bindings/ExtMediaContext";
import type { MediaProbeResult } from "@bindings/MediaProbeResult";
import type { MediaRequest } from "@bindings/MediaRequest";
import type { MediaSourceKind } from "@bindings/MediaSourceKind";
import { BridgeError, type NativeBridge } from "./native";
import { classifyMedia, cookieHeader } from "./mediaUtil";

export interface MediaItem {
  url: string;
  kind: MediaSourceKind;
  mime: string | null;
  size: number | null;
  pageUrl: string | null;
  seen: number;
}

const MAX_PER_TAB = 60;
let bridge: NativeBridge | null = null;
const byTab = new Map<number, MediaItem[]>();

async function persist(tabId: number) {
  try {
    await chrome.storage.session.set({ [`media:${tabId}`]: byTab.get(tabId) ?? [] });
  } catch {
    /* storage.session unavailable */
  }
}

async function itemsFor(tabId: number): Promise<MediaItem[]> {
  if (!byTab.has(tabId)) {
    try {
      const stored = await chrome.storage.session.get(`media:${tabId}`);
      byTab.set(tabId, (stored[`media:${tabId}`] as MediaItem[] | undefined) ?? []);
    } catch {
      byTab.set(tabId, []);
    }
  }
  return byTab.get(tabId)!;
}

async function updateBadge(tabId: number) {
  const n = (byTab.get(tabId) ?? []).length;
  try {
    await chrome.action.setBadgeText({ tabId, text: n ? String(Math.min(n, 99)) : "" });
    await chrome.action.setBadgeBackgroundColor({ tabId, color: "#2563eb" });
  } catch {
    /* tab gone */
  }
}

function header(headers: chrome.webRequest.HttpHeader[] | undefined, name: string): string | null {
  const h = headers?.find((x) => x.name.toLowerCase() === name);
  return h?.value ?? null;
}

async function onResponse(d: chrome.webRequest.OnResponseStartedDetails) {
  if (d.tabId < 0 || d.statusCode >= 400) return;
  const mime = header(d.responseHeaders, "content-type");
  const len = header(d.responseHeaders, "content-length");
  const range = header(d.responseHeaders, "content-range");
  const total = range?.match(/\/(\d+)$/)?.[1] ?? len;
  const size = total ? Number(total) : null;
  const kind = classifyMedia(d.url, mime, size);
  if (!kind) return;
  const items = await itemsFor(d.tabId);
  const key = d.url.split("#")[0];
  const existing = items.find((i) => i.url === key);
  if (existing) {
    existing.seen = Date.now();
    if (size && !existing.size) existing.size = size;
  } else {
    let pageUrl: string | null = null;
    try {
      pageUrl = (await chrome.tabs.get(d.tabId)).url ?? null;
    } catch {
      /* ignore */
    }
    items.push({ url: key, kind, mime, size, pageUrl, seen: Date.now() });
    if (items.length > MAX_PER_TAB) items.splice(0, items.length - MAX_PER_TAB);
    chrome.tabs.sendMessage(d.tabId, { type: "velox:media-updated" }).catch(() => undefined);
  }
  await persist(d.tabId);
  await updateBadge(d.tabId);
}

export function initMedia(b: NativeBridge) {
  bridge = b;
  chrome.webRequest.onResponseStarted.addListener(
    (d) => void onResponse(d),
    { urls: ["http://*/*", "https://*/*"], types: ["media", "xmlhttprequest", "other", "object", "sub_frame"] },
    ["responseHeaders"],
  );
  chrome.tabs.onRemoved.addListener((tabId) => {
    byTab.delete(tabId);
    void chrome.storage.session?.remove(`media:${tabId}`).catch(() => undefined);
  });
  chrome.tabs.onUpdated.addListener((tabId, change) => {
    // A new page in the tab: forget media of the previous page.
    if (change.url) {
      byTab.set(tabId, []);
      void persist(tabId);
      void updateBadge(tabId);
    }
  });
}

async function context(url: string, kind: MediaSourceKind, pageUrl: string | null, title: string | null): Promise<ExtMediaContext> {
  let cookies: string | null = null;
  try {
    cookies = cookieHeader(await chrome.cookies.getAll({ url }));
  } catch {
    /* no cookie permission for this URL */
  }
  return { url, kind, page_url: pageUrl, referrer: pageUrl, cookies, user_agent: navigator.userAgent, title };
}

export interface Candidate {
  url: string;
  kind: MediaSourceKind;
  size: number | null;
}

/** Media sources worth offering for a video element on a page. */
async function candidates(tabId: number, src: string | null, pageUrl: string | null): Promise<Candidate[]> {
  const out: Candidate[] = [];
  const seen = new Set<string>();
  const push = (c: Candidate) => {
    if (!seen.has(c.url)) {
      seen.add(c.url);
      out.push(c);
    }
  };
  if (src && /^https?:/i.test(src)) push({ url: src, kind: classifyMedia(src, null, null) ?? "direct", size: null });
  const items = [...(await itemsFor(tabId))].sort((a, b) => b.seen - a.seen);
  for (const i of items.filter((x) => x.kind === "hls" || x.kind === "dash")) push({ url: i.url, kind: i.kind, size: i.size });
  for (const i of items.filter((x) => x.kind === "direct")) push({ url: i.url, kind: "direct", size: i.size });
  if (pageUrl && /^https?:/i.test(pageUrl)) push({ url: pageUrl, kind: "page", size: null });
  return out;
}

type MediaMsg =
  | { type: "velox:media-list"; tabId?: number }
  | { type: "velox:media-candidates"; src: string | null }
  | { type: "velox:media-probe"; url: string; kind: MediaSourceKind; pageUrl: string | null; title: string | null }
  | { type: "velox:media-download"; url: string; kind: MediaSourceKind; pageUrl: string | null; title: string | null; selection: MediaRequest };

export async function handleMediaMessage(msg: { type?: string }, sender: chrome.runtime.MessageSender): Promise<unknown> {
  if (!bridge) throw new Error("not initialised");
  const m = msg as MediaMsg;
  const tabId = ("tabId" in m && m.tabId !== undefined ? m.tabId : sender.tab?.id) ?? -1;
  switch (m.type) {
    case "velox:media-list":
      return { items: await itemsFor(tabId) };
    case "velox:media-candidates":
      return { candidates: await candidates(tabId, m.src, sender.tab?.url ?? null) };
    case "velox:media-probe": {
      try {
        const r = await bridge.request({ type: "probe_media", media: await context(m.url, m.kind, m.pageUrl, m.title) }, 90_000);
        if (r.type !== "media") throw new BridgeError("protocol", "unexpected reply");
        return { result: r.result as MediaProbeResult };
      } catch (e) {
        return { error: (e as Error).message, code: (e as BridgeError).code };
      }
    }
    case "velox:media-download": {
      try {
        const r = await bridge.request(
          { type: "download_media", media: await context(m.url, m.kind, m.pageUrl, m.title), selection: m.selection, interactive: false },
          30_000,
        );
        return { ok: r.type === "added" && r.accepted };
      } catch (e) {
        return { error: (e as Error).message, code: (e as BridgeError).code };
      }
    }
  }
  return { error: "unknown message" };
}
