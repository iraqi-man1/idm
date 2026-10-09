# Features

Status legend: ✅ implemented and covered by automated tests · 🟡 implemented,
partially tested or manual-test only · ⏳ not yet implemented · 🚫 not
supported (see KNOWN_ISSUES)

"E2E" means the real application (or browser + extension + native host +
application) was driven in an automated test (`tests/e2e/`). "Engine test"
means an integration test that drives the real engine against local servers
(`crates/*/tests/`). See `DEVELOPMENT_PROGRESS.md`. Every status below was
re-checked against the code and tests before the first pull request.

## Download engine

| Feature | Status | Notes |
|---|---|---|
| HTTP downloads | ✅ | engine tests against the local test server, byte-exact |
| HTTPS (rustls + OS trust store) | 🟡 | implemented with `rustls-platform-verifier`; no automated TLS test (the test server is plain HTTP) |
| HTTP/2 | 🟡 | enabled through reqwest (ALPN over TLS); each connection slot uses its own client so segments are not multiplexed; not tested, since automated transfers are cleartext HTTP/1.1 |
| HTTP/3 | 🚫 | see KNOWN_ISSUES |
| Redirects | 🟡 | same-host redirect chains tested; the limit (Settings → Network) and cross-host behaviour are untested; across hosts reqwest drops only Authorization/Cookie (see KNOWN_ISSUES) |
| Basic authentication | ✅ | success and failure tested; credentials encrypted at rest |
| Cookies from browser | ✅ | engine test with a cookie-protected URL; encrypted at rest |
| Custom headers | 🟡 | persistence and encryption of sensitive headers unit-tested; no test that a custom header reaches the server |
| Proxy (system / HTTP / SOCKS5 / none) | 🟡 | implemented in the client builder; no automated proxy test |
| Size / MIME / file name detection (Content-Disposition RFC 5987) | ✅ | |
| Unicode file names, sanitization | ✅ | |
| Large files (> 4 GiB) | 🟡 | u64 offsets throughout; the 4 GiB + 12 KB engine test is opt-in (`#[ignore]`, not run in CI) and passed when run manually |
| Dynamic segmented downloading (1–32 connections) | ✅ | |
| Adaptive connection count | 🟡 | ramp/plateau logic implemented but not unit-tested; server connection-limit detection tested |
| Range support detection, single-connection fallback | ✅ | |
| Direct random-access writes, no merge | ✅ | sparse files on Windows |
| Pause / resume / cancel / restart / retry | ✅ | |
| Resume after app restart / crash / power loss | ✅ | fsync-before-commit checkpoints |
| ETag / Last-Modified validation (If-Range) | ✅ | |
| Expired link detection + address refresh | ✅ | |
| Network interruption recovery | ✅ | backoff retries |
| Global and per-download speed limits | ✅ | |
| Checksum verification (MD5/SHA-1/SHA-256/SHA-512) | ✅ | known-answer tests for all four; SHA-256 verified end to end incl. mismatch; the on-demand "Verify checksum" command is untested |
| FTP (passive, REST resume, multi-connection) | ✅ | in-process test server |
| FTPS (implicit `ftps://`, explicit `ftpes://`) | 🟡 | implemented with OS certificate verification; no automated TLS test (needs a trusted certificate) |
| SFTP (password / `~/.ssh` key login, multi-connection, resume) | 🟡 | password login, multi-connection and resume tested against the in-process SSH server and in E2E; `~/.ssh` key login implemented but untested |
| SSH host key verification (known_hosts, trust on first use, changed-key refusal) | ✅ | |
| Credentials in addresses moved to encrypted storage | ✅ | add, change-address and media manifest paths tested by scanning the database files (including the WAL) for the password; an address naming only the user keeps the stored password (tested) |

## Desktop app

