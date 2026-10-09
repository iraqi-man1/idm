import { afterEach, describe, expect, it } from "vitest";
import { formatBytes, formatDuration, formatSpeed, parseSize, setFormatLocale } from "./format";

afterEach(() => setFormatLocale("en"));

describe("format", () => {
  it("formats bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(1021 * 1024)).toBe("1.0 MB");
    expect(formatBytes(999)).toBe("999 B");
  });
  it("formats speed and duration", () => {
    expect(formatSpeed(2 * 1024 ** 2)).toBe("2.0 MB/s");
    expect(formatSpeed(0)).toBe("—");
    expect(formatDuration(59)).toBe("59s");
    expect(formatDuration(3725)).toBe("1h 2m");
    expect(formatDuration(90000)).toBe("1d 1h");
  });
  it("localizes to Arabic with western digits", () => {
    setFormatLocale("ar");
    expect(formatBytes(1536)).toBe("1.5 ك.ب");
    expect(formatDuration(125)).toBe("2د 5ث");
  });
  it("parses sizes", () => {
    expect(parseSize("2 MB")).toBe(2 * 1024 ** 2);
    expect(parseSize("512k")).toBe(512 * 1024);
    expect(parseSize("1.5m/s")).toBe(Math.round(1.5 * 1024 ** 2));
    expect(parseSize("100")).toBe(100);
    expect(parseSize("fast")).toBeNull();
  });
});
