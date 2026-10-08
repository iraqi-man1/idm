# Velox Download Manager — Architecture

Velox is a native desktop download manager (Tauri 2 + Rust + React) with an
IDM-class segmented download engine, browser integration through Native
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

## Security

* TLS: rustls with the operating system trust store (platform verifier); no
  option to disable verification.
* Secrets (cookies, credentials, `Authorization` headers, proxy password) are
  sealed with ChaCha20-Poly1305 bound to the record id. The 256-bit key is kept
  in the OS credential store (Windows Credential Manager / macOS Keychain /
  Secret Service). Without a keyring, secrets stay in memory only.
* File names from servers, URLs and browsers are sanitized (no path
  components, no control or bidi-override characters, no reserved device
  names, length limited).
* Native messaging: the host validates the calling extension origin against
  an allow-list, enforces message size limits and a strict schema
  (`deny_unknown_fields`, URL scheme allow-list, header injection checks) and
  authenticates to the app's IPC endpoint with a per-user token.
* Bundled tools are executed by absolute path with argument vectors (never a
  shell), and only from the application's own resource directory.
