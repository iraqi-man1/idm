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
import os
import re
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
        ActionChains(driver).context_click(header).perform()
        item = wait_for(lambda: driver.find_elements(By.XPATH, "//div[@role='menuitemcheckbox'][contains(.,'Connections')]"), what="column menu")[0]
        time.sleep(0.3)  # menu open animation
        ActionChains(driver).move_to_element(item).click().perform()
        time.sleep(0.2)
        ActionChains(driver).send_keys(Keys.ESCAPE).perform()
        wait_for(lambda: by_text(driver, "button", "Connections"), what="connections column")

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
