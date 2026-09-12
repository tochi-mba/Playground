#!/usr/bin/env bash
# Install the app + androidTest APKs and run the instrumented suite.
# Always leaves junit.xml, summary.json, logcat.txt, screenshot.png and instrument.raw in the
# results dir, and exits non-zero when any test failed or the run did not complete.
#
# usage: run-instrumented.sh <apk dir> <results dir> [extra am instrument args]
# env:   APP_ID  applicationId of the installed debug build (used to find its instrumentation)
set -uo pipefail

APK_DIR="${1:?apk dir}"
OUT="${2:?results dir}"
FILTER="${3:-}"
APP_ID="${APP_ID:?APP_ID (applicationId of the debug build) must be set}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "$OUT"

APP_APK=$(find "$APK_DIR" -name '*.apk' ! -name '*androidTest*' | head -1)
TEST_APK=$(find "$APK_DIR" -name '*androidTest*.apk' | head -1)
if [[ -z "$APP_APK" || -z "$TEST_APK" ]]; then
  echo "[run] app or androidTest APK not found under $APK_DIR:" >&2
  find "$APK_DIR" >&2
  exit 2
fi
echo "[run] app  APK: $APP_APK"
echo "[run] test APK: $TEST_APK"

set -e
adb install -r -t -g "$APP_APK"
adb install -r -t -g "$TEST_APK"
set +e

INSTR=$(adb shell pm list instrumentation | tr -d '\r' \
  | awk -v t="(target=$APP_ID)" 'index($0, t) { sub(/^instrumentation:/, "", $1); print $1; exit }')
if [[ -z "$INSTR" ]]; then
  echo "[run] no instrumentation targets $APP_ID. Installed instrumentations:" >&2
  adb shell pm list instrumentation >&2
  exit 2
fi
echo "[run] runner: $INSTR"
[[ -n "$FILTER" ]] && echo "[run] filter: $FILTER"

# $FILTER is intentionally unquoted: it is a list of `-e key value` args.
# shellcheck disable=SC2086
adb shell am instrument -w -r $FILTER "$INSTR" | tr -d '\r' | tee "$OUT/instrument.raw"

adb exec-out screencap -p > "$OUT/screenshot.png" 2>/dev/null || true
adb logcat -d > "$OUT/logcat.txt" 2>/dev/null || true

python3 "$HERE/instrument_to_junit.py" "$OUT/instrument.raw" "$OUT/junit.xml" "$OUT/summary.json"
