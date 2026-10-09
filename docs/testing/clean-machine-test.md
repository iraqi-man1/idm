# Clean-machine test (release gate)

This gate proves that the Windows installer works on a computer with no
development tools. **It has not been executed yet.** Do not mark it passed
in `RELEASE_CHECKLIST.md` until every step below was performed on real
virtual machines and the results were recorded in this file.

## Machines

Run the whole procedure twice:

| VM | Image | Notes |
|---|---|---|
| A | Windows 10 22H2 x64, fresh install, all updates | standard user account + one administrator |
| B | Windows 11 24H2 x64, fresh install, all updates | standard user account + one administrator |

Requirements for both:

* No Rust, Node.js, Python, Git, FFmpeg, yt-dlp, Visual Studio or other
  developer tools; `where python`, `where node`, `where ffmpeg` find nothing
  (the Microsoft Store Python alias may exist; it must not be used).
* Microsoft Edge present as shipped; Google Chrome and Mozilla Firefox
  installed from their official installers.
* Take a snapshot before step 1 so each run starts clean.
* For the "without internet" part, disconnect the VM's network adapter.

## Procedure

Use the installer produced by the `Release` workflow
(`Velox Download Manager_<version>_x64-setup.exe`): from a draft release
for a version tag, or from a manual run (Actions → Release → Run workflow),
which attaches it as the `installer-x86_64-pc-windows-msvc` artifact
together with `SHA256SUMS.txt`. Record the installer's SHA-256 and take
screenshots for each numbered step.

1. **Installer launches.** Copy the installer to the VM, disconnect the
   network, double-click it. Note SmartScreen's behaviour (expected:
   "Windows protected your PC" until the installer is code-signed).
2. **Installs.** Install once per user (default) and, after reverting the
   snapshot, once for all users (administrator). Both complete without
   errors, without internet, and without opening a console.
3. **Opens.** Start Velox from the Start menu. The main window appears in
   the system language (English or Arabic, RTL for Arabic).
4. **Bundled components.** Still offline, run
   `powershell -ExecutionPolicy Bypass -File check-install.ps1`
   (from `tests/clean-machine/`). All checks pass: application files,
   `ffmpeg.exe`, `ffprobe.exe`, `yt-dlp.exe` and `velox-nmh.exe` present and
   runnable with a minimal PATH, WebView2 runtime installed, native
   messaging host registered for Chrome, Edge and Firefox. In Velox,
   Settings → Media lists all three tools as *Bundled*.
5. **HTTP download.** Reconnect the network. Add a large public file (for
   example a Linux ISO over HTTPS). It completes; the file's checksum
   matches the published one (Velox's "Verify checksum").
6. **Segmented download.** During step 5 the Connections column and the
   progress window show several active connections and the segment map.
7. **Pause / resume.** Pause at about 30 %, close Velox from the tray, reopen
   it, resume. The download continues from the same offset (the progress
   does not restart) and the final checksum matches.
8. **Browser integration screen.** Settings → Browser integration detects
   the installed browsers, shows the host as registered, and "Install
   browser extension" opens the extension page / folder with instructions.
9. **Native messaging.** Install the extension in Chrome and Edge (and the
   Firefox build). Its popup shows *Connected*. Click a download link on a
   web page: Velox shows the download dialog, downloads the file, and the
   browser's own download is cancelled.
10. **Video.** Open a page with a non-DRM HTML5 video (an HLS test stream and
    a page with an MP4). The floating "Download This Video" button appears,
    the quality menu lists formats, the chosen quality downloads, and the
    resulting file plays in the Windows media player.
11. **Uninstall.** Uninstall from Settings → Apps. Run
    `check-uninstall.ps1 -DownloadsDir <folder used above>`: application
    files, native-messaging registry keys, autostart entry and shortcuts are
    removed; the downloaded files are still there.
12. **No command line.** Confirm no step required a command prompt or
    manual installation of anything (the two check scripts are evidence
    collection only).

## Results

| Step | VM A (Windows 10) | VM B (Windows 11) |
|---|---|---|
| 1 Installer launches | not run | not run |
| 2 Installs (per-user / per-machine) | not run | not run |
| 3 Opens | not run | not run |
| 4 Bundled components offline | not run | not run |
| 5 HTTP download | not run | not run |
| 6 Segmented download | not run | not run |
| 7 Pause / resume after restart | not run | not run |
| 8 Browser integration screen | not run | not run |
| 9 Native messaging | not run | not run |
| 10 Video download and playback | not run | not run |
| 11 Uninstall cleanup | not run | not run |
| 12 No command line needed | not run | not run |

Tester, date, installer version and SHA-256:

* VM A: —
* VM B: —
