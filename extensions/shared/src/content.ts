// Content script:
// * Alt+click on a link lets the browser download it (bypasses Velox).
// * Shows a floating "Download This Video" button over HTML5 videos and a
//   quality menu fed by the desktop app (HLS/DASH/direct/extractor probes).
//
// All UI lives in closed shadow roots so page styles cannot interfere, and
// styles use constructable stylesheets (not subject to page CSP).

import type { ExtBrowserConfig } from "@bindings/ExtBrowserConfig";
import type { MediaProbeResult } from "@bindings/MediaProbeResult";
import type { MediaRequest } from "@bindings/MediaRequest";
import type { MediaSourceKind } from "@bindings/MediaSourceKind";
import { buildOptions, type MediaOption } from "./mediaUtil";

const msg = (k: string, ...subs: string[]) => chrome.i18n.getMessage(k, subs) || k;

// ----- Alt+click bypass -------------------------------------------------------

document.addEventListener(
  "click",
  (e) => {
    if (!e.altKey) return;
    const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (a?.href) chrome.runtime.sendMessage({ type: "velox:bypass", url: a.href }).catch(() => undefined);
  },
  true,
);

// ----- configuration ----------------------------------------------------------

let config: ExtBrowserConfig | null = null;
let hiddenOnPage = false;

async function loadConfig() {
  try {
    const s = await chrome.storage.local.get(["appConfig"]);
    config = (s.appConfig as ExtBrowserConfig | null) ?? null;
  } catch {
    config = null;
  }
}

function buttonEnabled(): boolean {
  return !hiddenOnPage && !!config?.video_detection && !!config?.floating_button;
}

chrome.storage.onChanged.addListener((changes) => {
  if (changes.appConfig) {
    config = (changes.appConfig.newValue as ExtBrowserConfig | null) ?? null;
    scan();
  }
});

// ----- styles -----------------------------------------------------------------

const CSS = `
:host { all: initial; }
.wrap { position: fixed; top: 0; left: 0; z-index: 2147483647; pointer-events: none;
  font: 13px/1.35 system-ui, -apple-system, "Segoe UI", Roboto, "Noto Sans Arabic", sans-serif; }
.btn { pointer-events: auto; display: inline-flex; align-items: center; gap: 6px; cursor: pointer; white-space: nowrap;
  padding: 6px 12px 6px 9px; border: 1px solid rgba(255,255,255,.18); border-radius: 999px;
  background: rgba(17, 24, 39, .88); color: #fff; font: inherit; font-weight: 600;
  box-shadow: 0 6px 20px rgba(0,0,0,.35); backdrop-filter: blur(6px);
  opacity: 0; transform: translateY(-4px); transition: opacity .15s, transform .15s, background .15s; }
.btn.show { opacity: 1; transform: none; }
.btn:hover { background: rgba(37, 99, 235, .95); }
.btn svg { width: 16px; height: 16px; flex: none; }
.menu { pointer-events: auto; position: absolute; top: 38px; inset-inline-end: 0; min-width: 260px; max-width: 360px;
  max-height: 360px; overflow: auto; background: #fff; color: #111827; border-radius: 12px;
  box-shadow: 0 16px 40px rgba(0,0,0,.35); border: 1px solid rgba(0,0,0,.08); padding: 6px; }
.menu h4 { margin: 4px 8px 6px; font-size: 12px; font-weight: 600; color: #6b7280; text-transform: uppercase; letter-spacing: .03em; }
.item { display: flex; align-items: baseline; gap: 10px; width: 100%; text-align: start; border: 0; background: none;
  padding: 7px 8px; border-radius: 8px; cursor: pointer; font: inherit; color: inherit; }
.item:hover, .item:focus-visible { background: #eef2ff; outline: none; }
.item b { font-weight: 600; min-width: 64px; }
.item span { color: #6b7280; font-size: 12px; }
.note { padding: 8px; color: #6b7280; font-size: 12px; }
.err { color: #b91c1c; }
.sep { height: 1px; background: #e5e7eb; margin: 6px 4px; }
.foot { display: flex; justify-content: space-between; gap: 6px; padding: 4px; }
.link { border: 0; background: none; color: #2563eb; cursor: pointer; font: inherit; font-size: 12px; padding: 4px; }
.toast { pointer-events: none; position: absolute; top: 38px; inset-inline-end: 0; background: rgba(21,128,61,.95);
  color: #fff; padding: 6px 10px; border-radius: 8px; font-weight: 600; white-space: nowrap; }
.spin { width: 14px; height: 14px; border: 2px solid #c7d2fe; border-top-color: #2563eb; border-radius: 50%;
  display: inline-block; animation: s .8s linear infinite; vertical-align: -2px; margin-inline-end: 6px; }
@keyframes s { to { transform: rotate(360deg); } }
@media (prefers-color-scheme: dark) {
  .menu { background: #1f2937; color: #f3f4f6; border-color: rgba(255,255,255,.08); }
  .item:hover, .item:focus-visible { background: #374151; }
  .item span, .note, .menu h4 { color: #9ca3af; }
  .sep { background: #374151; }
}
@media (prefers-reduced-motion: reduce) { .btn { transition: none; } .spin { animation: none; } }
`;

