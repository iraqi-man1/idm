# Velox Download Manager — Architecture

Velox is a native desktop download manager (Tauri 2 + Rust + React) with a
multi-connection segmented download engine, browser integration through Native
Messaging, and a media subsystem built on bundled FFmpeg / yt-dlp.

This document describes how the pieces fit together. `DEVELOPMENT_PROGRESS.md`
says what is implemented and tested right now.

## Repository layout

```
Cargo.toml                 Rust workspace
package.json               pnpm workspace (desktop UI + extensions)
crates/
  shared-types/            velox-types: data contract (serde + ts-rs → TypeScript)
  segment-manager/         velox-segments: pure segmentation logic (no I/O)
  http-engine/             velox-http: HTTP(S) probe / range requests / header parsing
  ftp-engine/              velox-ftp: FTP(S) / SFTP transport
  persistence/             velox-persistence: SQLite, migrations, encrypted secrets
  download-core/           velox-core: engine (tasks, writer, manager)
  scheduler/               velox-scheduler: queues, schedules, post-actions
  media-engine/            velox-media: HLS/DASH parsing, FFmpeg/yt-dlp integration
  native-messaging/        velox-nm: protocol framing, IPC, native host binary
apps/desktop/              Tauri 2 app: React UI (src/) + Rust shell (src-tauri/)
extensions/
  shared/                  TypeScript shared by both extension builds
  chromium/                Manifest V3 build for Chrome / Edge / Brave / Opera
  firefox/                 Manifest V3 build for Firefox
resources/<os>/<arch>/     Bundled third-party tools (fetched, checksum-pinned)
installer/                 NSIS hooks, native-messaging registration templates
tests/test-server/         Local HTTP server that simulates hostile servers
scripts/                   Build, sidecar fetch, packaging scripts
docs/                      User and developer documentation
```

## Dependency rules

```
velox-types  ←  velox-segments
     ↑              ↑
velox-http     velox-persistence     velox-ftp
     ↑              ↑                   ↑
     └──────── velox-core ──────────────┘
                   ↑
     velox-scheduler   velox-media
                   ↑
          apps/desktop/src-tauri  ←→  React UI (typed commands + events)
velox-nm (protocol + host) depends only on velox-types.
```

* The engine crates never depend on Tauri or React.
* The browser extension never touches the database; it talks to the app
  through the native host and the app's local IPC endpoint only.
* The UI talks to Rust only through typed Tauri commands and events whose
  payload types are generated from `velox-types` (`apps/desktop/src/bindings`).

## Download engine

### Probe = first connection

A download starts with `GET` + `Range: bytes=0-`. The response tells the
engine the size (`Content-Range`), whether ranges work (206 vs 200), the
validators (`ETag`, `Last-Modified`) and the file name
(`Content-Disposition`, RFC 6266/5987). The same response body then becomes
the first connection's data stream, so no round trip is wasted.

Servers that answer `bytes=0-` with 200 but advertise `Accept-Ranges` are
verified with a one-byte range request before any splitting happens. A
server that sends a malformed `Content-Range` triggers a fallback to a plain
`GET` over one connection.

### Dynamic segmentation (velox-segments)

The file is a set of disjoint segments `[start, end)`. Each segment has a
`received` cursor (bytes claimed by a connection) and a `written` cursor
(bytes on disk). A new connection takes an unassigned segment or splits the
segment with the largest unreceived remainder in half; the owner of the split
segment just stops at the new end. Bytes are claimed under a lock before they
are queued for the disk, so a byte can never be claimed twice. A randomized
test simulates thousands of concurrent claims, splits and connection failures
and checks exact coverage.

### Adaptive connections

`Controller` (in `download-core/src/task.rs`) starts with up to 4 connections
and adds one at a time while the aggregate speed improves by at least 5 %.
After three non-improving additions the level is frozen and re-tested every
20 s. Servers that reject extra connections (403/429/503/509 or repeated
connection errors while other connections work) cap the level at the number
of working connections. A range-ignoring or inconsistent extra connection
disables splitting for the download.

### Disk writing and crash safety

Each download has one writer thread doing positional writes into a single
partial file (`name.ext.vdpart`). There is no merge step. On Windows the file
is marked sparse before it is sized, so writes far into a large file do not
trigger synchronous zero-filling.

Every 3 seconds the supervisor asks the writer for a checkpoint:

1. snapshot the segment table (only completed writes are counted),
2. `fsync` the partial file,
3. commit the snapshot to SQLite (`synchronous=FULL`, WAL).

Persisted progress therefore never exceeds durable data. After a crash or
power loss the engine resumes from the last checkpoint and re-downloads at
most a few seconds of data.

### Resume validation

A resume sends `Range: bytes=N-` with `If-Range: <strong ETag or
Last-Modified>`. A 206 with the same total size continues; a 200 with
different validators means the remote file changed → the download fails with
`RemoteChanged` and the partial file is left untouched. 403/404/410 or an HTML
page instead of the file → `LinkExpired`; the user can supply a new address
(`update_url`) and the next start validates that it serves the same entity.
Servers without range support are restarted from zero, never appended to.

