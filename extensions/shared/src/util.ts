// Pure helpers shared by the background script, popup and tests.

export function hostOf(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).hostname.toLowerCase() || null;
  } catch {
    return null;
  }
}

/** `example.com` matches `example.com` and any subdomain. */
export function hostMatches(host: string, pattern: string): boolean {
  const p = pattern.trim().toLowerCase().replace(/^\*\./, "").replace(/^\./, "");
  return p.length > 0 && (host === p || host.endsWith(`.${p}`));
}

export function isExcluded(urls: Array<string | null | undefined>, patterns: string[]): boolean {
  return urls.some((u) => {
    const h = hostOf(u);
    return h !== null && patterns.some((p) => hostMatches(h, p));
  });
}

/** Lower-case extension of a file name or URL path, without the dot. */
export function extOf(nameOrUrl: string | null | undefined): string | null {
  if (!nameOrUrl) return null;
  let path = nameOrUrl;
  try {
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(nameOrUrl)) path = decodeURIComponent(new URL(nameOrUrl).pathname);
  } catch {
    /* keep raw */
  }
  const base = path.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  if (dot <= 0 || dot === base.length - 1) return null;
  const ext = base.slice(dot + 1).toLowerCase();
  return /^[a-z0-9]{1,10}$/.test(ext) ? ext : null;
}

const MIME_EXT: Record<string, string> = {
  "application/zip": "zip",
  "application/x-zip-compressed": "zip",
  "application/x-7z-compressed": "7z",
  "application/vnd.rar": "rar",
  "application/x-rar-compressed": "rar",
  "application/x-msdownload": "exe",
  "application/x-msi": "msi",
  "application/x-apple-diskimage": "dmg",
  "application/pdf": "pdf",
  "application/gzip": "gz",
  "application/x-iso9660-image": "iso",
  "video/mp4": "mp4",
  "video/webm": "webm",
  "video/x-matroska": "mkv",
  "audio/mpeg": "mp3",
  "audio/mp4": "m4a",
  "audio/flac": "flac",
};

export function extFromMime(mime: string | null | undefined): string | null {
  if (!mime) return null;
  return MIME_EXT[mime.split(";")[0].trim().toLowerCase()] ?? null;
}

export function basename(path: string | null | undefined): string | null {
  if (!path) return null;
  const b = path.split(/[\\/]/).pop();
  return b && b.length > 0 ? b : null;
}

export interface CaptureConfig {
  enabled: boolean;
  extensions: string[];
  minSize: number;
  excluded: string[];
}

export interface CaptureInput {
  url: string;
  finalUrl?: string | null;
  filename?: string | null;
  mime?: string | null;
  fileSize?: number | null;
  referrer?: string | null;
  tabUrl?: string | null;
  byExtensionId?: string | null;
  incognito?: boolean;
  bypass?: boolean;
  recentlyHandled?: boolean;
}

export type CaptureDecision =
  | { capture: true }
  | {
      capture: false;
      reason: "disabled" | "scheme" | "extension" | "bypass" | "incognito" | "excluded" | "type" | "small" | "duplicate";
    };

/** Decide whether a browser download should be handed to Velox. */
export function shouldCapture(d: CaptureInput, cfg: CaptureConfig): CaptureDecision {
  if (!cfg.enabled) return { capture: false, reason: "disabled" };
  const url = d.finalUrl || d.url;
  if (!/^(https?|ftps?):\/\//i.test(url)) return { capture: false, reason: "scheme" };
  if (d.byExtensionId) return { capture: false, reason: "extension" };
  if (d.bypass) return { capture: false, reason: "bypass" };
  if (d.incognito) return { capture: false, reason: "incognito" };
  if (isExcluded([d.url, d.finalUrl, d.referrer, d.tabUrl], cfg.excluded)) return { capture: false, reason: "excluded" };
  const ext = extOf(d.filename) ?? extOf(d.finalUrl) ?? extOf(d.url) ?? extFromMime(d.mime);
  const wanted = new Set(cfg.extensions.map((e) => e.toLowerCase().replace(/^\./, "")));
  if (!ext || !wanted.has(ext)) return { capture: false, reason: "type" };
  if (d.fileSize && d.fileSize > 0 && cfg.minSize > 0 && d.fileSize < cfg.minSize) return { capture: false, reason: "small" };
  if (d.recentlyHandled) return { capture: false, reason: "duplicate" };
  return { capture: true };
}

export function cookieHeader(cookies: Array<{ name: string; value: string }>): string | null {
  const parts = cookies.filter((c) => c.name).map((c) => `${c.name}=${c.value}`);
  return parts.length ? parts.join("; ") : null;
}

/** Remembers recently handled URLs to avoid duplicate hand-overs. */
export class RecentSet {
  private readonly items = new Map<string, number>();
  constructor(private readonly ttlMs: number) {}
  add(key: string, now = Date.now()) {
    this.items.set(key, now);
  }
  has(key: string, now = Date.now()): boolean {
    for (const [k, t] of this.items) if (now - t > this.ttlMs) this.items.delete(k);
    return this.items.has(key);
  }
}
