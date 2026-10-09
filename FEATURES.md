# Features

Status legend: ✅ implemented and covered by automated tests · 🟡 implemented,
partially tested or manual-test only · ⏳ not yet implemented · 🚫 not
supported (see KNOWN_ISSUES)

"E2E" means the real application (or browser + extension + native host +
application) was driven in an automated test; see `DEVELOPMENT_PROGRESS.md`.

## Download engine

| Feature | Status | Notes |
|---|---|---|
| HTTP / HTTPS downloads | ✅ | rustls + OS trust store |
| HTTP/2 | ✅ | negotiated by ALPN (reqwest/hyper) |
| HTTP/3 | 🚫 | see KNOWN_ISSUES |
| Redirects | ✅ | configurable limit; sensitive headers dropped cross-host |
| Basic authentication | ✅ | credentials encrypted at rest |
| Cookies from browser | ✅ | encrypted at rest |
| Custom headers | ✅ | sensitive ones encrypted |
| Proxy (system / HTTP / SOCKS5 / none) | 🟡 | implemented in client builder; no automated proxy test yet |
| Size / MIME / file name detection (Content-Disposition RFC 5987) | ✅ | |
| Unicode file names, sanitization | ✅ | |
| Large files (> 4 GiB) | ✅ | opt-in test |
| Dynamic segmented downloading (1–32 connections) | ✅ | |
| Adaptive connection count | 🟡 | ramp/plateau logic implemented; server-limit detection tested; ramp seen in E2E |
| Range support detection, single-connection fallback | ✅ | |
| Direct random-access writes, no merge | ✅ | sparse files on Windows |
| Pause / resume / cancel / restart / retry | ✅ | |
| Resume after app restart / crash / power loss | ✅ | fsync-before-commit checkpoints |
| ETag / Last-Modified validation (If-Range) | ✅ | |
| Expired link detection + address refresh | ✅ | |
| Network interruption recovery | ✅ | backoff retries |
| Global and per-download speed limits | ✅ | |
| Checksum verification (MD5/SHA-1/SHA-256/SHA-512) | ✅ | |
| FTP (passive, REST resume, multi-connection) | ✅ | in-process test server |
| FTPS (implicit `ftps://`, explicit `ftpes://`) | 🟡 | implemented with OS certificate verification; no automated TLS test (needs a trusted certificate) |
| SFTP (password / `~/.ssh` key login, multi-connection, resume) | ✅ | in-process SSH server |
| SSH host key verification (known_hosts, trust on first use, changed-key refusal) | ✅ | |
| Credentials in addresses moved to encrypted storage | ✅ | |

## Desktop app

| Feature | Status | Notes |
|---|---|---|
| Sidebar: status filters and categories | ✅ | E2E |
| Download table: columns, sorting, column chooser | ✅ | E2E (column chooser) |
| Toolbar and context-menu actions | 🟡 | pause/resume E2E; others call tested engine commands |
| Add URL dialog with probing (size, resume, type, duplicates) | ✅ | E2E |
| Advanced add options (referrer, auth, cookies, checksum, speed limit) | 🟡 | engine-tested; dialog fields manual |
| Batch import / "download all links" | 🟡 | manual |
| Detailed progress window (connections, segment map, speed graph) | ✅ | E2E, real per-connection data |
| Statistics page | 🟡 | renders in E2E; numbers from SQLite counters |
| Settings (general, downloads, network, browser, media, appearance, power, updates) | 🟡 | media + appearance in E2E |
| Light / dark / system theme | ✅ | E2E (dark) |
| English and Arabic (RTL) | ✅ | E2E; translation key parity unit test |
| Tray menu (pause/resume all, speed limit, quit) | 🟡 | manual |
| Completion/failure notifications | 🟡 | manual |
| Single instance, URL arguments, start in tray | 🟡 | launch-argument parsing unit-tested |
| Start with the system | 🟡 | autostart plugin; manual |
| Clipboard monitoring | ✅ | E2E (link copied with xclip opens the Add dialog) |
| Queues: limits, priorities, start/stop, move downloads, retry failed | ✅ | engine tests + E2E |
| Scheduler: start/stop times, days of week, scheduled downloads | ✅ | injected-clock tests + real-clock E2E |
| Post-completion actions (quit, sleep, hibernate, shut down) | 🟡 | countdown and cancel in E2E; the OS commands are unit-tested, not executed in CI |
| Hold queues on low battery / metered connection | 🟡 | policy and Linux battery tested; Windows/macOS readers compile-checked only |
| Signed auto-update | 🟡 | UI + plugin wired; signing keys and release feed in Phase 7 |