### Retries

Transient errors (timeouts, resets, 5xx, 429) end the current attempt; the
task retries with exponential backoff (honouring `Retry-After`) up to
`max_retries`, resetting the counter whenever real progress was made.
Non-transient errors (404, auth, disk full, remote changed) fail immediately.

### Manager

`DownloadManager` keeps every record in memory behind `Arc<Mutex<_>>`, owns
running tasks, emits `EngineEvent`s on a broadcast channel (record updates
plus a 500 ms batched progress event with per-segment data), and implements
the commands used by the UI and the browser bridge (add, start, pause,
cancel, restart, remove, rename, move, verify checksum, refresh address,
per-download and global speed limits).

## Desktop app (apps/desktop)

* `src-tauri/src/lib.rs` opens the database, loads the master key from the OS
  keyring, opens the `DownloadManager`, registers the media runner, resumes
  interrupted downloads, starts the browser bridge and the tray. A second
  launch (single-instance plugin) forwards its URL arguments; `--background`
  starts hidden in the tray (used by autostart and the native host).
* Commands (`commands/*.rs`, `integration/commands.rs`, `media_bridge.rs`) are
  thin wrappers over the manager; errors cross the boundary as
  `{ code, message }`. Engine events are forwarded as `engine://event`; a
  500 ms progress event carries per-segment data for the progress window.
* The React UI (Vite, Tailwind, Radix-based components, Zustand stores,
  i18next with English/Arabic and RTL) never computes progress itself: every
  number shown comes from engine events. Extra windows use hash routes:
  `#/progress/<id>` (live connections, segment map, speed graph) and
  `#/capture/<id>` (download offered by the browser).

## Browser integration

```
extension (MV3) ⇄ stdio framing ⇄ velox-nmh ⇄ authenticated IPC ⇄ desktop app
```

* `velox-nmh` is launched by the browser. It checks the caller's origin
  against the allow-list, validates every message (`ExtMessage::validate`:
  schema with `deny_unknown_fields`, URL scheme allow-list, size limits,
  CR/LF checks), and forwards it to the app. If the app is not running it is
  started with `--background` (detached from the browser's job object on
  Windows).
* The app's endpoint is a named pipe `\\.\pipe\velox-dm-<random>` (remote
  clients rejected, server PID verified by the client) or a Unix socket in the
  data directory (mode 0600). `nm-endpoint.json` holds the name and a random
  token that the host must present in its handshake.
* The extension captures downloads (`downloads.onDeterminingFilename` on
  Chromium; pause → hand off → cancel/erase on Firefox), offers context
  menus, detects media requests per tab (`webRequest`), and draws the floating
  "Download This Video" button in a closed shadow root over `<video>`
  elements. The button's quality menu is filled by the app's media probe.
* Quality options (`apps/desktop/src/shared/mediaOptions.ts`) are shared by
  the in-page menu and the app's Add Download dialog.

## Media engine (velox-media)

`MediaEngine` implements the manager's `MediaRunner` for HLS, DASH and
extractor (`Page`) downloads and answers probes.

* **Parsing.** `hls.rs` (master/media playlists, byte ranges, `EXT-X-MAP`,
  keys) and `dash.rs` (BaseURL resolution, SegmentTemplate with
  `$Number$`/`$Time$`/SegmentTimeline, SegmentList with byte ranges, single
  files). `SAMPLE-AES`, key formats other than `identity`, session keys and
  DASH `ContentProtection` mark the media as DRM-protected; such media is
  never downloaded. Live playlists are refused.
