# Development Plan

The project is built in phases. Each phase must compile, pass its tests and
leave the repository in a working state. Status lives in
`DEVELOPMENT_PROGRESS.md`.

| Phase | Scope |
|-------|-------|
| 1 | Architecture, monorepo, shared types, SQLite persistence, context docs |
| 2 | HTTP(S) engine: probe, dynamic segmentation, writer, checkpoints, resume, retries, rate limiting, test server, integration tests |
| 3 | Tauri 2 shell, typed commands/events, React UI (sidebar, table, progress window, add dialog, settings), themes, EN/AR with RTL, tray |
| 4 | Native messaging host + IPC, Chromium/Firefox MV3 extensions: download capture, context menus, popup, options, exclusions, browser setup screen |
| 5 | Media: HLS/DASH parsing and segmented download, FFmpeg remux, yt-dlp extractor, video detection, floating "Download This Video" button |
| 6 | Queues and scheduler, post-completion actions, statistics, clipboard monitor, batch import, FTP/SFTP, battery/metered handling |
| 7 | NSIS installer hooks (native host registration), sidecar fetching with pinned checksums, updater, security review, CI for Windows/macOS/Linux |
| 8 | Clean-machine test execution (Windows VM), release packaging, docs, third-party notices |

## Working agreements

* No simulated data anywhere: progress, speeds and segment views come from
  the engine; buttons call real commands.
* A feature is "done" only with tests (or a documented manual test when it
  needs a browser or OS UI).
* Browser/OS/DRM limitations are documented in `KNOWN_ISSUES.md`, never
  hidden.
* Update `DEVELOPMENT_PROGRESS.md`, `FEATURES.md` and `KNOWN_ISSUES.md` at the
  end of each milestone.
