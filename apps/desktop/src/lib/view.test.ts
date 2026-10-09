import { describe, expect, it } from "vitest";
import type { DownloadInfo } from "@/bindings/DownloadInfo";
import { countByFilter, fraction, matchesFilter, matchesSearch, mergeRow, sortRows } from "./view";

function d(over: Partial<DownloadInfo>): DownloadInfo {
  return {
    id: over.id ?? crypto.randomUUID(),
    url: "https://example.com/file.zip",
    final_url: null,
    page_url: null,
    file_name: "file.zip",
    save_dir: "/downloads",
    kind: "http",
    status: "paused",
    category: "archive",
    total_size: 1000,
    downloaded: 0,
    resumable: true,
    max_connections: 8,
    active_connections: 0,
    speed: 0,
    avg_speed: 0,
    eta_secs: null,
    error: null,
    error_kind: null,
    mime: null,
    referer: null,
    queue_id: "main",
    priority: 0,
    created_at: 1,
    started_at: null,
    completed_at: null,
    scheduled_at: null,
    next_retry_at: null,
    retry_count: 0,
    speed_limit: 0,
    checksum: null,
    checksum_ok: null,
    media: null,
    elapsed_ms: 0,
    has_secrets: false,
    ...over,
  };
}

describe("filters", () => {
  it("matches status and category filters", () => {
    expect(matchesFilter(d({ status: "downloading" }), "downloading")).toBe(true);
    expect(matchesFilter(d({ status: "retrying" }), "downloading")).toBe(true);
    expect(matchesFilter(d({ status: "completed" }), "downloading")).toBe(false);
    expect(matchesFilter(d({ status: "cancelled" }), "paused")).toBe(true);
    expect(matchesFilter(d({ status: "queued", scheduled_at: 5 }), "queued")).toBe(false);
    expect(matchesFilter(d({ status: "queued", scheduled_at: 5 }), "scheduled")).toBe(true);
    expect(matchesFilter(d({ category: "video" }), "cat:video")).toBe(true);
    expect(matchesFilter(d({ category: "video" }), "cat:music")).toBe(false);
  });

  it("searches name, url and folder case-insensitively", () => {
    const x = d({ file_name: "Report.PDF", url: "https://Host.example/a" });
    expect(matchesSearch(x, "report")).toBe(true);
    expect(matchesSearch(x, "host.EXAMPLE")).toBe(true);
    expect(matchesSearch(x, "nothing")).toBe(false);
  });

  it("counts", () => {
    const c = countByFilter([d({ status: "failed" }), d({ status: "failed" }), d({ status: "completed" })], ["failed", "completed", "all"]);
    expect(c).toEqual({ failed: 2, completed: 1, all: 3 });
  });
});

describe("rows", () => {
  it("merges live progress over the stored record", () => {
    const base = d({ id: "x", status: "downloading", downloaded: 10 });
    const row = mergeRow(base, {
      id: "x",
      status: "downloading",
      downloaded: 500,
      total_size: 1000,
      speed: 42,
      avg_speed: 40,
      eta_secs: 12,
      active_connections: 4,
      elapsed_ms: 100,
      segments: [],
      stage: null,
    });
    expect(row.downloaded).toBe(500);
    expect(row.speed).toBe(42);
    expect(fraction(row)).toBe(0.5);
  });

  it("fraction handles unknown, empty and completed", () => {
    expect(fraction({ downloaded: 5, total_size: null, status: "downloading" })).toBeNull();
    expect(fraction({ downloaded: 0, total_size: 0, status: "paused" })).toBe(1);
    expect(fraction({ downloaded: 0, total_size: null, status: "completed" })).toBe(1);
  });

  it("sorts by key and direction with a stable tiebreak", () => {
    const rows = [d({ file_name: "b", created_at: 1 }), d({ file_name: "a", created_at: 2 }), d({ file_name: "c", created_at: 3 })].map((x) =>
      mergeRow(x, undefined),
    );
    expect(sortRows(rows, { key: "name", dir: "asc" }).map((r) => r.file_name)).toEqual(["a", "b", "c"]);
    expect(sortRows(rows, { key: "date_added", dir: "desc" }).map((r) => r.file_name)).toEqual(["c", "a", "b"]);
  });
});