* **Planning.** A selection (`MediaRequest`) picks a variant/representation
  (explicit id, or the best within the preferred maximum height), the audio
  rendition (explicit or the variant's default) and subtitle tracks.
* **Segments.** `segments.rs` downloads up to `segment_concurrency` segments
  in parallel through the global and per-download rate limiters. Each segment
  is written to `<save_dir>/.velox-<id>/<track>/<n>.seg`, fsynced and renamed
  into place, so pause, crash or restart resumes by skipping finished
  segments. AES-128 segments are decrypted (key fetched once per URI, IV from
  the playlist or the media sequence number).
* **Muxing.** `mux.rs` runs FFmpeg with an argument vector: stream copy into
  MP4/MKV (audio re-encoded to AAC only if the container cannot hold the
  source codec), audio extraction to M4A/MP3, subtitles embedded (mov_text in
  MP4) or saved as `<name>.<lang>.vtt`.
* **Extractor.** `ytdlp.rs` runs yt-dlp for web pages (`-J` probe, download
  with a machine-readable progress template); browser cookies are passed in a
  temporary owner-only cookie file in the per-user data directory
  (`private/`), deleted afterwards and swept at startup after a crash.
* **Tools.** Release builds only use FFmpeg/ffprobe/yt-dlp shipped next to the
  executable (Tauri sidecars). Debug builds may also use `VELOX_FFMPEG`,
  `VELOX_FFPROBE`, `VELOX_YTDLP` or `PATH`; Settings → Media shows which copy
  is in use.

## Queues and scheduler (velox-scheduler)

* Every download belongs to a queue (`main` by default, started by
  default). Manual "Start" runs a download immediately; queue processing
  only starts downloads that wait (`Queued`) in a started queue, within the
  queue's and the global `max_concurrent`, highest priority first.
* `time.rs` turns a schedule (start/stop time, days of week) into
  edge-triggered start/stop events between two passes, so a queue the user
  stopped inside its window is not restarted, and a start missed while the
  computer slept still fires; at startup a queue inside its window starts.
* `plan.rs` is the pure start planner; `Scheduler` runs it on every engine
  event and once a second, owns queue CRUD, sends `requeue` to downloads of
  a stopped queue, and emits `QueueFinished` when a started batch has no
  waiting or running downloads left. The app then runs the queue's
  post-completion action (quit, sleep, hibernate, shut down) after a
  cancellable 60-second countdown.
* `power.rs` reads the battery (Linux sysfs, Windows
  `GetSystemPowerStatus`, macOS `pmset`) and, on Windows, the connection
  cost (WinRT); the power settings can hold queue processing back.

## FTP and SFTP (velox-ftp)

`Remote` opens a remote file at an offset for an optional length; the
engine's `Transport::Remote` maps that onto the same segmented task as
HTTP (one remote session per connection, `SIZE`/`MDTM` or SFTP attributes as
validators, `REST` for FTP offsets). FTPS uses rustls with the OS trust
store (`ftps://` implicit, `ftpes://` explicit TLS). SFTP host keys are
checked against `~/.ssh/known_hosts`, then the app's own known-hosts file
(trust on first use); a changed key is refused. Credentials written into an
address are moved into the encrypted secrets store.

## Packaging and release

* `scripts/prepare-release.mjs` downloads FFmpeg, ffprobe (BtbN LGPL builds)
  and yt-dlp from the URLs pinned in `resources/sidecars.lock.json`, verifies
  their SHA-256 (a mismatch stops the build), extracts them, builds the
  native host in release mode and builds the extensions. Everything lands in
  `apps/desktop/src-tauri/binaries/<name>-<target>` for Tauri's `externalBin`.
* `tauri.release.conf.json` (merged with `--config` for release builds only,
  so development builds do not need the binaries) adds the sidecars, the
  extension folders, FFmpeg's license and `THIRD_PARTY_NOTICES.md`. Sidecars
  are installed next to `velox-desktop`, which is where `Tools::discover`
  and the native-host registration look for them.
* Windows: NSIS installer with the WebView2 offline installer embedded
  (`webviewInstallMode: offlineInstaller`), per-user or per-machine install.
  `installer/nsis/hooks.nsh` runs `velox-nmh --register` after installing and
  `--unregister` (plus removal of the autostart entry) before uninstalling.
* Updates: the Tauri updater with signed artifacts; the public key is added
  by the release workflow from a repository variable, the private key comes
  from a secret (see `RELEASE_CHECKLIST.md`).
* CI (`.github/workflows/ci.yml`): fmt, clippy `-D warnings`, all Rust tests
  with media tests required, UI/extension checks, Windows tests with the
  pinned Windows tools, and the desktop + browser end-to-end tests.
  `release.yml` builds the Windows installer and Linux AppImage from a tag.
* `scripts/third-party-notices.mjs` generates `THIRD_PARTY_NOTICES.md` from
  `cargo metadata` and the UI's production npm packages, with license texts.

## Security

* TLS: rustls with the operating system trust store (platform verifier); no
  option to disable verification.
* Secrets (cookies, credentials, `Authorization` headers, proxy password) are
  sealed with ChaCha20-Poly1305 bound to the record id. The 256-bit key is kept
  in the OS credential store (Windows Credential Manager / macOS Keychain /
  Secret Service). Without a keyring, secrets stay in memory only.
* A login in an address (`user:pass@host`) is moved into those secrets when a
  download is added or its address is changed, including media manifest
  addresses; media downloads send it only to the manifest's own origin.
* File names from servers, URLs and browsers are sanitized (no path
  components, no control or bidi-override characters, no reserved device
  names, length limited).
* Native messaging: the host validates the calling extension origin against
  an allow-list, enforces message size limits and a strict schema
  (`deny_unknown_fields`, URL scheme allow-list, header injection checks) and
  authenticates to the app's IPC endpoint with a per-user token.
* Bundled tools are executed by absolute path with argument vectors (never a
  shell), and in release builds only from the application's own directory.
  FFmpeg inputs are restricted to local files with a forced demuxer, so
  downloaded data cannot be interpreted as a playlist that references other
  files; yt-dlp runs without user configuration or plugin directories.
* Window capabilities: only the main window may read the clipboard, use the
  updater or change autostart; progress and capture windows get window
  management and the folder picker.