## Browser integration

| Feature | Status | Notes |
|---|---|---|
| Native messaging host with origin allow-list and schema validation | ✅ | host E2E tests |
| Authenticated local IPC (named pipe / Unix socket + token) | ✅ | |
| Chrome / Chromium extension (MV3) | ✅ | E2E in Chromium |
| Edge / Brave / Opera | 🟡 | same Chromium build; host registration paths implemented, not E2E-tested |
| Firefox extension (MV3) | 🟡 | built and unit-tested; no Firefox E2E yet |
| Download capture with browser download cancelled | ✅ | E2E |
| "Download file info" capture dialog | ✅ | E2E |
| Alt+click bypass | ✅ | E2E (synthetic Alt-click on Linux, see KNOWN_ISSUES) |
| Context menus (link, image, media, all links) | 🟡 | manual |
| Popup with connection status | ✅ | E2E |
| Site exclusions, size/extension filters | 🟡 | exclusion matching unit-tested |
| Media detection (HLS / DASH / direct files) per tab | ✅ | classifier unit tests; used in E2E |
| Floating "Download This Video" button with quality menu | ✅ | E2E with real HLS |
| Per-user host registration, repair, install instructions | 🟡 | Linux manifest paths exercised; Windows registry path runs in CI/clean-machine test |
| Silent extension installation | 🚫 | not allowed by browsers (KNOWN_ISSUES) |

## Media

| Feature | Status | Notes |
|---|---|---|
| HLS master / media playlists | ✅ | E2E with FFmpeg-generated streams |
| HLS separate audio renditions, alternate audio tracks | ✅ | E2E (app picker + engine) |
| HLS AES-128 (identity key format) | ✅ | E2E |
| HLS fMP4 (`EXT-X-MAP`) and byte ranges | ✅ | E2E (fMP4); byte ranges unit-tested |
| DASH SegmentTemplate / SegmentTimeline | ✅ | E2E |
| DASH SegmentList with byte ranges | ✅ | E2E |
| DASH single-file (SegmentBase) representations | 🟡 | parser unit test |
| Quality selection, preferred maximum height | ✅ | E2E |
| Subtitles: embed (MP4/MKV) or save as .vtt | ✅ | E2E (HLS WebVTT) |
| DASH subtitle tracks | ⏳ | listed by the probe, not downloaded yet |
| Output MP4 / MKV / original container | ✅ | E2E |
| Audio extraction MP3 / M4A | ✅ / 🟡 | MP3 E2E; M4A arguments unit-tested |
| Resume after pause / restart (finished segments kept) | ✅ | E2E |
| Expired segment URLs reported as expired link | ✅ | E2E |
| DRM detection and refusal | ✅ | E2E (SAMPLE-AES); DASH ContentProtection unit-tested |
| Live streams | 🚫 | refused with a clear message |
| Web pages via yt-dlp | ✅ | E2E with yt-dlp's generic extractor; site-specific extractors depend on the bundled yt-dlp version |
| FFmpeg / yt-dlp bundled with the installer | 🟡 | pinned + verified sidecars; Linux release package built and E2E-tested here; Windows installer built in CI (not run here) |
| Tool status in settings | ✅ | E2E |

## Packaging and release

| Feature | Status | Notes |
|---|---|---|
| Pinned, checksum-verified FFmpeg/ffprobe/yt-dlp | ✅ | Windows x64 and Linux x64 hashes verified by download; macOS FFmpeg not pinned |
| Windows NSIS installer with offline WebView2, native-host registration hooks | 🟡 | built by `release.yml` on GitHub Actions; contents (app, host, FFmpeg, ffprobe, yt-dlp, extensions, notices, WebView2 offline installer) checked by `scripts/check-installer.sh`; never installed on a clean machine yet (gate pending) |
| Linux AppImage / .deb | 🟡 | .deb release build verified here; AppImage built and content-checked by `release.yml`, not run on a clean machine; see KNOWN_ISSUES for .deb |
| macOS .dmg | ⏳ | needs a pinned FFmpeg source and signing/notarization |
| Signed auto-updates | 🟡 | updater wired; signing key must be configured by the maintainer |
| CI on Linux and Windows, E2E in CI | 🟡 | workflows written; first run pending |
| Third-party notices | ✅ | generated from cargo metadata and npm |
