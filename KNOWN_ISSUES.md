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
