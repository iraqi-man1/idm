# Release Checklist

Mark an item only after actually performing it. Record results in
`docs/testing/` with date, version and environment.

## One-time setup

* **Updater signing key.** Run `pnpm --filter desktop tauri signer generate -w velox-updater.key`
  on a trusted machine. Store the private key and its password as the
  repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and the public key as the repository
  variable `VELOX_UPDATER_PUBKEY`. Never commit the private key. Without these,
  releases build normally but contain no update artifacts and the app's
  update check stays disabled.
* **Code signing (Windows).** An Authenticode certificate is not part of the
  repository. Configure `bundle.windows.certificateThumbprint` / a signing
  command in CI once a certificate is available; until then SmartScreen warns
  on first run.
* **GitHub Actions.** The workflows use major-version action tags; pin them to
  commit SHAs for supply-chain hardening.

## Pinned third-party tools

`resources/sidecars.lock.json` pins the URL and SHA-256 of FFmpeg, ffprobe
(BtbN LGPL builds) and yt-dlp per target. `scripts/prepare-release.mjs`
refuses to bundle a file whose hash differs.

* FFmpeg is pinned to the last BtbN build of a month: BtbN keeps those for
  two years (daily builds for 14 days; its `latest` URL changes every day
  and must not be pinned). To move to a newer build, run the **Refresh
  pinned tools** workflow (Actions → Run workflow) or
  `node scripts/prepare-release.mjs update-lock` with network access to the
  GitHub API. It pins the last build of the most recent completed month,
  refreshes every hash and verifies the downloads; review the diff (and the
  upstream changes), regenerate `THIRD_PARTY_NOTICES.md` (it names the
  pinned build; CI fails while it is stale) and commit both files. Refresh
  at least once a year.
* To move to a new yt-dlp version or FFmpeg release branch, change the
  version, URLs and asset names in the lock file, then run `update-lock`.
* macOS has no pinned FFmpeg source yet (the lock entries are `null`); a
  macOS release needs a verified FFmpeg/ffprobe build added first.

## Build

- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace` (with `VELOX_REQUIRE_MEDIA_TESTS=1`) passes on Linux and Windows (CI)
- [ ] UI and extension typecheck, unit tests and builds pass (CI `frontend` job)
- [ ] Desktop and browser end-to-end tests pass (CI `e2e-linux` job)
- [ ] `node scripts/third-party-notices.mjs` run and `THIRD_PARTY_NOTICES.md` committed
- [ ] Release candidate built with a manual `Release` run (Actions → Release →
  Run workflow; installers attached to the run, no release created) and used
  for the clean-machine gate below
- [ ] Tag `vX.Y.Z` pushed; the `Release` workflow built:
  - [ ] Windows x64 NSIS installer (sidecars verified, WebView2 offline installer embedded)
  - [ ] Linux x64 AppImage
- [ ] Update artifacts signed and `latest.json` attached (when the key is configured)
- [ ] Installer and binaries Authenticode-signed (requires a certificate)
- [ ] Chromium and Firefox extension zips built (`pnpm --filter extensions build`)

## Clean-machine gate (Windows 10 and Windows 11 VMs without dev tools)

Follow `docs/testing/clean-machine-test.md`. All must pass; do not mark the
gate passed without executing every step on real VMs.

- [ ] 1. Installer launches (SmartScreen behaviour noted)
- [ ] 2. Application installs (per-user and per-machine)
- [ ] 3. Application opens
- [ ] 4. WebView2, FFmpeg, ffprobe, yt-dlp present / provisioned without internet
- [ ] 5. HTTP download works
- [ ] 6. Segmented download shows multiple active connections
- [ ] 7. Pause and resume continue from the same offset
- [ ] 8. Browser integration setup screen detects browsers and opens the extension page
- [ ] 9. Native messaging: extension shows "Connected", captured download appears in the app
- [ ] 10. A supported (non-DRM) video downloads and plays
- [ ] 11. Uninstall removes app, native-host registry keys and shortcuts; downloads are preserved
- [ ] 12. No command-line installation step was needed

## Store submissions

- [ ] Chrome Web Store listing (extension ID added to `CHROMIUM_STORE_EXTENSION_IDS`)
- [ ] Microsoft Edge Add-ons listing
- [ ] addons.mozilla.org signed release
