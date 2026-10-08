# Features

Status legend: ✅ implemented and tested · 🟡 implemented, partially tested or
manual-test only · ⏳ not yet implemented · 🚫 not supported (see KNOWN_ISSUES)

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
| Adaptive connection count | 🟡 | ramp/plateau logic implemented; server-limit detection tested |
| Range support detection, single-connection fallback | ✅ | |
| Direct random-access writes, no merge | ✅ | sparse files on Windows |
| Pause / resume / cancel / restart / retry | ✅ | |
| Resume after app restart / crash / power loss | ✅ | fsync-before-commit checkpoints |
| ETag / Last-Modified validation (If-Range) | ✅ | |
| Expired link detection + address refresh | ✅ | |
| Network interruption recovery | ✅ | backoff retries |
| Global and per-download speed limits | ✅ | |
| Checksum verification (MD5/SHA-1/SHA-256/SHA-512) | ✅ | |
| FTP / SFTP | ⏳ | Phase 6 |

## Desktop app, browser integration, media, scheduler

⏳ Phases 3–7 (see DEVELOPMENT_PLAN.md).
