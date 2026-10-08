# Development Progress

Last updated: 2026-10-08. Newest entries first. A future session should read
this file, `ARCHITECTURE.md` and `KNOWN_ISSUES.md` before continuing.

## Environment used so far

* Linux x86_64 container (Ubuntu 24.04), Rust 1.97, Node 22, pnpm 10.
* Windows is not available in this environment: Windows builds and the
  clean-machine gate run in GitHub Actions / a Windows VM (see
  `RELEASE_CHECKLIST.md`). GitHub release downloads (yt-dlp, FFmpeg builds)
  are blocked from this container, so sidecars are fetched in CI.

## Phase 2 — download engine (done)

* `velox-http`, `velox-core`, `velox-test-server` implemented.
* Tests: `cargo test -p velox-core` → 10 unit + 21 end-to-end tests, all
  passing; 5 consecutive runs without flakes. Opt-in `large_file_over_4gib`
  (4 GiB + 12 KB, 16 connections) passed in 36 s in release mode.
* Covered: segmented download, no-range server, chunked/unknown length,
  empty file, pause/resume (no re-download), crash recovery from durable
  checkpoint (runtime killed abruptly), interrupted transfers, server
  connection limit, remote change detection, expired link + address
  refresh, malformed Content-Range fallback, 404 vs 503 retry policy, stalled
  connection timeout, cancel, checksums, hostile/Unicode file names,
  redirects, Basic auth, cookies (encrypted at rest), concurrency, global
  speed limit, queued/paused adds, URL probing.

## Phase 1 — foundation (done)

* `velox-types` (62 tests incl. TS binding export), `velox-segments`
  (10 tests incl. randomized invariant test), `velox-persistence` (10 tests).

## Next steps

1. Phase 3: Tauri app shell + React UI wired to `DownloadManager`.
2. Phase 4: native host + extensions.
