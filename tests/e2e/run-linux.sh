#!/usr/bin/env bash
# Run the desktop E2E smoke test on Linux under Xvfb with an isolated HOME.
# Usage: tests/e2e/run-linux.sh [python] [out-dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PY="${1:-python3}"
OUT="${2:-$ROOT/target/e2e}"
APP="$ROOT/target/debug/velox-desktop"
WORK="$(mktemp -d)"
export HOME="$WORK/home"
mkdir -p "$HOME/Downloads" "$OUT"
export XDG_DATA_HOME="$HOME/.local/share" XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache"
export NO_AT_BRIDGE=1 WEBKIT_DISABLE_COMPOSITING_MODE=1

cleanup() {
  kill ${PIDS:-} 2>/dev/null || true
  cp -r "$XDG_DATA_HOME/com.veloxdm.app/logs" "$OUT/app-logs" 2>/dev/null || true
  rm -rf "$WORK" 2>/dev/null || true
}
trap cleanup EXIT
PIDS=""

Xvfb :99 -screen 0 1440x900x24 >/dev/null 2>&1 & PIDS="$PIDS $!"
export DISPLAY=:99
eval "$(dbus-launch --sh-syntax)"
PIDS="$PIDS $DBUS_SESSION_BUS_PID"

"$ROOT/target/debug/velox-test-server" --port 8787 >"$OUT/test-server.log" 2>&1 & PIDS="$PIDS $!"
tauri-driver --port 4444 --native-port 4443 >"$OUT/tauri-driver.log" 2>&1 & PIDS="$PIDS $!"
sleep 2

"$PY" "$ROOT/tests/e2e/smoke_test.py" --app "$APP" --server http://127.0.0.1:8787 \
  --out "$OUT" --downloads "$HOME/Downloads"
