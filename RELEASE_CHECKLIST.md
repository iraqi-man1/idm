# Release Checklist

Mark an item only after actually performing it. Record results in
`docs/testing/` with date, version and environment.

## Build

- [ ] `cargo test --workspace` passes
- [ ] `pnpm -r test` and `pnpm -r typecheck` pass
- [ ] `cargo clippy --workspace --all-targets` has no warnings
- [ ] Sidecars fetched by `scripts/fetch-sidecars.mjs`, SHA-256 verified against `resources/sidecars.lock.json`
- [ ] Windows x64 NSIS installer built by CI (`.github/workflows/release.yml`)
- [ ] Installer and binaries Authenticode-signed (requires a code-signing certificate — not available in the repository)
- [ ] Update artifacts signed with the Tauri updater key; `latest.json` published
- [ ] Chromium and Firefox extension zips built (`pnpm --filter extensions build`)

## Clean-machine gate (Windows 10 and Windows 11 VMs without dev tools)

Follow `docs/testing/clean-machine-test.md`. All must pass:

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

- [ ] Chrome Web Store listing (extension ID added to `allowed_origins`)
- [ ] Microsoft Edge Add-ons listing
- [ ] addons.mozilla.org signed release
