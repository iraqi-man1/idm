# Development Progress

Last updated: 2026-10-09. Newest entries first. A future session should read
this file, `ARCHITECTURE.md` and `KNOWN_ISSUES.md` before continuing.

## Environment used so far

* Linux x86_64 container (Ubuntu 24.04), Rust 1.97, Node 22, pnpm 10,
  FFmpeg 6.1 (system package, used by tests and debug builds only),
  WebKitWebDriver + tauri-driver, Playwright's Chromium build.
* Windows is not available in this environment: Windows builds and the
  clean-machine gate run in GitHub Actions / a Windows VM (see
  `RELEASE_CHECKLIST.md`). GitHub release downloads (yt-dlp, FFmpeg builds)
  are blocked from this container, so sidecars are fetched in CI.

## How to run the tests

```bash
cargo test --workspace                       # all Rust unit + integration tests
pnpm --filter desktop exec vitest run        # UI unit tests
pnpm --filter extensions test                # extension unit tests
tests/e2e/run-linux.sh <python>              # desktop app E2E (Xvfb + tauri-driver)
tests/e2e/run-browser-linux.sh <python>      # Chromium + extension + host + app E2E
```

The E2E runners need `selenium` and `playwright` in `<python>`, a debug build
(`pnpm --filter desktop build && cargo build -p velox-desktop --features
custom-protocol -p velox-nm -p velox-test-server`) and the built extension
(`pnpm --filter extensions build`). Media tests need `ffmpeg`/`ffprobe`;
`VELOX_REQUIRE_MEDIA_TESTS=1` turns a missing FFmpeg into a failure instead
of a skip, and `VELOX_YTDLP=<path>` + `VELOX_REQUIRE_YTDLP_TESTS=1` do the
same for the yt-dlp tests.

## Phase 5 — media (done)

* New crate `velox-media` (`crates/media-engine`): HLS/DASH parsers, segment
  downloader with AES-128, FFmpeg muxing/audio extraction, yt-dlp extractor,
  tool discovery; `MediaEngine` is the manager's `MediaRunner`.
* Manager: media work directories are deleted on cancel/remove; the preferred
  quality setting applies when no rendition was chosen; playlist names
  (`master.m3u8`) no longer leak into file names.
* App: `media_bridge.rs` answers `probe_media` / `download_media` from the
  extension and the `probe_media` / `get_media_tools` commands. The Add
  Download dialog shows a quality / audio track / output format / subtitle
  picker for HLS, DASH and (on request) web pages; Settings → Media shows the
  tools in use.
* Extension: quality options moved to a module shared with the app; the
  floating button no longer wraps its label when fonts load late.
* Tests:
  * `cargo test -p velox-media`: 18 unit tests + 16 end-to-end tests that
    generate real media with FFmpeg (HLS TS + separate audio + subtitles,
    alternate audio track, AES-128, fMP4, DASH template/timeline, DASH
    SegmentList byte ranges), download them through the manager from the test
    server's `/static` route, and verify the output with ffprobe and a full
    decode. Also: DRM refusal, expired segment URL → `LinkExpired`,
    pause + application restart + resume without re-downloading finished
    segments, cancel removes the work directory, yt-dlp probe/download of a
    page with a `<video>` (yt-dlp 2026.08.19 via `VELOX_YTDLP`).
  * Desktop E2E (`run-linux.sh`), new steps: HLS master in the Add dialog →
    pick 180p and the Arabic audio track → merged MP4 verified with ffprobe
    (180p, 64 kb/s audio); Settings → Media lists the tools.
  * Browser E2E (`run-browser-linux.sh`), new steps: floating "Download This
    Video" button over an HLS `<video>` in Chromium → menu lists 360p / 180p
    / audio only (probed by the app through the native host) → 180p → Velox
    downloads and merges it (ffprobe-verified).
  * 3 new unit tests for the browser media request mapping (URL validation,
    subtitle defaults, direct-file naming).

## Phase 4 — browser integration (done)

* `velox-nm`: framing, IPC (named pipe / Unix socket + token handshake),
  native host `velox-nmh` (origin allow-list, schema validation, app launch),
  per-user host registration for Chrome/Edge/Brave/Chromium/Firefox.
* MV3 extensions (Chromium and Firefox builds from shared TypeScript):
  download capture with browser-side cancel, Alt+click bypass, context
  menus, popup with live connection status, options page, site exclusions,
  media detection, floating video button.
* App: capture dialog window, browser integration settings (status, repair,
  install instructions), protocol compatibility check.
* Tests: protocol round-trip tests (found and fixed a reply-id clash), 5 host
  end-to-end tests over real IPC, extension unit tests, and the browser E2E:
  link click in Chromium → capture dialog → Velox downloads the file, the
  browser's download is cancelled, Alt+click leaves the download to the
  browser.

## Phase 3 — desktop app (done)

* Tauri 2 shell (tray, single instance, notifications, autostart, updater
  plugin wiring), typed commands/events, React UI: sidebar filters and
  categories, virtualized table with configurable columns, toolbar and
  context menus, Add URL dialog with probing, batch import, progress window
  (per-connection table, segment map, speed graph), statistics page, settings
  sections, light/dark themes, English/Arabic with RTL.
* Found and fixed: commands invoked from non-runtime threads panicked
  (manager now spawns on a captured runtime handle; regression test added).
* Desktop E2E: add → multi-connection ramp → progress window → pause →
  resume → complete → file verified; statistics; Arabic RTL + dark theme.

## Phase 2 — download engine (done)

* `velox-http`, `velox-core`, `velox-test-server` implemented.
* Tests: `cargo test -p velox-core` → 10 unit + 22 end-to-end tests, all
  passing; 5 consecutive runs without flakes. Opt-in `large_file_over_4gib`
  (4 GiB + 12 KB, 16 connections) passed in 36 s in release mode.
* Covered: segmented download, no-range server, chunked/unknown length,
  empty file, pause/resume (no re-download), crash recovery from durable
  checkpoint (runtime killed abruptly), interrupted transfers, server
  connection limit, remote change detection, expired link + address
  refresh, malformed Content-Range fallback, 404 vs 503 retry policy, stalled
  connection timeout, cancel, checksums, hostile/Unicode file names,
  redirects, Basic auth, cookies (encrypted at rest), concurrency, global
  speed limit, queued/paused adds, URL probing, commands from non-runtime
  threads.

## Phase 1 — foundation (done)

* `velox-types` (73 tests incl. TS binding export), `velox-segments`
  (10 tests incl. randomized invariant test), `velox-persistence` (10 tests).

## Next steps

1. Phase 6: queues and scheduler (start/stop times, days, post-completion
   shutdown/sleep), max concurrent downloads per queue, clipboard monitor,
   FTP/SFTP, battery/metered handling, queue UI.
2. Phase 7: sidecar fetch script with pinned SHA-256, `externalBin` bundling
   (velox-nmh, ffmpeg, ffprobe, yt-dlp), NSIS hooks for native-messaging
   registration, updater signing, GitHub Actions for Windows/macOS/Linux,
   THIRD_PARTY_NOTICES.
3. Phase 8: clean-machine test on Windows 10/11 VMs (not marked passed until
   executed), release packaging, final docs.