/** Download icon built with DOM APIs (no innerHTML: safe under Trusted Types). */
function icon(): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2.4");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  for (const d of ["M12 3v12", "m6 10 6 6 6-6", "M5 21h14"]) {
    const p = document.createElementNS(ns, "path");
    p.setAttribute("d", d);
    svg.append(p);
  }
  return svg;
}

function spinnerNote(text: string): HTMLDivElement {
  const n = document.createElement("div");
  n.className = "note";
  const s = document.createElement("span");
  s.className = "spin";
  n.append(s, text);
  return n;
}

let sheet: CSSStyleSheet | null = null;
function adoptStyles(root: ShadowRoot) {
  try {
    if (!sheet) {
      sheet = new CSSStyleSheet();
      sheet.replaceSync(CSS);
    }
    root.adoptedStyleSheets = [sheet];
  } catch {
    const style = document.createElement("style");
    style.textContent = CSS;
    root.appendChild(style);
  }
}

// ----- overlay per video ------------------------------------------------------

const MIN_W = 200;
const MIN_H = 110;
const overlays = new Map<HTMLVideoElement, Overlay>();

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

interface Candidate {
  url: string;
  kind: MediaSourceKind;
  size: number | null;
}

class Overlay {
  readonly host: HTMLDivElement;
  private readonly wrap: HTMLDivElement;
  private readonly button: HTMLButtonElement;
  private menu: HTMLDivElement | null = null;
  private visibleUntil = 0;
  private readonly root: ShadowRoot;

