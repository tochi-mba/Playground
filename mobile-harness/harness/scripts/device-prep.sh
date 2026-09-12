#!/usr/bin/env bash
# Wake and unlock the screen, keep it on for the run, and warn the person holding the phone.
set -euo pipefail

LABEL="${1:-device tests}"
LABEL="${LABEL//\'/}"   # keep the shell quoting below simple

adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard || true
adb shell svc power stayon true
adb logcat -c || true
adb shell "cmd notification post -S bigtext -t 'Device tests' harness 'Starting: $LABEL. The screen is busy until the Done notification.'" >/dev/null 2>&1 || true

echo "[device-prep] screen awake and pinned on: $LABEL"
