#!/usr/bin/env bash
# Lists what a built installer contains and fails if anything the app needs
# at runtime is missing (bundled tools, native messaging host, extensions,
# notices, and on Windows the offline WebView2 installer).
#
# Usage: scripts/check-installer.sh <bundle dir> <target triple>
set -euo pipefail

bundle=$1
target=$2
listing=$(mktemp)
trap 'rm -rf "$listing" "${extract:-}"' EXIT

case "$target" in
  *-windows-*)
    installer=$(find "$bundle/nsis" -name '*-setup.exe' | head -1)
    [ -n "$installer" ] || { echo "no NSIS installer in $bundle/nsis" >&2; exit 1; }
    7z l -ba "$installer" | tr '\\' '/' >"$listing"
    required=(velox-desktop.exe velox-nmh.exe ffmpeg.exe ffprobe.exe yt-dlp.exe
      extensions/chromium/manifest.json extensions/firefox/manifest.json
      THIRD_PARTY_NOTICES.md licenses/ WebView2)
    ;;
  *-linux-*)
    installer=$(find "$bundle/appimage" -name '*.AppImage' | head -1)
    [ -n "$installer" ] || { echo "no AppImage in $bundle/appimage" >&2; exit 1; }
    installer=$(realpath "$installer")
    extract=$(mktemp -d)
    (cd "$extract" && "$installer" --appimage-extract >/dev/null)
    (cd "$extract/squashfs-root" && find . -type f) >"$listing"
    required=(usr/bin/velox-desktop usr/bin/velox-nmh usr/bin/ffmpeg usr/bin/ffprobe usr/bin/yt-dlp
      extensions/chromium/manifest.json extensions/firefox/manifest.json THIRD_PARTY_NOTICES.md)
    ;;
  *) echo "unsupported target $target" >&2; exit 1 ;;
esac

echo "== $(basename "$installer"): $(wc -l <"$listing") entries"
grep -iE 'velox|ffmpeg|ffprobe|yt-dlp|webview2|manifest\.json|NOTICES|licenses/' "$listing" | head -60
missing=0
for f in "${required[@]}"; do
  if ! grep -qiF -- "$f" "$listing"; then
    echo "MISSING: $f" >&2
    missing=1
  fi
done
[ "$missing" = 0 ] && echo "all required files present"
exit "$missing"
