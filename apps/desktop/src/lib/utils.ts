import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Extract downloadable URLs from free text (one per line, or embedded). */
export function extractUrls(text: string): string[] {
  const re = /\b(?:https?|ftps?|ftpes|sftp):\/\/[^\s"'<>]+/gi;
  const out: string[] = [];
  const seen = new Set<string>();
  for (const m of text.matchAll(re)) {
    const url = m[0].replace(/[),.;]+$/, "");
    if (!seen.has(url)) {
      seen.add(url);
      out.push(url);
    }
  }
  return out;
}

/**
 * Expand numeric range patterns: `file[01-12].zip` -> file01.zip … file12.zip.
 * Zero padding follows the width of the start number. At most `limit` URLs.
 */
export function expandPattern(url: string, limit = 2000): string[] {
  const m = url.match(/\[(\d+)-(\d+)\]/);
  if (!m || m.index === undefined) return [url];
  const [whole, a, b] = m;
  const start = parseInt(a, 10);
  const end = parseInt(b, 10);
  if (end < start) return [url];
  const width = a.length > 1 && a.startsWith("0") ? a.length : 0;
  const out: string[] = [];
  for (let i = start; i <= end && out.length < limit; i++) {
    const n = width ? String(i).padStart(width, "0") : String(i);
    const replaced = url.slice(0, m.index) + n + url.slice(m.index + whole.length);
    out.push(...expandPattern(replaced, limit - out.length));
  }
  return out;
}

export function isProbablyUrl(s: string): boolean {
  return /^(?:https?|ftps?|ftpes|sftp):\/\/\S+$/i.test(s.trim());
}