  constructor(readonly video: HTMLVideoElement) {
    this.host = document.createElement("div");
    this.host.setAttribute("data-velox", "");
    this.root = this.host.attachShadow({ mode: "closed" });
    adoptStyles(this.root);
    this.wrap = el("div", "wrap");
    this.button = el("button", "btn");
    this.button.type = "button";
    this.button.append(icon(), el("span", undefined, msg("downloadThisVideo")));
    this.button.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (this.menu) this.closeMenu();
      else void this.openMenu();
    });
    this.wrap.append(this.button);
    this.root.append(this.wrap);
    this.mount();
    video.addEventListener("play", () => this.reveal(4000));
  }

  /** Attach to the fullscreen element when the video is inside it. */
  mount() {
    const fs = document.fullscreenElement;
    const parent = fs && fs !== this.video && fs.contains(this.video) ? fs : document.documentElement;
    if (this.host.parentNode !== parent) parent.appendChild(this.host);
  }

  reveal(ms: number) {
    this.visibleUntil = Math.max(this.visibleUntil, performance.now() + ms);
    requestFrame();
  }

  /** Returns true while the overlay needs further animation frames. */
  update(now: number): boolean {
    const r = this.video.getBoundingClientRect();
    const onScreen = r.width >= MIN_W && r.height >= MIN_H && r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
    const show = buttonEnabled() && onScreen && (this.menu !== null || now < this.visibleUntil);
    this.button.classList.toggle("show", show);
    this.wrap.style.visibility = onScreen && buttonEnabled() ? "visible" : "hidden";
    if (onScreen) {
      const bw = this.button.offsetWidth || 170;
      const x = Math.max(4, Math.min(r.right - bw - 10, innerWidth - bw - 4));
      const y = Math.max(4, r.top + 10);
      this.wrap.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
      this.wrap.style.width = `${bw}px`;
    }
    return show;
  }

  contains(x: number, y: number): boolean {
    const r = this.video.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }

  destroy() {
    this.host.remove();
  }

  private closeMenu() {
    this.menu?.remove();
    this.menu = null;
    this.reveal(1500);
  }

  private toast(text: string) {
    const t = el("div", "toast", text);
    this.wrap.append(t);
    setTimeout(() => t.remove(), 2200);
  }

  async openMenu() {
    const menu = el("div", "menu");
    menu.setAttribute("role", "menu");
    this.menu = menu;
    this.wrap.append(menu);
    menu.append(el("h4", undefined, msg("downloadQualities")));
    const status = spinnerNote(msg("loadingFormats"));
    menu.append(status);
    requestFrame();

    const src = this.video.currentSrc && /^https?:/i.test(this.video.currentSrc) ? this.video.currentSrc : null;
    let cands: Candidate[] = [];
    try {
      const r = await chrome.runtime.sendMessage({ type: "velox:media-candidates", src });
      cands = (r?.candidates as Candidate[]) ?? [];
    } catch {
      cands = [];
    }
    let shown = 0;
    let lastError: string | null = null;
    for (const c of cands) {
      if (this.menu !== menu) return; // closed meanwhile
      const res = await chrome.runtime
        .sendMessage({ type: "velox:media-probe", url: c.url, kind: c.kind, pageUrl: location.href, title: document.title })
        .catch((e: Error) => ({ error: e.message }));
      if (this.menu !== menu) return;
      if (!res || res.error) {
        lastError = res?.error ?? "error";
        continue;
      }
      const result = res.result as MediaProbeResult;
      if (result.drm_protected) {
        lastError = msg("drmProtected");
        continue;
      }
      if (result.is_live) {
        lastError = msg("liveStream");
        continue;
      }
      const options = buildOptions(result, msg("audioOnly"));
      if (!options.length) continue;
      if (shown > 0) menu.insertBefore(el("div", "sep"), status);
      if (c.kind === "page" && shown > 0) menu.insertBefore(el("h4", undefined, msg("pageExtractor")), status);
      for (const o of options) menu.insertBefore(this.optionButton(c, result, o), status);
      shown += options.length;
      // Manifests and direct files are enough; only fall back to the
      // (slower) site extractor when nothing else was found.
      if (c.kind !== "page" && shown > 0 && cands.some((x) => x.kind === "page")) {
        const more = el("button", "link", msg("pageExtractor"));
        const page = cands.find((x) => x.kind === "page")!;
        more.addEventListener("click", (e) => {
          e.stopPropagation();
          more.remove();
          void this.probeInto(menu, status, page);
        });
        status.replaceChildren(more);
        status.className = "foot";
        this.addFooter(menu);
        return;
      }
    }
    if (shown === 0) {
      status.className = lastError ? "note err" : "note";
      status.textContent = lastError ?? msg("noFormats");
    } else {
      status.remove();
    }
    this.addFooter(menu);
  }

  private async probeInto(menu: HTMLDivElement, status: HTMLDivElement, c: Candidate) {
    const row = spinnerNote(msg("loadingFormats"));
    menu.insertBefore(row, status);
    const res = await chrome.runtime
      .sendMessage({ type: "velox:media-probe", url: c.url, kind: c.kind, pageUrl: location.href, title: document.title })
      .catch((e: Error) => ({ error: e.message }));
    if (!res || res.error) {
      row.className = "note err";
      row.textContent = res?.error ?? "error";
      return;
    }
    const result = res.result as MediaProbeResult;
    const options = buildOptions(result, msg("audioOnly"));
    row.replaceWith(el("div", "sep"));
    for (const o of options) menu.insertBefore(this.optionButton(c, result, o), status);
    if (!options.length) menu.insertBefore(el("div", "note", msg("noFormats")), status);
  }

  private addFooter(menu: HTMLDivElement) {
    const foot = el("div", "foot");
    const hide = el("button", "link", msg("floatingButtonHidden"));
    hide.addEventListener("click", (e) => {
      e.stopPropagation();
      hiddenOnPage = true;
      this.closeMenu();
      requestFrame();
    });
    const close = el("button", "link", msg("close"));
    close.addEventListener("click", (e) => {
      e.stopPropagation();
      this.closeMenu();
    });
    foot.append(hide, close);
    menu.append(foot);
  }

  private optionButton(c: Candidate, result: MediaProbeResult, o: MediaOption): HTMLButtonElement {
    const b = el("button", "item");
    b.type = "button";
    b.setAttribute("role", "menuitem");
    b.append(el("b", undefined, o.label), el("span", undefined, o.detail));
    b.addEventListener("click", async (e) => {
      e.stopPropagation();
      const selection: MediaRequest = {
        kind: c.kind,
        url: result.url || c.url,
        format_id: o.formatId,
        audio_format_id: o.audioFormatId,
        subtitle_languages: [],
        embed_subtitles: false,
        container: o.container,
        title: result.title ?? document.title ?? null,
        max_height: o.height,
      };
      const r = await chrome.runtime
        .sendMessage({ type: "velox:media-download", url: c.url, kind: c.kind, pageUrl: location.href, title: selection.title, selection })
        .catch((err: Error) => ({ error: err.message }));
      this.closeMenu();
      this.toast(r?.ok ? msg("sentToVelox") : msg("failed", r?.error ?? "error"));
    });
    return b;
  }
}

