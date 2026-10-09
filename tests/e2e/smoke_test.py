#!/usr/bin/env python3
"""End-to-end smoke test of the real desktop application.

Drives the built Tauri binary through tauri-driver (WebDriver) and performs a
real multi-connection download from the local test server: add URL, watch
live progress, pause, resume, complete, then verify the file on disk. Also
switches the UI to Arabic (RTL) and dark mode and saves screenshots.

Requirements (Linux): WebKitWebDriver (webkit2gtk-driver), tauri-driver
(`cargo install tauri-driver`), Xvfb, Python 3 with selenium.

Usage:
    cargo build -p velox-desktop --features custom-protocol
    python3 tests/e2e/smoke_test.py --app target/debug/velox-desktop \
        --server http://127.0.0.1:8787 --out /tmp/velox-e2e
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

from selenium import webdriver
from selenium.common.exceptions import StaleElementReferenceException
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.options import ArgOptions


def wait_for(fn, timeout=30.0, interval=0.2, what="condition"):
    deadline = time.time() + timeout
    last_exc = None
    while time.time() < deadline:
        try:
            v = fn()
            if v:
                return v
        except StaleElementReferenceException as e:  # re-rendered rows
            last_exc = e
        time.sleep(interval)
    raise TimeoutError(f"timed out waiting for {what} ({last_exc})")


def by_text(driver, tag, text):
    els = driver.find_elements(By.XPATH, f"//{tag}[contains(normalize-space(.), '{text}')]")
    return els[0] if els else None


def set_value(driver, el, value):
    """Set an input's value the way typing does (React sees an input event).

    Used where WebKitWebDriver's synthetic keyboard is unreliable (after
    earlier key actions it can drop the Shift state, typing ';' for ':')."""
    driver.execute_script(
        "const [el, v] = arguments;"
        "const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;"
        "Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);"
        "el.dispatchEvent(new Event('input', {bubbles: true}));",
        el,
        value,
    )


def field_switch(driver, label):
    """The switch on the settings/queue row labelled `label`."""
    return driver.find_element(
        By.XPATH, f"//label[normalize-space(.)='{label}']/ancestor::div[contains(@class,'py-3')][1]//button[@role='switch']"
    )


def pick(driver, combobox, option_text):
    """Choose an option of a Radix select (WebKitWebDriver needs move + Enter)."""
    combobox.click()
    opt = wait_for(lambda: driver.find_elements(By.XPATH, f"//div[@role='option'][contains(.,'{option_text}')]"), what=option_text)[0]
    time.sleep(0.3)
    ActionChains(driver).move_to_element(opt).perform()
    ActionChains(driver).send_keys(Keys.ENTER).perform()
    wait_for(lambda: option_text in combobox.text, what=f"{option_text} selected")


def row_status(driver, name):
    rows = driver.find_elements(By.XPATH, f"//div[@data-index][.//span[normalize-space(.)='{name}']]")
    # textContent: WebKitWebDriver's visible-text algorithm skips truncated cells.
    return driver.execute_script("return arguments[0].textContent", rows[0]) if rows else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--server", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--downloads", required=True)
    ap.add_argument("--driver", default="http://127.0.0.1:4444")
    ap.add_argument("--media", action="store_true", help="server has the media fixtures at /static")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)

    opts = ArgOptions()
    opts.set_capability("browserName", "wry")
    opts.set_capability("tauri:options", {"application": os.path.abspath(args.app)})
    driver = webdriver.Remote(command_executor=args.driver, options=opts)
    driver.set_window_size(1280, 800)
    results = []

    def step(name):
        print(f"[e2e] {name}", flush=True)
        results.append(name)

    try:
        step("app launched")
        wait_for(lambda: by_text(driver, "button", "All downloads"), what="sidebar")
        driver.save_screenshot(os.path.join(args.out, "01-empty.png"))

        size = 40 * 1024 * 1024
        url = f"{args.server}/file/e2e-video.mp4?size={size}&rate=400000&mime=video%2Fmp4"
        step("open add dialog")
        by_text(driver, "button", "Add URL").click()
        area = wait_for(lambda: driver.find_element(By.ID, "add-url"), what="url field")
        area.clear()
        area.send_keys(url)
        step("probe shows size")
        wait_for(lambda: by_text(driver, "b", "40 MB"), what="probe result")
        driver.save_screenshot(os.path.join(args.out, "02-add-dialog.png"))
        by_text(driver, "button", "Start download").click()

        step("enable the Connections column from the header menu")
        header = wait_for(lambda: by_text(driver, "button", "File name"), what="header")

        def toggle_connections_column():
            ActionChains(driver).context_click(header).perform()
            item = wait_for(
                lambda: [e for e in driver.find_elements(By.XPATH, "//div[@role='menuitemcheckbox'][contains(.,'Connections')]") if e.is_displayed()],
                what="column menu",
            )[0]
            time.sleep(0.3)  # menu open animation
            try:
                ActionChains(driver).move_to_element(item).click().perform()
            except Exception:  # noqa: BLE001 - menu still animating; close and retry
                ActionChains(driver).send_keys(Keys.ESCAPE).perform()
                return False
            time.sleep(0.2)
            ActionChains(driver).send_keys(Keys.ESCAPE).perform()
            return by_text(driver, "button", "Connections")

        wait_for(toggle_connections_column, timeout=20, interval=0.5, what="connections column")

        step("download running with multiple connections")
        def multi_conn():
            txt = row_status(driver, "e2e-video.mp4") or ""
            m = re.search(r"(\d+)/8", txt)
            return m is not None and int(m.group(1)) >= 2
        wait_for(multi_conn, timeout=30, what="2+ active connections")
        time.sleep(1.5)
        driver.save_screenshot(os.path.join(args.out, "03-downloading.png"))

        step("progress window shows live connections")
        main_handle = driver.current_window_handle
        row = driver.find_element(By.XPATH, "//div[@data-index][.//span[normalize-space(.)='e2e-video.mp4']]")
        ActionChains(driver).double_click(row).perform()
        wait_for(lambda: len(driver.window_handles) > 1, what="progress window")
        driver.switch_to.window([h for h in driver.window_handles if h != main_handle][0])
        wait_for(lambda: by_text(driver, "span", "Transfer rate"), what="progress details")
        time.sleep(2.5)
        driver.save_screenshot(os.path.join(args.out, "03b-progress-window.png"))
        by_text(driver, "button", "Connections").click()
        wait_for(lambda: len(driver.find_elements(By.XPATH, "//tbody/tr")) >= 2, what="connection rows")
        driver.save_screenshot(os.path.join(args.out, "03c-progress-connections.png"))
        by_text(driver, "button", "Hide").click()
        driver.switch_to.window(main_handle)

        step("pause via context menu")
        row = driver.find_element(By.XPATH, "//div[@data-index][.//span[normalize-space(.)='e2e-video.mp4']]")
        ActionChains(driver).context_click(row).perform()
        wait_for(lambda: by_text(driver, "div", "Pause") and driver.find_elements(By.XPATH, "//div[@role='menuitem'][contains(.,'Pause')]"), what="menu")
        driver.find_element(By.XPATH, "//div[@role='menuitem'][contains(.,'Pause')]").click()
        def paused():
            txt = row_status(driver, "e2e-video.mp4") or ""
            return "Paused" in txt
        wait_for(paused, what="paused")
        driver.save_screenshot(os.path.join(args.out, "04-paused.png"))
        partial = [f for f in os.listdir(args.downloads) if f.endswith(".vdpart")]
        assert partial, "partial file exists while paused"

        step("resume from toolbar")
        row = driver.find_element(By.XPATH, "//div[@data-index][.//span[normalize-space(.)='e2e-video.mp4']]")
        row.click()
        driver.find_element(By.XPATH, "//button[.//span[normalize-space(.)='Resume'] or normalize-space(.)='Resume']").click()
        step("download completes")
        wait_for(lambda: "Completed" in (row_status(driver, "e2e-video.mp4") or ""), timeout=90, what="completed")
        driver.save_screenshot(os.path.join(args.out, "05-completed.png"))
        path = os.path.join(args.downloads, "e2e-video.mp4")
        assert os.path.getsize(path) == size, f"size {os.path.getsize(path)} != {size}"
        assert not any(f.endswith(".vdpart") for f in os.listdir(args.downloads)), "partial removed"

        if args.media:
            step("HLS master playlist: quality picker in the add dialog")
            by_text(driver, "button", "Add URL").click()
            area = wait_for(lambda: driver.find_element(By.ID, "add-url"), what="url field")
            set_value(driver, area, f"{args.server}/static/hls/master.m3u8")
            wait_for(lambda: driver.find_elements(By.CSS_SELECTOR, "[data-testid='media-picker']"), timeout=30, what="media picker")
            radio = wait_for(
                lambda: driver.find_elements(By.XPATH, "//button[@role='radio'][.//b[normalize-space(.)='180p']]"), what="180p option"
            )[0]
            radio.click()
            wait_for(lambda: radio.get_attribute("aria-checked") == "true", what="180p selected")
            step("pick the Arabic audio track")
            track = wait_for(lambda: driver.find_elements(By.XPATH, "//button[@role='combobox'][@aria-label='Audio track']"), what="audio track select")[0]
            assert "English" in track.text, track.text
            track.click()
            opt = wait_for(lambda: driver.find_elements(By.XPATH, "//div[@role='option'][contains(.,'Arabic')]"), what="arabic track option")[0]
            time.sleep(0.3)
            ActionChains(driver).move_to_element(opt).perform()
            ActionChains(driver).send_keys(Keys.ENTER).perform()
            wait_for(lambda: "Arabic" in track.text, what="arabic track selected")
            set_value(driver, driver.find_element(By.ID, "add-name"), "hls-clip")
            time.sleep(0.3)
            driver.save_screenshot(os.path.join(args.out, "05b-media-picker.png"))
            by_text(driver, "button", "Start download").click()
            step("HLS download is merged into one MP4")
            wait_for(lambda: "Completed" in (row_status(driver, "hls-clip.mp4") or ""), timeout=90, what="hls completed")
            probe = subprocess.run(
                ["ffprobe", "-v", "error", "-show_entries", "stream=codec_type,height,bit_rate", "-of", "json",
                 os.path.join(args.downloads, "hls-clip.mp4")],
                capture_output=True, text=True, check=True,
            )
            streams = json.loads(probe.stdout)["streams"]
            assert [x["codec_type"] for x in streams] == ["video", "audio"], streams
            assert streams[0]["height"] == 180, streams
            # The Arabic rendition is encoded at 64 kb/s, the English one at 96 kb/s.
            assert int(streams[1]["bit_rate"]) < 80_000, streams
            assert not [f for f in os.listdir(args.downloads) if f.startswith(".velox-")], "work dir removed"

            step("media tools status in settings")
            by_text(driver, "button", "Settings").click()
            wait_for(lambda: driver.find_elements(By.XPATH, "//button[normalize-space(.)='Media']"), what="settings nav")[0].click()
            tool = wait_for(lambda: driver.find_elements(By.CSS_SELECTOR, "[data-testid='tool-FFmpeg']"), timeout=30, what="tool rows")[0]
            assert "Not available" not in driver.execute_script("return arguments[0].textContent", tool)
            driver.save_screenshot(os.path.join(args.out, "05c-media-tools.png"))
            by_text(driver, "button", "All downloads").click()

        step("scheduler: a queue scheduled for the next minute")
        by_text(driver, "button", "Scheduler").click()
        new = wait_for(lambda: driver.find_elements(By.XPATH, "//input[@placeholder='New queue name']"), what="scheduler page")[0]
        set_value(driver, new, "Night")
        driver.find_element(By.XPATH, "//button[@aria-label='Create queue']").click()
        wait_for(lambda: by_text(driver, "h2", "Night"), what="queue editor")
        pick(driver, driver.find_element(By.XPATH, "//button[@role='combobox'][@aria-label='When the queue finishes']"), "Quit Velox")
        field_switch(driver, "Start and stop this queue automatically").click()
        # The next minute boundary (local time, as the app uses); leave at least 15 s to set up.
        now = time.time()
        start = now + 60 - (now % 60) + (60 if now % 60 > 45 else 0)
        hhmm = time.strftime("%H:%M", time.localtime(start))
        start_input = wait_for(lambda: driver.find_elements(By.XPATH, "//input[@type='time']"), what="time input")[0]
        # Enabling fills in a default start time; wait for it before typing ours.
        wait_for(lambda: start_input.get_attribute("value") == "02:00", what="default start time")
        set_value(driver, start_input, hhmm)
        driver.execute_script("arguments[0].blur()", start_input)
        time.sleep(1)
        assert start_input.get_attribute("value") == hhmm, start_input.get_attribute("value")
        driver.save_screenshot(os.path.join(args.out, "06a-scheduler.png"))

        step("add a download to the stopped queue")
        by_text(driver, "button", "All downloads").click()
        wait_for(lambda: by_text(driver, "button", "Add URL"), what="downloads view").click()
        area = wait_for(lambda: driver.find_element(By.ID, "add-url"), what="url field")
        set_value(driver, area, f"{args.server}/file/night.bin?size=2097152")
        wait_for(lambda: by_text(driver, "b", "2.0 MB"), what="probe result")
        by_text(driver, "button", "Advanced options").click()
        pick(driver, wait_for(lambda: driver.find_element(By.XPATH, "//button[@role='combobox'][@aria-label='Queue']"), what="queue select"), "Night")
        by_text(driver, "button", "Add to queue").click()
        wait_for(lambda: "Queued" in (row_status(driver, "night.bin") or ""), what="queued in Night")
        assert time.time() < start - 2, "setup took too long for the scheduled minute"
        time.sleep(1.5)
        assert "Queued" in row_status(driver, "night.bin"), "must wait for the start time"

        step(f"queue starts at {hhmm} and the download completes")
        wait_for(lambda: "Completed" in (row_status(driver, "night.bin") or ""), timeout=start - time.time() + 40, what="scheduled download")
        assert time.time() >= start - 1, "started before its time"
        assert os.path.getsize(os.path.join(args.downloads, "night.bin")) == 2_097_152

        step("post-completion countdown can be cancelled")
        dialog = wait_for(lambda: driver.find_elements(By.CSS_SELECTOR, "[data-testid='post-action-dialog']"), timeout=15, what="countdown")[0]
        assert "Velox will quit" in dialog.text, dialog.text
        driver.save_screenshot(os.path.join(args.out, "06b-post-action.png"))
        dialog.find_element(By.XPATH, ".//button[normalize-space(.)='Cancel']").click()
        wait_for(lambda: not driver.find_elements(By.CSS_SELECTOR, "[data-testid='post-action-dialog']"), what="countdown closed")

        step("clipboard monitor offers a copied download link")
        by_text(driver, "button", "Settings").click()
        wait_for(lambda: by_text(driver, "button", "General"), what="settings").click()
        field_switch(driver, "Watch the clipboard for download links").click()
        time.sleep(2)  # the monitor ignores what was on the clipboard before
        clip_url = f"{args.server}/file/clip.zip?size=1000"
        xclip = subprocess.Popen(["xclip", "-selection", "clipboard"], stdin=subprocess.PIPE)
        xclip.communicate(clip_url.encode(), timeout=5)
        area = wait_for(
            lambda: (lambda e: e[0] if e and e[0].get_attribute("value") == clip_url else None)(driver.find_elements(By.ID, "add-url")),
            timeout=15,
            what="add dialog with the copied link",
        )
        driver.save_screenshot(os.path.join(args.out, "06c-clipboard.png"))
        by_text(driver, "button", "Cancel").click()
        by_text(driver, "button", "All downloads").click()

        step("FTP download through the add dialog")
        ftp_url = "ftp://127.0.0.1:2121/files/3145728/ftp-file.bin"
        by_text(driver, "button", "Add URL").click()
        area = wait_for(lambda: driver.find_element(By.ID, "add-url"), what="url field")
        set_value(driver, area, ftp_url)
        wait_for(lambda: by_text(driver, "b", "3.0 MB"), what="FTP size from SIZE")
        by_text(driver, "button", "Start download").click()
        wait_for(lambda: "Completed" in (row_status(driver, "ftp-file.bin") or ""), timeout=60, what="FTP download")
        assert os.path.getsize(os.path.join(args.downloads, "ftp-file.bin")) == 3145728

        step("SFTP download (credentials in the address, host key recorded)")
        by_text(driver, "button", "Add URL").click()
        area = wait_for(lambda: driver.find_element(By.ID, "add-url"), what="url field")
        set_value(driver, area, "sftp://tester:pw@127.0.0.1:2222/files/2097152/sftp-file.bin")
        wait_for(lambda: by_text(driver, "b", "2.0 MB"), what="SFTP size")
        by_text(driver, "button", "Start download").click()
        wait_for(lambda: "Completed" in (row_status(driver, "sftp-file.bin") or ""), timeout=60, what="SFTP download")
        assert os.path.getsize(os.path.join(args.downloads, "sftp-file.bin")) == 2097152
        driver.save_screenshot(os.path.join(args.out, "06d-ftp-sftp.png"))

        step("statistics page")
        by_text(driver, "button", "Statistics").click()
        wait_for(lambda: by_text(driver, "h1", "Statistics"), what="stats page")
        time.sleep(0.5)
        driver.save_screenshot(os.path.join(args.out, "06-statistics.png"))

        step("switch to Arabic (RTL) and dark theme")
        by_text(driver, "button", "Settings").click()
        wait_for(lambda: by_text(driver, "button", "Appearance"), what="settings").click()
        by_text(driver, "button", "Dark").click()
        lang = driver.find_element(By.XPATH, "//button[@role='combobox']")
        lang.click()
        opt = wait_for(lambda: driver.find_elements(By.XPATH, "//div[@role='option'][contains(.,'العربية')]"), what="lang option")[0]
        time.sleep(0.3)
        ActionChains(driver).move_to_element(opt).perform()
        ActionChains(driver).send_keys(Keys.ENTER).perform()
        wait_for(lambda: driver.execute_script("return document.documentElement.dir") == "rtl", what="rtl")
        time.sleep(0.5)
        driver.save_screenshot(os.path.join(args.out, "07-settings-ar-dark.png"))
        wait_for(lambda: by_text(driver, "button", "كل التنزيلات"), what="arabic sidebar").click()
        time.sleep(0.5)
        driver.save_screenshot(os.path.join(args.out, "08-list-ar-dark.png"))

        print("[e2e] PASS:", ", ".join(results), flush=True)
        return 0
    except Exception as e:  # noqa: BLE001
        driver.save_screenshot(os.path.join(args.out, "failure.png"))
        print(f"[e2e] FAIL after steps {results}: {e!r}", flush=True)
        return 1
    finally:
        driver.quit()


if __name__ == "__main__":
    sys.exit(main())
