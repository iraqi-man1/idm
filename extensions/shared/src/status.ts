// Render the bridge state for the popup and options page.
import type { BridgeState } from "./native";
import { t } from "./i18n";

export function renderStatus(state: BridgeState | undefined, statusEl: HTMLElement, fixEl: HTMLElement) {
  fixEl.hidden = true;
  statusEl.className = "status";
  switch (state?.kind) {
    case "connected":
      statusEl.textContent = t("statusConnected", state.appVersion);
      statusEl.classList.add("ok");
      break;
    case "app_not_running":
      statusEl.textContent = t("statusAppNotRunning");
      statusEl.classList.add("warn");
      break;
    case "host_missing":
      statusEl.textContent = t("statusHostMissing");
      statusEl.classList.add("err");
      fixEl.textContent = t("fixHostMissing");
      fixEl.hidden = false;
      break;
    case "incompatible":
      statusEl.textContent = t("statusIncompatible");
      statusEl.classList.add("err");
      break;
    case "error":
      statusEl.textContent = t("statusError", state.message);
      statusEl.classList.add("err");
      break;
    default:
      statusEl.textContent = t("statusConnecting");
  }
}
