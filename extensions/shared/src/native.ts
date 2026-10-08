// Connection to the Velox desktop app through the native messaging host.

import type { ExtBrowserConfig } from "@bindings/ExtBrowserConfig";
import type { ExtMessage } from "@bindings/ExtMessage";
import type { ExtReply } from "@bindings/ExtReply";
import type { ExtResponse } from "@bindings/ExtResponse";

export const HOST_NAME = "com.veloxdm.host";
export const PROTOCOL_VERSION = 1;

export type BridgeState =
  | { kind: "connecting" }
  | { kind: "connected"; appVersion: string }
  | { kind: "app_not_running" }
  | { kind: "host_missing"; detail: string }
  | { kind: "incompatible"; message: string }
  | { kind: "error"; message: string };

export class BridgeError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

interface Pending {
  resolve: (r: ExtReply) => void;
  reject: (e: BridgeError) => void;
  timer: ReturnType<typeof setTimeout>;
}

type Listener = (state: BridgeState, config: ExtBrowserConfig | null) => void;

export class NativeBridge {
  private port: chrome.runtime.Port | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private retryDelay = 1000;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private listeners = new Set<Listener>();
  state: BridgeState = { kind: "connecting" };
  config: ExtBrowserConfig | null = null;

  constructor(private readonly browserName: string, private readonly extensionVersion: string) {}

  onChange(l: Listener): () => void {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }

  private setState(s: BridgeState) {
    this.state = s;
    for (const l of this.listeners) l(s, this.config);
  }

  private ensurePort(): chrome.runtime.Port {
    if (this.port) return this.port;
    const port = chrome.runtime.connectNative(HOST_NAME);
    this.port = port;
    port.onMessage.addListener((msg: ExtResponse) => this.onMessage(msg));
    port.onDisconnect.addListener(() => {
      const err = chrome.runtime.lastError?.message ?? "disconnected";
      this.port = null;
      for (const [, p] of this.pending) {
        clearTimeout(p.timer);
        p.reject(new BridgeError("disconnected", err));
      }
      this.pending.clear();
      if (/not found|no such native application|access to the specified native messaging host is forbidden/i.test(err)) {
        this.setState({ kind: "host_missing", detail: err });
      } else {
        this.setState({ kind: "error", message: err });
      }
      this.scheduleReconnect();
    });
    return port;
  }

  private onMessage(msg: ExtResponse) {
    if (msg.id === 0) {
      // Unsolicited notification from the app.
      if (msg.type === "config") {
        this.config = msg.config;
        this.setState(this.state);
      }
      return;
    }
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    clearTimeout(p.timer);
    if (msg.type === "error") p.reject(new BridgeError(msg.code, msg.message));
    else p.resolve(msg);
  }

  private scheduleReconnect() {
    if (this.retryTimer) return;
    const delay = this.retryDelay;
    this.retryDelay = Math.min(this.retryDelay * 2, 60_000);
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      void this.hello();
    }, delay);
  }

  /** Send a request; resolves with the reply or rejects with BridgeError. */
  request(message: ExtMessage, timeoutMs = 15_000): Promise<ExtReply> {
    let port: chrome.runtime.Port;
    try {
      port = this.ensurePort();
    } catch (e) {
      return Promise.reject(new BridgeError("host_missing", String(e)));
    }
    const id = this.nextId++;
    return new Promise<ExtReply>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new BridgeError("timeout", "Velox did not answer in time"));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        port.postMessage({ id, ...message });
      } catch (e) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(new BridgeError("disconnected", String(e)));
      }
    });
  }

  /** Handshake; refreshes state and capture configuration. */
  async hello(): Promise<BridgeState> {
    if (this.state.kind !== "connected") this.setState({ kind: "connecting" });
    try {
      const r = await this.request(
        { type: "hello", extension_version: this.extensionVersion, browser: this.browserName, protocol_version: PROTOCOL_VERSION },
        10_000,
      );
      if (r.type !== "hello") throw new BridgeError("protocol", `unexpected reply ${r.type}`);
      this.retryDelay = 1000;
      this.config = r.config;
      this.setState(r.compatible ? { kind: "connected", appVersion: r.app_version } : { kind: "incompatible", message: r.message ?? "" });
    } catch (e) {
      const err = e as BridgeError;
      if (err.code === "app_not_running") this.setState({ kind: "app_not_running" });
      else if (this.state.kind !== "host_missing") this.setState({ kind: "error", message: err.message });
    }
    return this.state;
  }
}
