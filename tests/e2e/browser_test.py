#!/usr/bin/env python3
"""Browser integration end-to-end test.

Runs the *real* chain:  Chromium + Velox extension  ->  velox-nmh (native
messaging host)  ->  authenticated IPC  ->  Velox desktop app.

* Chromium (Playwright) loads the unpacked extension; a native messaging
  host manifest is placed in the profile's NativeMessagingHosts folder.
* The desktop app is driven through tauri-driver (Selenium).
* A link click in the browser must be taken over: the app shows its
  "Download file info" dialog, the test clicks "Start download", the file is
  downloaded by Velox, and the browser's own download is cancelled.
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.options import ArgOptions

DEV_ID = "encnclpojnlecaheiiibdkkgiapnhocl"


def wait_for(fn, timeout=30.0, interval=0.25, what="condition"):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            v = fn()
            if v:
                return v
        except Exception as e:  # noqa: BLE001 - polling
            last = e
        time.sleep(interval)
    raise TimeoutError(f"timed out waiting for {what} (last error: {last!r})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--host", required=True, help="path to velox-nmh")
    ap.add_argument("--extension", required=True, help="unpacked chromium extension dir")
    ap.add_argument("--chrome", required=True)
    ap.add_argument("--server", required=True)
    ap.add_argument("--downloads", required=True)
    ap.add_argument("--data-dir", required=True)
    ap.add_argument("--profile", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--driver", default="http://127.0.0.1:4444")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    steps = []

    def step(s):
        print(f"[browser-e2e] {s}", flush=True)
        steps.append(s)

    opts = ArgOptions()
    opts.set_capability("browserName", "wry")
    opts.set_capability("tauri:options", {"application": os.path.abspath(args.app)})
    app = webdriver.Remote(command_executor=args.driver, options=opts)
    app.set_window_size(1280, 800)
    pw = sync_playwright().start()
    ctx = None
    chrome = None
    try:
        step("desktop app started")
        wait_for(lambda: app.find_elements(By.XPATH, "//button[contains(.,'All downloads')]"), what="app ui")
        endpoint = os.path.join(args.data_dir, "nm-endpoint.json")
        wait_for(lambda: os.path.exists(endpoint), what="IPC endpoint file")
        step("IPC endpoint published")

        # Register the native messaging host for this Chromium profile.
        nm_dir = os.path.join(args.profile, "NativeMessagingHosts")
        os.makedirs(nm_dir, exist_ok=True)
        with open(os.path.join(nm_dir, "com.veloxdm.host.json"), "w") as f:
            json.dump(
                {
                    "name": "com.veloxdm.host",
                    "description": "Velox Download Manager browser integration",
                    "path": os.path.abspath(args.host),
                    "type": "stdio",
                    "allowed_origins": [f"chrome-extension://{DEV_ID}/"],
                },
                f,
            )

        # Launch Chromium as a normal process (Playwright's own launcher
        # intercepts downloads through CDP, which bypasses the downloads API
        # the extension relies on) and attach over CDP for page interaction.
        ext = os.path.abspath(args.extension)
        chrome_dl = os.path.join(args.profile, "browser-downloads")
        os.makedirs(os.path.join(args.profile, "Default"), exist_ok=True)
        with open(os.path.join(args.profile, "Default", "Preferences"), "w") as f:
            json.dump({"download": {"default_directory": chrome_dl, "prompt_for_download": False}}, f)
        chrome = subprocess.Popen(
            [
                args.chrome,
                f"--user-data-dir={args.profile}",
                "--remote-debugging-port=9333",
                f"--disable-extensions-except={ext}",
                f"--load-extension={ext}",
                "--no-first-run",
                "--no-default-browser-check",
                "--window-size=1100,760",
                "about:blank",
            ]
            # CI containers often run as root, where Chromium requires this.
            + (["--no-sandbox"] if hasattr(os, "geteuid") and os.geteuid() == 0 else []),
            stdout=subprocess.DEVNULL,
            stderr=open(os.path.join(args.out, "chromium.log"), "w"),
        )
        wait_for(lambda: urllib.request.urlopen("http://127.0.0.1:9333/json/version", timeout=1).status == 200, what="chromium devtools")
        browser = pw.chromium.connect_over_cdp("http://127.0.0.1:9333")
        ctx = browser.contexts[0]
        # Playwright routes downloads to its own artifacts folder, which skips
        # the downloads API; restore normal browser behaviour.
        browser.new_browser_cdp_session().send("Browser.setDownloadBehavior", {"behavior": "default"})
        # An extension page gives access to the same chrome.* APIs as the
        # background script (storage, downloads).
        ext_page = ctx.new_page()
        wait_for(lambda: ext_page.goto(f"chrome-extension://{DEV_ID}/options.html") and True, what="extension page")
        sw = ext_page
        step(f"extension loaded ({DEV_ID})")

        state = wait_for(
            lambda: (lambda s: s if s and s.get("kind") == "connected" else None)(
                sw.evaluate("chrome.storage.local.get('bridgeState').then(r => r.bridgeState)")
            ),
            timeout=30,
            what="extension connected to app",
        )
        step(f"extension connected to Velox {state.get('appVersion')}")

        popup = ctx.new_page()
        popup.set_viewport_size({"width": 360, "height": 520})
        popup.goto(f"chrome-extension://{DEV_ID}/popup.html")
        popup.wait_for_function("document.getElementById('status').textContent.includes('Connected')", timeout=10000)
        popup.screenshot(path=os.path.join(args.out, "b1-popup.png"))
        popup.close()
        step("popup shows connected status")

        size = 8_000_000
        file_url = f"{args.server}/file/captured.zip?size={size}&rate=2000000"
        page = ctx.new_page()
        page.goto(f"{args.server}/page?href={file_url.replace('&', '%26').replace('?', '%3F')}")
        main_handle = app.current_window_handle
        page.click("#link")
        step("link clicked in the browser")

        wait_for(lambda: len(app.window_handles) > 1, timeout=30, what="capture dialog window")
        app.switch_to.window([h for h in app.window_handles if h != main_handle][0])
        name = wait_for(lambda: app.find_element(By.ID, "cap-name").get_attribute("value"), what="captured file name")
        assert name == "captured.zip", name
        wait_for(lambda: "7.6 MB" in app.execute_script("return document.body.textContent"), what="probed size")
        app.save_screenshot(os.path.join(args.out, "b2-capture-dialog.png"))
        step("capture dialog shows file info")
        app.find_element(By.XPATH, "//button[normalize-space(.)='Start download']").click()
        app.switch_to.window(main_handle)

        def completed():
            rows = app.find_elements(By.XPATH, "//div[@data-index][.//span[normalize-space(.)='captured.zip']]")
            return rows and "Completed" in app.execute_script("return arguments[0].textContent", rows[0])

        wait_for(completed, timeout=60, what="captured download completed in Velox")
        app.save_screenshot(os.path.join(args.out, "b3-completed-in-app.png"))
        path = os.path.join(args.downloads, "captured.zip")
        assert os.path.getsize(path) == size, os.path.getsize(path)
        step("Velox downloaded the captured file")

        items = sw.evaluate("chrome.downloads.search({}).then(r => r.map(i => ({url: i.url, state: i.state})))")
        assert not any(i["url"].startswith(file_url.split("?")[0]) and i["state"] == "complete" for i in items), items
        assert not os.path.exists(os.path.join(chrome_dl, "captured.zip")), "browser also saved the file"
        step("browser download was cancelled (no duplicate)")

        # Alt+click bypass: the browser keeps the download, Velox does not see it.
        bypass_url = f"{args.server}/file/bypass.zip?size=300000"
        page.goto(f"{args.server}/page?href={bypass_url.replace('&', '%26').replace('?', '%3F')}")
        page.wait_for_timeout(500)
        # Chromium on Linux does not turn a real Alt+click into a download
        # (Windows/ChromeOS do), so dispatch an Alt-modified click: the content
        # script registers the bypass and the link activation starts the download.
        page.evaluate(
            "document.getElementById('link').dispatchEvent(new MouseEvent('click', {bubbles: true, cancelable: true, altKey: true}))"
        )
        wait_for(lambda: os.path.exists(os.path.join(chrome_dl, "bypass.zip")), timeout=30, what="browser saved bypassed download")
        rows = app.find_elements(By.XPATH, "//div[@data-index][.//span[normalize-space(.)='bypass.zip']]")
        assert not rows, "bypassed download must not reach Velox"
        step("Alt+click bypass leaves the download to the browser")

        print(f"[browser-e2e] PASS: {', '.join(steps)}", flush=True)
        return 0
    except Exception as e:  # noqa: BLE001
        try:
            app.save_screenshot(os.path.join(args.out, "b-failure-app.png"))
        except Exception:  # noqa: BLE001
            pass
        print(f"[browser-e2e] FAIL after {steps}: {e!r}", flush=True)
        return 1
    finally:
        if chrome:
            chrome.terminate()
        pw.stop()
        app.quit()


if __name__ == "__main__":
    sys.exit(main())
