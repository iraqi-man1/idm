import { describe, expect, it } from "vitest";
import { expandPattern, extractUrls, isProbablyUrl } from "./utils";

describe("extractUrls", () => {
  it("finds urls in free text and dedupes", () => {
    const text = "see https://a.example/x.zip, and (ftp://b.example/y.iso) https://a.example/x.zip\nsftp://h/z";
    expect(extractUrls(text)).toEqual(["https://a.example/x.zip", "ftp://b.example/y.iso", "sftp://h/z"]);
  });
  it("ignores other schemes", () => {
    expect(extractUrls("javascript:alert(1) file:///etc/passwd")).toEqual([]);
  });
});

describe("expandPattern", () => {
  it("expands numeric ranges with padding", () => {
    expect(expandPattern("https://e.com/f[08-11].zip")).toEqual([
      "https://e.com/f08.zip",
      "https://e.com/f09.zip",
      "https://e.com/f10.zip",
      "https://e.com/f11.zip",
    ]);
    expect(expandPattern("https://e.com/f[1-3].zip")).toHaveLength(3);
  });
  it("expands multiple patterns and respects the limit", () => {
    expect(expandPattern("https://e.com/[1-2]/[1-2]")).toEqual([
      "https://e.com/1/1",
      "https://e.com/1/2",
      "https://e.com/2/1",
      "https://e.com/2/2",
    ]);
    expect(expandPattern("https://e.com/[1-100000]", 10)).toHaveLength(10);
    expect(expandPattern("https://e.com/[5-1]")).toEqual(["https://e.com/[5-1]"]);
  });
  it("detects urls", () => {
    expect(isProbablyUrl(" https://e.com/a ")).toBe(true);
    expect(isProbablyUrl("e.com/a")).toBe(false);
  });
});
