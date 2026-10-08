import type { BridgeState } from "./native";
import { localize } from "./i18n";
import type { MediaItem } from "./media";
import { renderStatus } from "./status";
import { hostOf } from "./util";

interface Status {
  state: BridgeState;
  prefs: { captureEnabled: boolean; excludedSites: string[] };
}

localize();
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function main() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const host = tab?.url && /^https?:/i.test(tab.url) ? hostOf(tab.url) : null;
  const render = (s: Status) => {
    renderStatus(s.state, $("status"), $("fix"));
    $<HTMLInputElement>("capture").checked = s.prefs.captureEnabled;
    const site = $<HTMLInputElement>("site");
    $("site-row").hidden = !host;
    site.checked = !!host && !s.prefs.excludedSites.includes(host);
    site.disabled = !s.prefs.captureEnabled;
  };
  render((await chrome.runtime.sendMessage({ type: "velox:status" })) as Status);
  void chrome.runtime.sendMessage({ type: "velox:refresh" }).then((s: Status) => s?.state && render(s));

  $<HTMLInputElement>("capture").addEventListener("change", async (e) => {
    await chrome.runtime.sendMessage({ type: "velox:set-capture", enabled: (e.target as HTMLInputElement).checked });
    render((await chrome.runtime.sendMessage({ type: "velox:status" })) as Status);
  });
  $<HTMLInputElement>("site").addEventListener("change", async (e) => {
    if (!host) return;
    await chrome.runtime.sendMessage({ type: "velox:set-site-excluded", host, excluded: !(e.target as HTMLInputElement).checked });
  });
  $("open").addEventListener("click", () => void chrome.runtime.sendMessage({ type: "velox:show-app" }).then(() => window.close()));
  $("options").addEventListener("click", () => void chrome.runtime.openOptionsPage());

  // Media detected on this tab.
  const list = $("media");
  if (tab?.id !== undefined) {
    const r = (await chrome.runtime.sendMessage({ type: "velox:media-list", tabId: tab.id })) as { items?: MediaItem[] };
    const items = (r.items ?? []).slice().sort((a, b) => b.seen - a.seen);
    if (!items.length) {
      const li = document.createElement("li");
      li.className = "empty";
      li.textContent = chrome.i18n.getMessage("mediaNone");
      list.append(li);
    }
    for (const item of items.slice(0, 20)) {
      const li = document.createElement("li");
      const kind = document.createElement("span");
      kind.className = "kind";
      kind.textContent = item.kind;
      const name = document.createElement("span");
      name.className = "name";
      name.title = item.url;
      name.textContent = decodeURIComponent(new URL(item.url).pathname.split("/").pop() || item.url);
      const btn = document.createElement("button");
      btn.className = "small";
      btn.textContent = "↓";
      btn.title = chrome.i18n.getMessage("menuDownloadMedia");
      btn.addEventListener("click", async () => {
        btn.disabled = true;
        const res = await chrome.runtime.sendMessage({
          type: "velox:media-download",
          tabId: tab.id,
          url: item.url,
          kind: item.kind,
          pageUrl: item.pageUrl,
          title: tab.title ?? null,
          selection: {
            kind: item.kind,
            url: item.url,
            format_id: null,
            audio_format_id: null,
            subtitle_languages: [],
            embed_subtitles: false,
            container: "mp4",
            title: tab.title ?? null,
            max_height: null,
          },
        });
        btn.textContent = res?.ok ? "✓" : "!";
        btn.title = res?.ok ? chrome.i18n.getMessage("sentToVelox") : String(res?.error ?? "");
      });
      li.append(kind, name, btn);
      list.append(li);
    }
  }
}

void main();
