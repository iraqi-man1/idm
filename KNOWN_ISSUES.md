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
* **Bundled tools**: until the Phase 7 packaging work lands, development
  builds use FFmpeg/yt-dlp from `PATH` or `VELOX_FFMPEG`/`VELOX_YTDLP`.
  Release builds use only the copies shipped with the installer.
