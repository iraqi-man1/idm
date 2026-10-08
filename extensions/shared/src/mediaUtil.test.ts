import { describe, expect, it } from "vitest";
import { classifyMedia } from "./mediaUtil";

describe("classifyMedia", () => {
  it("detects manifests by extension or MIME", () => {
    expect(classifyMedia("https://e.com/live/master.m3u8?token=1", null, null)).toBe("hls");
    expect(classifyMedia("https://e.com/playlist", "application/vnd.apple.mpegurl", 500)).toBe("hls");
    expect(classifyMedia("https://e.com/manifest.mpd", null, null)).toBe("dash");
    expect(classifyMedia("https://e.com/m", "application/dash+xml; charset=utf-8", 1)).toBe("dash");
  });
  it("detects complete media files", () => {
    expect(classifyMedia("https://e.com/v.mp4", "video/mp4", 5_000_000)).toBe("direct");
    expect(classifyMedia("https://e.com/a.mp3", "application/octet-stream", 3_000_000)).toBe("direct");
    expect(classifyMedia("https://e.com/stream", "audio/ogg", null)).toBe("direct");
  });
  it("ignores segments, tiny files and non-media", () => {
    expect(classifyMedia("https://e.com/seg-001.ts", "video/mp2t", 900_000)).toBeNull();
    expect(classifyMedia("https://e.com/chunk.m4s", "video/mp4", 900_000)).toBeNull();
    expect(classifyMedia("https://e.com/init", "video/iso.segment", 900)).toBeNull();
    expect(classifyMedia("https://e.com/ping.mp4", "video/mp4", 1000)).toBeNull();
    expect(classifyMedia("https://e.com/page.html", "text/html", 50_000)).toBeNull();
    expect(classifyMedia("https://e.com/x.mp4", "text/html", 50_000_000)).toBeNull();
    expect(classifyMedia("blob:https://e.com/1", "video/mp4", 5_000_000)).toBeNull();
  });
});

import { buildOptions } from "./mediaUtil";
import type { MediaFormat } from "@bindings/MediaFormat";
import type { MediaProbeResult } from "@bindings/MediaProbeResult";

function f(over: Partial<MediaFormat>): MediaFormat {
  return {
    id: "x",
    label: "",
    has_video: true,
    has_audio: true,
    ext: "mp4",
    width: null,
    height: null,
    fps: null,
    bitrate: null,
    vcodec: null,
    acodec: null,
    filesize: null,
    language: null,
    url: null,
    ...over,
  };
}

function result(formats: MediaFormat[]): MediaProbeResult {
  return { kind: "page", url: "https://e.com", title: "t", duration_secs: null, thumbnail: null, formats, subtitles: [], extractor: null, is_live: false, drm_protected: false };
}

describe("buildOptions", () => {
  it("lists muxed variants highest first", () => {
    const o = buildOptions(result([f({ id: "a", height: 360 }), f({ id: "b", height: 1080, vcodec: "avc1.640028" }), f({ id: "c", height: 720 })]));
    expect(o.map((x) => x.label)).toEqual(["1080p", "720p", "360p"]);
    expect(o[0].detail).toContain("H.264");
    expect(o[0].audioFormatId).toBeNull();
  });
  it("pairs video-only renditions with the best audio and adds audio-only", () => {
    const o = buildOptions(
      result([
        f({ id: "v1", height: 1080, has_audio: false, bitrate: 4_000_000, filesize: 100 * 1024 * 1024 }),
        f({ id: "v1b", height: 1080, has_audio: false, bitrate: 2_000_000 }),
        f({ id: "v2", height: 480, has_audio: false, bitrate: 900_000 }),
        f({ id: "a1", has_video: false, bitrate: 128_000, acodec: "mp4a.40.2", filesize: 5 * 1024 * 1024 }),
        f({ id: "a2", has_video: false, bitrate: 64_000 }),
      ]),
    );
    expect(o.map((x) => [x.label, x.formatId, x.audioFormatId, x.container])).toEqual([
      ["1080p", "v1", "a1", "mp4"],
      ["480p", "v2", "a1", "mp4"],
      ["Audio only", "a1", null, "m4a"],
    ]);
    expect(o[0].size).toBe(105 * 1024 * 1024);
  });
  it("falls back to the single format of a media playlist", () => {
    const o = buildOptions(result([f({ id: "only", has_video: true, has_audio: true, height: null, bitrate: 1_500_000 })]));
    expect(o).toHaveLength(1);
    expect(o[0].label).toBe("1500 kbps");
  });
});
