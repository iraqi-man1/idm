import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import ar from "./ar.json";
import en from "./en.json";
import { setFormatLocale } from "@/lib/format";

export const SUPPORTED = ["en", "ar"] as const;
export type Lang = (typeof SUPPORTED)[number];

/** Resolve the "system" preference to a supported language. */
export function resolveLanguage(pref: string | undefined): Lang {
  if (pref === "en" || pref === "ar") return pref;
  const nav = (navigator.languages?.[0] ?? navigator.language ?? "en").toLowerCase();
  return nav.startsWith("ar") ? "ar" : "en";
}

export function applyLanguage(pref: string | undefined): Lang {
  const lang = resolveLanguage(pref);
  document.documentElement.lang = lang;
  document.documentElement.dir = lang === "ar" ? "rtl" : "ltr";
  setFormatLocale(lang);
  if (i18n.language !== lang) void i18n.changeLanguage(lang);
  return lang;
}

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, ar: { translation: ar } },
  lng: resolveLanguage(undefined),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18n;
