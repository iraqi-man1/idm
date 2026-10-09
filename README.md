# Velox Download Manager

A fast, native desktop download manager with IDM-class segmented
downloading, browser integration and a media downloader — built with
Tauri 2, Rust and React.

> Velox is an independent implementation. It is not affiliated with, and does
> not use code or assets from, Internet Download Manager.

## Highlights

* **Segmented downloads** with up to 32 connections, dynamic splitting and
  adaptive connection management that respects server limits.
* **Crash-safe resume**: progress is checkpointed only after data is
  fsynced; downloads survive app restarts, crashes and power loss.
* **Safe resume**: `If-Range` validation detects changed remote files; expired
  links can be refreshed without losing progress.
* **Browser integration** for Chrome, Edge, Brave, Opera and Firefox via
  Native Messaging, including a floating **Download This Video** button.
* **Media downloads** (HLS, DASH, extractor-supported sites) using bundled
  FFmpeg and yt-dlp — no separate installs. DRM-protected media is not
  supported.
* **FTP, FTPS and SFTP** with the same segmented, resumable engine.
* Queues with a scheduler and post-completion actions, speed limits,
  clipboard monitoring, statistics, English and Arabic (RTL) UI, light/dark
  themes.

See [FEATURES.md](FEATURES.md) for the exact, tested status of every feature
and [KNOWN_ISSUES.md](KNOWN_ISSUES.md) for limitations.

## For users

No release has been published yet. Installers are built by the `Release`
workflow from a version tag (see [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md));
the Windows installer has not yet passed the clean-machine test in
[docs/testing/clean-machine-test.md](docs/testing/clean-machine-test.md).

Once released: download the Windows installer from the Releases page and run
it. Everything the app needs (WebView2 offline installer, FFmpeg, ffprobe,
yt-dlp and the native messaging host) is inside the installer; no Python,
Node.js or other tools are required. Then open **Settings → Browser** to add
the browser extension (browsers require you to confirm extension installs;
the app cannot install it silently).

## For developers

Requirements: Rust (stable, ≥ 1.90), Node.js 22, pnpm 10, and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
pnpm install                     # JS dependencies (desktop UI + extensions)
cargo test --workspace           # engine, media, persistence, protocol tests
pnpm --filter desktop tauri dev  # run the desktop app
pnpm --filter extensions build   # build browser extensions into extensions/*/dist
cargo run -p velox-test-server -- --port 8787 [--static DIR]   # local test server
```

Media tests generate real HLS/DASH streams with FFmpeg, so `ffmpeg` and
`ffprobe` must be on `PATH` (they are skipped otherwise; set
`VELOX_REQUIRE_MEDIA_TESTS=1` to make that an error). Debug builds of the app
also find FFmpeg and yt-dlp on `PATH` or through `VELOX_FFMPEG` /
`VELOX_YTDLP`; release builds only use the copies bundled with the app.
End-to-end tests of the real app and browser are in `tests/e2e/` (see
`DEVELOPMENT_PROGRESS.md`).

Documentation:

* [ARCHITECTURE.md](ARCHITECTURE.md) — design and module boundaries
* [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md) — phases
* [DEVELOPMENT_PROGRESS.md](DEVELOPMENT_PROGRESS.md) — current status
* [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) — release gates

## License

MIT OR Apache-2.0 for Velox's own code. Bundled third-party tools keep their
own licenses; see `THIRD_PARTY_NOTICES.md`.
