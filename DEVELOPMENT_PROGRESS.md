# Development Progress

Last updated: 2026-10-09 (Phase 8). Newest entries first. A future session should read
this file, `ARCHITECTURE.md` and `KNOWN_ISSUES.md` before continuing.

## Environment used so far

* Linux x86_64 container (Ubuntu 24.04), Rust 1.97, Node 22, pnpm 10,
  FFmpeg 6.1 (system package, used by tests and debug builds only),
  WebKitWebDriver + tauri-driver, Playwright's Chromium build.
* Windows is not available in this environment: Windows builds and the
  clean-machine gate run in GitHub Actions / a Windows VM (see
  `RELEASE_CHECKLIST.md`).

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

## Phase 8 — clean-machine gate, release packaging, final docs (gate NOT executed)

* `docs/testing/clean-machine-test.md`: the Windows 10/11 VM procedure (12
  steps, offline install, bundled tools, segmented download, pause/resume
  across restart, browser integration, native messaging, video, uninstall)
  with an empty results table. **The gate has not been run**; it needs real
  Windows VMs, which this environment does not have. Nothing in
  `RELEASE_CHECKLIST.md` is ticked.
* `tests/clean-machine/check-install.ps1` / `check-uninstall.ps1`: read-only
  evidence collection for steps 4 and 11 (files, tools runnable with a
  minimal PATH, WebView2 registry key, native-messaging keys, cleanup).
  Not executed here (no PowerShell/Windows available).
* `release.yml` can be run manually (Actions → Release → Run workflow): it
  builds the Windows NSIS installer and the Linux AppImage and attaches them
  with `SHA256SUMS.txt` to the run, without creating a release.
* `scripts/check-installer.sh` runs in the `Release` workflow after the
  build: it lists the NSIS installer (7-Zip) or the extracted AppImage and
  fails when the app, native host, FFmpeg, ffprobe, yt-dlp, extensions,
  notices or (Windows) the WebView2 offline installer are missing.
