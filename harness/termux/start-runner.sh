#!/data/data/com.termux/files/usr/bin/bash
# Start (and keep restarting) the GitHub Actions runner. Safe to run from the widget, Termux:Boot,
# or a terminal. Pulls the latest harness scripts first so the phone never needs a manual update.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$HERE/../.." && pwd)"
LOG_DIR="$HOME/.harness"
mkdir -p "$LOG_DIR"
exec > >(tee -a "$LOG_DIR/runner.log") 2>&1

notify() {
  termux-notification --id harness-runner --title "Harness runner" --content "$1" >/dev/null 2>&1 || true
}

termux-wake-lock >/dev/null 2>&1 || true
git -C "$REPO_DIR" pull --ff-only -q 2>/dev/null && echo "harness scripts up to date" || echo "git pull skipped"

# Attach adb now if we can, so the first job does not have to discover the port.
if "$HERE/ubuntu.sh" /mnt/harness/harness/scripts/adb-connect.sh; then
  echo "adb attached"
elif command -v termux-dialog >/dev/null 2>&1; then
  port=$(termux-dialog text -n -t "Wireless debugging port" -i "Turn Wireless debugging ON, then type the port it shows" 2>/dev/null \
    | sed -n 's/.*"text": *"\([0-9]*\)".*/\1/p')
  if [[ -n "$port" ]]; then
    "$HERE/ubuntu.sh" bash -c "mkdir -p /root/.harness && echo $port > /root/.harness/adb_port"
    "$HERE/ubuntu.sh" /mnt/harness/harness/scripts/adb-connect.sh || echo "adb still not attached; jobs will retry"
  fi
else
  echo "adb not attached; jobs will retry discovery when they start"
fi

notify "Runner online. Tap to open Termux."
trap 'echo "stopping runner"; notify "Runner stopped."; termux-wake-unlock >/dev/null 2>&1; exit 0' INT TERM

while true; do
  "$HERE/ubuntu.sh" bash -lc 'export RUNNER_ALLOW_RUNASROOT=1 DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1; cd /root/actions-runner && ./run.sh'
  rc=$?
  echo "runner exited with $rc; restarting in 10s (Ctrl-C to stop)"
  sleep 10
done
