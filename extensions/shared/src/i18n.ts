// Localize static pages: elements with data-i18n get their message text.
export function localize(root: ParentNode = document) {
  for (const el of Array.from(root.querySelectorAll<HTMLElement>("[data-i18n]"))) {
    const text = chrome.i18n.getMessage(el.dataset.i18n!);
    if (text) el.textContent = text;
  }
  document.documentElement.dir = chrome.i18n.getMessage("@@bidi_dir") || "ltr";
  document.documentElement.lang = chrome.i18n.getUILanguage();
}

export const t = (key: string, ...subs: string[]) => chrome.i18n.getMessage(key, subs) || key;