| Feature | Status | Notes |
|---|---|---|
| Sidebar: status filters and categories | ✅ | filter matching, counts and category detection unit-tested; E2E navigates the sidebar |
| Download table: columns, sorting, column chooser | ✅ | E2E (column chooser); sorting unit-tested |
| Toolbar and context-menu actions | 🟡 | pause, resume and show progress in E2E; cancel/remove/restart/refresh address call engine-tested commands; rename, move, pause/resume all, per-download limit, verify checksum, open file/folder, copy URL untested |
| Add URL dialog with probing (size, resume, type, duplicates) | ✅ | E2E (HTTP, FTP, SFTP, HLS) |
| Advanced add options (referrer, auth, cookies, checksum, speed limit) | 🟡 | auth, cookies and checksum engine-tested; referrer and per-download limit untested; dialog fields manual |
| Batch import / "download all links" | 🟡 | URL extraction and `[a-b]` range expansion unit-tested; dialog and extension path manual |
| Detailed progress window (connections, segment map, speed graph) | ✅ | E2E, real per-connection data |
| Statistics page | 🟡 | renders in E2E; counters unit-tested |
| Settings (general, downloads, network, browser, media, appearance, power, updates) | 🟡 | general, media and appearance driven in E2E; persistence and normalization unit-tested; other sections manual |
| Light / dark / system theme | ✅ / 🟡 | light and dark asserted in E2E; following the system theme is manual |
| English and Arabic (RTL) | ✅ | E2E; translation key parity unit test |
| Tray menu (pause/resume all, speed limit, quit) | 🟡 | manual |
| Completion/failure notifications | 🟡 | manual |
| Single instance, URL arguments, start in tray | 🟡 | launch-argument parsing unit-tested |
| Start with the system | 🟡 | autostart plugin; manual |
| Clipboard monitoring | ✅ | E2E (link copied with xclip opens the Add dialog) |
| Queues: limits, priorities, start/stop, move downloads, retry failed | ✅ | engine tests + E2E |
| Scheduler: start/stop times, days of week, scheduled downloads | ✅ | injected-clock tests + real-clock E2E |
| Post-completion actions (quit, sleep, hibernate, shut down) | 🟡 | countdown and cancel in E2E; the OS commands are unit-tested, not executed in CI |
| Hold queues on low battery / metered connection | 🟡 | hold policy, Linux sysfs and macOS `pmset` parsing unit-tested; holds tested with injected power state; the Windows reader is built in Windows CI but never exercised; metered detection is Windows-only |
| Update check and install (signed updater) | 🟡 | manual check and automatic daily check implemented (check decision unit-tested); needs the maintainer's signing key, so no signed update has been built or installed yet |

## Browser integration

| Feature | Status | Notes |
|---|---|---|
| Native messaging host with origin allow-list and schema validation | ✅ | host integration tests on Linux and Windows CI (real host binary, real IPC); browser E2E |
| Authenticated local IPC (named pipe / Unix socket + token) | ✅ | |
| Chrome / Chromium extension (MV3) | ✅ | E2E in Chromium |
| Edge / Brave / Opera | 🟡 | same Chromium build; host registration paths implemented, not E2E-tested |
| Firefox extension (MV3) | 🟡 | built; shared code unit-tested; no Firefox E2E |
| Download capture with browser download cancelled | ✅ | E2E |
| "Download file info" capture dialog | ✅ | E2E |
| Alt+click bypass | ✅ | E2E (synthetic Alt-click on Linux, see KNOWN_ISSUES) |
| Context menus (link, image, media, all links) | 🟡 | manual |
| Popup with connection status | ✅ | E2E |
| Site exclusions, size/extension filters | 🟡 | capture decision (exclusion, extension, minimum size) unit-tested in the extension; the app re-checks only site exclusions (unit-tested); settings fields manual |
| Media detection (HLS / DASH / direct files) per tab | 🟡 | classifier unit-tested; per-tab request detection, badge and popup list not covered by automated tests |
| Floating "Download This Video" button with quality menu | ✅ | E2E with real HLS |
| Per-user host registration, repair, install instructions | 🟡 | Linux registration unit-tested in a fake home; the Windows registry path is never executed by a test (clean-machine gate pending); repair and instructions manual |
| Silent extension installation | 🚫 | not allowed by browsers (KNOWN_ISSUES) |

