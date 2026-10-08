import type { BridgeState } from "./native";
import { localize, t } from "./i18n";
import { renderStatus } from "./status";

interface Status {
  state: BridgeState;
  prefs: { captureEnabled: boolean; excludedSites: string[] };
}

localize();
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function refresh() {
  const s = (await chrome.runtime.sendMessage({ type: "velox:refresh" })) as Status;
  renderStatus(s.state, $("status"), $("fix"));
  $<HTMLInputElement>("capture").checked = s.prefs.captureEnabled;
  const list = $("exclusions");
  list.replaceChildren();
  if (!s.prefs.excludedSites.length) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = t("noExclusions");
    list.append(li);
  }
  for (const host of s.prefs.excludedSites) {
    const li = document.createElement("li");
    const name = document.createElement("span");
    name.style.flex = "1";
    name.textContent = host;
    const rm = document.createElement("button");
    rm.className = "small";
    rm.textContent = t("remove");
    rm.addEventListener("click", async () => {
      await chrome.runtime.sendMessage({ type: "velox:set-site-excluded", host, excluded: false });
      await refresh();
    });
    li.append(name, rm);
    list.append(li);
  }
  // Firefox grants host permissions only on request.
  const granted = await chrome.permissions.contains({ origins: ["<all_urls>"] });
  $("perm").hidden = granted;
}

$<HTMLInputElement>("capture").addEventListener("change", async (e) => {
  await chrome.runtime.sendMessage({ type: "velox:set-capture", enabled: (e.target as HTMLInputElement).checked });
  await refresh();
});
$("grant").addEventListener("click", async () => {
  await chrome.permissions.request({ origins: ["<all_urls>"] });
  await refresh();
});

void refresh();
