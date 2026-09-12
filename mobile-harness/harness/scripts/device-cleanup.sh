#!/usr/bin/env bash
# Give the screen back and post the outcome as a phone notification. Safe to run after a failed step.
set -uo pipefail

OUT="${1:-results}"
MSG="finished"
if [[ -f "$OUT/summary.json" ]]; then
  MSG=$(python3 - "$OUT/summary.json" <<'PY' 2>/dev/null || echo "finished"
import json, sys
s = json.load(open(sys.argv[1]))
print(f"{s['passed']} passed, {s['failed'] + s['errors']} failed, {s['skipped']} skipped")
PY
)
fi

adb shell svc power stayon false >/dev/null 2>&1 || true
adb shell "cmd notification post -S bigtext -t 'Device tests' harness 'Done: $MSG. Your screen is yours again.'" >/dev/null 2>&1 || true
echo "[cleanup] $MSG"