## Media

| Feature | Status | Notes |
|---|---|---|
| HLS master / media playlists | ✅ | E2E with FFmpeg-generated streams |
| HLS separate audio renditions, alternate audio tracks | ✅ | E2E (app picker) + engine tests |
| HLS AES-128 (identity key format) | ✅ | engine test (decrypted and decoded, key fetched once) |
| HLS fMP4 (`EXT-X-MAP`) and byte ranges | ✅ / 🟡 | fMP4 engine test; `EXT-X-BYTERANGE` only parser-tested |
| DASH SegmentTemplate / SegmentTimeline | ✅ | engine tests |
| DASH SegmentList with byte ranges | ✅ | engine test (range requests asserted) |
| DASH single-file (SegmentBase) representations | 🟡 | whole-file fallback only parser-tested; `indexRange` ignored |
| Quality selection, preferred maximum height | ✅ / 🟡 | explicit quality pick in app and browser E2E; the preferred-maximum-height setting is untested |
| Subtitles: embed (MP4/MKV) or save as .vtt | ✅ / 🟡 | engine tests: WebVTT embedded in MP4 and saved as .vtt; MKV embedding untested |
| DASH subtitle tracks | ⏳ | listed by the probe, not downloaded yet |
| Output MP4 / MKV / original container | ✅ | MP4 in E2E; MKV and original in engine tests |
| Audio extraction MP3 / M4A | ✅ / 🟡 | MP3 engine test; M4A arguments unit-tested |
| Resume after pause / restart (finished segments kept) | ✅ | engine test (manager restarted on the same database) |
| Expired segment URLs reported as expired link | ✅ | engine test |
| DRM detection and refusal | ✅ | engine test (SAMPLE-AES refused, nothing fetched); HLS/DASH/yt-dlp detection unit-tested |
| Live streams | 🚫 | refused with a clear message |
| Web pages via yt-dlp | ✅ | engine tests with yt-dlp's generic extractor (Linux and Windows CI); site-specific extractors depend on the bundled yt-dlp version |
| Login in a manifest address | ✅ | moved to encrypted storage; sent only when the manifest is on the origin the login was entered for, and only to that origin; engine tests (protected server; login for another origin never sent) |
| FFmpeg / yt-dlp bundled with the installer | 🟡 | pinned and checksum-verified; present in both installers (checked in CI); the pinned Windows binaries run the Windows CI media tests; no installed package has run them yet (clean-machine gate pending) |
| Tool status in settings | ✅ | E2E |

## Packaging and release

| Feature | Status | Notes |
|---|---|---|
| Pinned, checksum-verified FFmpeg/ffprobe/yt-dlp | ✅ | Windows x64 and Linux x64; `prepare-release.mjs` refuses mismatches (CI); macOS FFmpeg not pinned |
| Windows NSIS installer with offline WebView2, native-host registration hooks | 🟡 | built by `release.yml`; contents checked by `scripts/check-installer.sh`; never installed on a clean machine yet (gate pending) |
| Linux AppImage | 🟡 | built by `release.yml` and content-checked; never launched by a test or run on a clean machine |
| Linux .deb | 🚫 | not a release format: bundled tools in `/usr/bin` would collide with distro packages (KNOWN_ISSUES) |
| macOS .dmg | ⏳ | needs a pinned FFmpeg source and signing/notarization |
| Signed auto-updates | 🟡 | `release.yml` signs update artifacts once the maintainer configures the key; none has been built yet |
| CI on Linux and Windows, E2E in CI | ✅ | `ci.yml`: Linux (fmt, clippy, all tests), Windows (all tests with the pinned tools), UI/extensions, E2E (desktop + browser); no Windows E2E |
| Third-party notices | ✅ | generated from cargo metadata, pnpm and the sidecar lock; CI fails if the committed file is stale; bundled in both installers |
