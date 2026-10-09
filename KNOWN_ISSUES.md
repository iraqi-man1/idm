# Known Issues and Limitations

This file lists what does not work, works partially, or cannot work because
of platform restrictions. Keep it honest and current.

## Platform and policy limitations (by design)

* **No DRM circumvention.** Encrypted media (Widevine, PlayReady, FairPlay,
  HLS `SAMPLE-AES` with key systems, DASH with `ContentProtection`) is detected
  and refused.
* **Browser extensions cannot be installed silently.** Chrome, Edge and Firefox
  require user consent and (for normal users) store distribution. The app
  opens the store page or shows developer-mode instructions.
* **Browser download capture** relies on `chrome.downloads` / `browser.downloads`.
  Downloads that never reach the downloads API (some `blob:` URLs, downloads
  triggered by page scripts with POST bodies) cannot be transferred and are
  left to the browser.
* **Redirects to another host** keep custom headers. Authorization, Cookie
  and Proxy-Authorization are removed when a redirect leaves the original
  host, but other request headers, including `X-API-Key` and `X-Auth-Token`
  (which Velox stores encrypted), are sent to the new host as well, as
  browsers and curl do.
* **HTTP/3** is not enabled: reqwest's HTTP/3 support is still behind an
  unstable cfg flag. HTTP/1.1 and HTTP/2 are supported.
* **Alt+click bypass** relies on the browser turning Alt+click into a download.
  Chrome/Edge on Windows do this; Chromium on Linux does not, so on Linux use
  the popup's "Take over downloads" switch to let the browser handle a file.
* **Incognito/private windows**: downloads there are deliberately not taken
  over (their cookies must not leave the private session).
* **Extension store IDs**: the extension is not yet published. Until it is,
  Chromium browsers use the unpacked build shipped with the app (stable ID
  `encnclpojnlecaheiiibdkkgiapnhocl` from the public key in the manifest) and
  Firefox needs a temporary add-on or an AMO-signed build. Store IDs must be
  added to `CHROMIUM_STORE_EXTENSION_IDS` (crates/native-messaging) when known.

## Media

* **Live streams** (HLS without `EXT-X-ENDLIST`, dynamic DASH) are refused:
  they cannot be downloaded to completion.
* **DASH subtitle tracks** are listed by the probe but not downloaded yet
  (HLS WebVTT subtitles are supported).
* **Segment downloads use one request per segment**, several segments in
  parallel (Settings → Media → Parallel stream segments). The progress
  window's per-connection view therefore shows a single stream connection for
  media downloads; segment-level progress is shown as "Segments n/m".
* **Pausing a media download** keeps every finished segment; segments that
  were in flight are fetched again on resume (each is small).
* **The in-page quality menu** offers the best audio track with each video
  quality; choosing another audio language is possible in the app's Add
  Download dialog.
* **Site extractors** come from the yt-dlp version bundled with the app. Sites
  change often; extractor fixes arrive with app updates. Velox never
  downloads or replaces tool binaries on its own.
* **Bundled tools**: development builds use FFmpeg/yt-dlp from `PATH` or
  `VELOX_FFMPEG`/`VELOX_YTDLP`. Release builds use only the copies shipped
  with the installer.
* **yt-dlp cookies**: yt-dlp reads cookies from a file. While it runs, the
  cookies the browser sent for that site are in a temporary file in Velox's
  per-user data folder (owner-only on Linux and macOS; on Windows the folder
  lies in the user's profile). The file is deleted when yt-dlp finishes, and
  files left by a crash are deleted at the next start.
* **Logins for web pages**: a user name and password in the address of a page
  handled by yt-dlp are stored encrypted but not passed to yt-dlp; such pages
  need the browser's cookies instead. (HLS/DASH manifests do use them, for
  the manifest's own server only.)

## Queues and power

* **Post-completion actions** run real OS commands (`shutdown.exe`,
  `SetSuspendState`, `systemctl`, `pmset`/AppleScript). They are not executed
  in automated tests; the 60-second countdown and its cancellation are.
* **Sleep on Windows** uses `SetSuspendState`; when hibernation is enabled
  in Windows, it may hibernate instead of sleeping.
* **Hibernate on macOS** is not supported.
* **Metered connections** are detected on Windows only.

## FTP and SFTP

* FTP uses passive mode only (active mode is not supported).
* `ftps://` means implicit TLS (port 990); use `ftpes://` for servers that
  require explicit TLS (`AUTH TLS`).
* SFTP logs in with a password or an unencrypted key in `~/.ssh`
  (`id_ed25519`, `id_ecdsa`, `id_rsa`); passphrase-protected keys and SSH
  agents are not supported yet.
* A host not listed in `~/.ssh/known_hosts` is trusted on first use and its
  key recorded in Velox's own known-hosts file; later key changes are
  refused.

## Packaging

* **Linux .deb**: Tauri installs bundled programs next to the executable in
  `/usr/bin`, where `ffmpeg`, `ffprobe` and `yt-dlp` would collide with the
  distribution's packages. Linux releases are therefore AppImages; a .deb
  needs the tools moved to a private directory first.
* **macOS**: no FFmpeg build is pinned yet, and the app is not signed or
  notarized, so there is no macOS release.
* **Windows code signing**: the installer is not Authenticode-signed until a
  certificate is configured; SmartScreen warns on first run.
* **Installer size**: the static LGPL FFmpeg and ffprobe builds are about
  130 MB each, and the WebView2 offline installer adds about 130 MB.
* **yt-dlp and JavaScript**: some sites (notably YouTube) need a JavaScript
  runtime for yt-dlp's extractors; Velox does not bundle one, so those sites
  may fail while generic pages, HLS and DASH work.