* GitHub Actions results so far:
  * UI/extensions, Rust on Linux (fmt, clippy, all tests) and the desktop +
    browser E2E passed on the first run.
  * The Windows job found real bugs, all fixed with regression tests:
    `available_space()` passed a file path to `GetDiskFreeSpaceExW`, so the
    free-space check was silently skipped whenever the partial file existed;
    and `requeue()` (queue stop, power hold) briefly reported the download
    as Paused before Queued (now `StopReason::Requeue`; the task's exit is
    atomic for observers). The named-pipe client also retries
    `ERROR_PIPE_BUSY`. CI runs with `--no-fail-fast`.
  * Two E2E races (text read during a dialog's fade-in; column menu click)
    were fixed in the test.
  * CI is green on all four jobs (Linux, Windows, UI/extensions, E2E).
  * A manual `Release` run built the Windows NSIS installer (13 min) and the
    Linux AppImage on the first attempt. The second run's content check
    listed the Windows installer (311 MiB): `velox-desktop.exe`,
    `velox-nmh.exe`, `ffmpeg.exe`, `ffprobe.exe`, `yt-dlp.exe`, both
    extension manifests, `THIRD_PARTY_NOTICES.md`, the FFmpeg licenses and
    `MicrosoftEdgeWebView2RuntimeInstaller.exe` (offline, 214 MB). That
    installer has not been run on a clean machine yet.
* Feature-claim audit before the first pull request: every FEATURES.md row
  and the user's security constraints were traced to code and tests by
  independent reviewers, and every disputed finding was re-checked by a
  skeptic. 41 corrections were upheld and applied (mostly ✅ → 🟡 where
  only parsing, a single variant or a manual run backed the claim, and notes
  that said "E2E" for engine tests). Real defects it found, now fixed:
  * changing a download's address stored `user:pass@` in plain text, and a
    login in a media manifest address was kept in plain text. Logins now go
    to the encrypted secrets; media downloads send them only when the
    manifest is on the origin the login was entered for. Regression tests
    that fail on the old code: `remote.rs` (address change, user-name-only
    address), `media.rs` (protected server; login for another origin never
    sent), `context_for` unit test.
  * the database scans in the credential tests ignored the SQLite WAL
    (fixed in those tests, which now also prove they see the record);
  * yt-dlp's cookie file lived next to the downloads and survived a crash.
    It now gets a unique name in the per-user data directory (downloads and
    probes) and is swept at startup. Unique names and the sweep are
    unit-tested; the directory wiring is not.
  * "Check for updates automatically" was stored but never read. It is now
    a daily check (failed checks retried hourly) that offers the update in
    a notification; the checker logic is unit-tested, the React hook that
    drives it is not.
  * `third-party-notices.mjs` could silently produce an empty npm section.
    That is now an error, and CI regenerates the file and fails when the
    committed copy is stale; the script itself has no test.
  A second, independent review of these fixes found two regressions in
  them before they were merged (a `user@host` address blanked a stored
  password; a login was sent to a CDN the server redirected to), a missing
  retry after a failed update check, and an E2E assertion that could not
  fail; all fixed with tests.
  Comparative "IDM-class/IDM-style" wording was removed from docs and
  comments; the non-affiliation statement remains.
* MSRV corrected to 1.90 (Tauri 2.12, russh 0.64 and suppaftp 12 require
  it). README states plainly that no release exists yet.

## Phase 7 — installer, bundled tools, updater, CI, hardening (done here; Windows build runs in CI)

* `resources/sidecars.lock.json`: FFmpeg/ffprobe (BtbN LGPL 8.1) and yt-dlp
  2026.08.19 per target with SHA-256 taken from the upstream checksum lists;
  the Windows x64 and Linux x64 archives were downloaded here and their
  hashes and layout verified. macOS FFmpeg is not pinned yet.
* `scripts/prepare-release.mjs` (fetch + verify + extract, host build,
  extensions; `update-lock`), `tauri.release.conf.json`, NSIS hooks,
  `velox-nmh --register/--unregister`, release profile (thin LTO, stripped).
* Verified here: `prepare-release.mjs` for x86_64-unknown-linux-gnu, a release
  `tauri build` of the Linux package (168 MiB .deb: sidecars next to the
  executable, extensions/notices/FFmpeg license as resources), and the full
  desktop E2E against that packaged release binary — Settings → Media shows
  all three tools as *Bundled* and the HLS step merges with the bundled
  FFmpeg. All 16 media tests also pass with the bundled FFmpeg/ffprobe/yt-dlp
  binaries (fixtures use OpenH264 when libx264 is absent).
* Host registration tested with fake Chrome/Firefox profiles (manifests
  written and removed).
* GitHub Actions: `ci.yml` (Linux, UI, Windows, E2E) and `release.yml`
  (Windows NSIS, Linux AppImage, optional updater signing). Not executed from
  this environment.
* Security pass: removed the unused shell plugin; split window capabilities;
  FFmpeg inputs restricted to local files with forced demuxers; yt-dlp
  without plugin directories.
* `THIRD_PARTY_NOTICES.md` generated (731 crates, 83 npm packages, bundled
  programs, 379 distinct license texts).
* Not done here: the Windows installer itself (no Windows toolchain in this
  container; the whole app is type-checked for the Windows target with
  `cargo check --target x86_64-pc-windows-gnu`), Authenticode signing, and
  the clean-machine gate.

## Phase 6 — queues, scheduler, FTP/SFTP, advanced features (done)

* New crate `velox-scheduler`: schedule arithmetic, start planner,
  battery/metered detection, `Scheduler` runner (queue CRUD, start/stop,
  schedules, power holds, finished-queue events). App: queue commands,
  post-completion actions with a cancellable countdown, clipboard monitor;
  UI: queues in the sidebar, Scheduler page, "Move to queue", queue choice
  in the Add dialog, countdown dialog, power-hold indicator.
* New crate `velox-ftp`: FTP/FTPS/SFTP sources; engine transport, error
  classification and probing; SSH host-key policy; credentials from
  addresses moved to encrypted secrets.
* Found and fixed: a lock-order inversion between `DownloadManager::start`
  and `list` that could deadlock a newly added download while the list was
  read (the scheduler does that on every add). It hung the desktop E2E once;
  a regression test reproduced it (about one run in three with the old
  order) and always passes now.
* Found and fixed in E2E: schedule times entered in the UI were only saved
  on blur; time fields now save as soon as a complete time is entered, and
  edits build on the latest queue state.
* The Windows build is now type-checked here with
  `cargo check --target x86_64-pc-windows-gnu` (MinGW headers for ring); this
  caught a real Windows-only compile error (`SetSuspendState` signature).
* Tests:
  * `cargo test -p velox-scheduler`: 14 unit + 9 integration tests (queue and
    global limits, stop/requeue/resume, schedules driven by an injected
    clock, scheduled downloads, finished-queue event once per batch, low
    battery hold and release, retry of failed downloads, queue deletion).
  * `cargo test -p velox-ftp`: 3 unit + 5 integration tests against the
    in-process FTP and SFTP servers.
  * `cargo test -p velox-core --test remote`: 8 FTP/SFTP download tests.
  * Desktop E2E: a queue scheduled on the real clock for the next minute
    starts its download on time; the post-action countdown appears and is
    cancelled; a copied link opens the Add dialog; FTP and SFTP downloads
    through the Add dialog.

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

1. Execute the clean-machine gate on Windows 10 and 11 VMs with the
   installer from a manual `Release` run and record the results in
   `docs/testing/clean-machine-test.md`.
2. Configure the updater signing key and, when available, an Authenticode
   certificate (`RELEASE_CHECKLIST.md`).
3. Pin the GitHub Actions to commit SHAs; move off the Node 20 based action
   versions GitHub has deprecated.
