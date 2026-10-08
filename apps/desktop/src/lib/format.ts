// Locale-aware formatting of sizes, speeds, durations and dates.

let currentLocale = "en";

export function setFormatLocale(lang: string) {
  // Arabic UI with Western digits (common in technical software).
  currentLocale = lang === "ar" ? "ar-u-nu-latn" : "en";
}

const UNITS: Record<string, string[]> = {
  en: ["B", "KB", "MB", "GB", "TB"],
  ar: ["بايت", "ك.ب", "م.ب", "ج.ب", "ت.ب"],
};

function units(): string[] {
  return currentLocale.startsWith("ar") ? UNITS.ar : UNITS.en;
}

export function formatBytes(bytes: number | null | undefined, digits = 1): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return "—";
  const u = units();
  if (bytes < 1000) return `${new Intl.NumberFormat(currentLocale).format(bytes)} ${u[0]}`;
  let v = bytes;
  let i = 0;
  // Switch units at 1000 so values never show as "1,021 KB".
  while (v >= 1000 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  const nf = new Intl.NumberFormat(currentLocale, {
    minimumFractionDigits: v < 10 ? digits : 0,
    maximumFractionDigits: v < 10 ? digits : v < 100 ? 1 : 0,
  });
  return `${nf.format(v)} ${u[i]}`;
}

export function formatSpeed(bytesPerSec: number | null | undefined): string {
  if (!bytesPerSec) return "—";
  const per = currentLocale.startsWith("ar") ? "/ث" : "/s";
  return `${formatBytes(bytesPerSec)}${per}`;
}

export function formatDuration(totalSeconds: number | null | undefined): string {
  if (totalSeconds === null || totalSeconds === undefined || !Number.isFinite(totalSeconds)) return "—";
  const s = Math.max(0, Math.round(totalSeconds));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const ar = currentLocale.startsWith("ar");
  const L = ar ? { d: "ي", h: "س", m: "د", s: "ث" } : { d: "d", h: "h", m: "m", s: "s" };
  if (d > 0) return `${d}${L.d} ${h}${L.h}`;
  if (h > 0) return `${h}${L.h} ${m}${L.m}`;
  if (m > 0) return `${m}${L.m} ${sec}${L.s}`;
  return `${sec}${L.s}`;
}

export function formatDate(ms: number | null | undefined): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  return new Intl.DateTimeFormat(currentLocale, sameDay
    ? { hour: "2-digit", minute: "2-digit" }
    : { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" },
  ).format(d);
}

export function formatPercent(fraction: number): string {
  return new Intl.NumberFormat(currentLocale, { style: "percent", maximumFractionDigits: 1 }).format(fraction);
}

export function formatNumber(n: number): string {
  return new Intl.NumberFormat(currentLocale).format(n);
}

/** Parse "1.5 MB", "512k", "2m" into bytes per second. Returns null when invalid. */
export function parseSize(input: string): number | null {
  const m = input.trim().toLowerCase().match(/^(\d+(?:\.\d+)?)\s*([kmgt]?)(?:i?b)?(?:\/s)?$/);
  if (!m) return null;
  const mult: Record<string, number> = { "": 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3, t: 1024 ** 4 };
  return Math.round(parseFloat(m[1]) * mult[m[2]]);
}