// ----- scanning and positioning -----------------------------------------------

let frameRequested = false;
function requestFrame() {
  if (frameRequested) return;
  frameRequested = true;
  requestAnimationFrame((now) => {
    frameRequested = false;
    let again = false;
    for (const o of overlays.values()) again = o.update(now) || again;
    if (again) setTimeout(requestFrame, 120);
  });
}

function scan() {
  for (const [v, o] of overlays) {
    if (!v.isConnected) {
      o.destroy();
      overlays.delete(v);
    }
  }
  if (!config?.video_detection) return;
  for (const v of Array.from(document.querySelectorAll("video"))) {
    if (!overlays.has(v)) overlays.set(v, new Overlay(v));
  }
  requestFrame();
}

let scanTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleScan() {
  if (scanTimer) return;
  scanTimer = setTimeout(() => {
    scanTimer = null;
    scan();
  }, 400);
}

new MutationObserver((records) => {
  for (const r of records) {
    for (const n of Array.from(r.addedNodes)) {
      if (n instanceof HTMLVideoElement || (n instanceof Element && n.querySelector?.("video"))) {
        scheduleScan();
        return;
      }
    }
    if (r.removedNodes.length) scheduleScan();
  }
}).observe(document.documentElement, { childList: true, subtree: true });

let lastMove = 0;
document.addEventListener(
  "mousemove",
  (e) => {
    const now = performance.now();
    if (now - lastMove < 80) return;
    lastMove = now;
    for (const o of overlays.values()) if (o.contains(e.clientX, e.clientY)) o.reveal(2500);
  },
  { passive: true, capture: true },
);
addEventListener("scroll", requestFrame, { passive: true, capture: true });
addEventListener("resize", requestFrame, { passive: true });
document.addEventListener("fullscreenchange", () => {
  for (const o of overlays.values()) o.mount();
  requestFrame();
});

chrome.runtime.onMessage.addListener((m: { type?: string }) => {
  if (m?.type === "velox:media-updated") scheduleScan();
  if (m?.type === "velox:open-media-menu") {
    // Context menu on a video with a blob: source: open the menu of the largest video.
    const best = [...overlays.values()].sort((a, b) => {
      const ra = a.video.getBoundingClientRect();
      const rb = b.video.getBoundingClientRect();
      return rb.width * rb.height - ra.width * ra.height;
    })[0];
    if (best) {
      best.reveal(5000);
      void best.openMenu();
    }
  }
});

void loadConfig().then(scan);
