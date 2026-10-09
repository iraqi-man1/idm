import { describe, expect, it } from "vitest";
import { cookieHeader, extOf, isExcluded, RecentSet, shouldCapture, type CaptureConfig } from "./util";

const cfg: CaptureConfig = { enabled: true, extensions: ["zip", "mp4", "exe"], minSize: 1000, excluded: ["bank.example"] };

describe("extOf", () => {
  it("handles names, urls and edge cases", () => {
    expect(extOf("C:\\Users\\a\\Downloads\\setup.EXE")).toBe("exe");
    expect(extOf("https://e.com/dir/file.tar.gz?x=1")).toBe("gz");
    expect(extOf("https://e.com/dir/")).toBeNull();
    expect(extOf(".bashrc")).toBeNull();
    expect(extOf("https://e.com/%D9%85%D9%84%D9%81.zip")).toBe("zip");
    expect(extOf(null)).toBeNull();
  });
});

describe("shouldCapture", () => {
  const base = { url: "https://cdn.example.com/a.zip", fileSize: 5000 };
  it("captures matching downloads", () => {
    expect(shouldCapture(base, cfg)).toEqual({ capture: true });
    expect(shouldCapture({ url: "https://e.com/download?id=1", mime: "video/mp4" }, cfg)).toEqual({ capture: true });
    expect(shouldCapture({ url: "https://e.com/x", filename: "/home/u/Downloads/Setup.exe" }, cfg)).toEqual({ capture: true });
  });
  it("respects every exclusion rule", () => {
    expect(shouldCapture(base, { ...cfg, enabled: false })).toMatchObject({ reason: "disabled" });
    expect(shouldCapture({ url: "blob:https://e.com/1" }, cfg)).toMatchObject({ reason: "scheme" });
    expect(shouldCapture({ url: "data:text/plain,hi" }, cfg)).toMatchObject({ reason: "scheme" });
    expect(shouldCapture({ ...base, byExtensionId: "abc" }, cfg)).toMatchObject({ reason: "extension" });
    expect(shouldCapture({ ...base, bypass: true }, cfg)).toMatchObject({ reason: "bypass" });
    expect(shouldCapture({ ...base, incognito: true }, cfg)).toMatchObject({ reason: "incognito" });
    expect(shouldCapture({ ...base, referrer: "https://www.bank.example/x" }, cfg)).toMatchObject({ reason: "excluded" });
    expect(shouldCapture({ url: "https://e.com/page.html" }, cfg)).toMatchObject({ reason: "type" });
    expect(shouldCapture({ ...base, fileSize: 10 }, cfg)).toMatchObject({ reason: "small" });
    expect(shouldCapture({ ...base, recentlyHandled: true }, cfg)).toMatchObject({ reason: "duplicate" });
  });
});

describe("helpers", () => {
  it("excludes subdomains but not lookalikes", () => {
    expect(isExcluded(["https://a.bank.example/"], ["bank.example"])).toBe(true);
    expect(isExcluded(["https://notbank.example/"], ["bank.example"])).toBe(false);
    expect(isExcluded([null, "not a url"], ["bank.example"])).toBe(false);
  });
  it("builds cookie headers", () => {
    expect(cookieHeader([{ name: "a", value: "1" }, { name: "b", value: "2" }])).toBe("a=1; b=2");
    expect(cookieHeader([])).toBeNull();
  });
  it("expires recent entries", () => {
    const r = new RecentSet(1000);
    r.add("x", 0);
    expect(r.has("x", 500)).toBe(true);
    expect(r.has("x", 2000)).toBe(false);
  });
});
