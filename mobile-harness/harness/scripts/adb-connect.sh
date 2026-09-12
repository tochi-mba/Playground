#!/usr/bin/env bash
# Connect adb to this phone's own Wireless debugging port over loopback.
# Runs inside the runner (proot Ubuntu) but is plain bash + adb, so it works anywhere.
#
# Order: existing connection -> saved port -> mDNS discovery -> port scan.
# The Wireless debugging port changes every time the toggle is flipped, so the saved
# port is only a hint. Exit 2 with instructions when nothing works.
set -euo pipefail

STATE_DIR="${HARNESS_STATE_DIR:-$HOME/.harness}"
PORT_FILE="$STATE_DIR/adb_port"
HOST="${ADB_HOST:-127.0.0.1}"
SCAN_FROM="${ADB_SCAN_FROM:-30000}"
SCAN_TO="${ADB_SCAN_TO:-49999}"
mkdir -p "$STATE_DIR"

log() { echo "[adb-connect] $*"; }

device_summary() {
  echo "$(adb shell getprop ro.product.model | tr -d '\r') / Android $(adb shell getprop ro.build.version.release | tr -d '\r')"
}

try_connect() {
  local port="$1" out
  out=$(timeout 15 adb connect "$HOST:$port" 2>&1 || true)
  if [[ "$out" == *"connected to"* ]]; then
    # "connected to" also appears in "failed to connect to"; confirm with state.
    timeout 10 adb -s "$HOST:$port" get-state >/dev/null 2>&1
  else
    return 1
  fi
}

discover_mdns() {
  ADB_MDNS_OPENSCREEN=1 timeout 10 adb mdns services 2>/dev/null \
    | awk '/_adb-tls-connect\._tcp/ { n = split($NF, a, ":"); print a[n]; exit }'
}

# Wireless debugging listens on a random high port. Loopback refusals are instant,
# so scanning the range takes well under a minute even in Termux.
scan_ports() {
  local p
  for ((p = SCAN_FROM; p <= SCAN_TO; p++)); do
    if (exec 3<>"/dev/tcp/$HOST/$p") 2>/dev/null; then
      exec 3>&- 2>/dev/null || true
      echo "$p"
    fi
  done
}

adb start-server >/dev/null 2>&1 || true

if adb get-state >/dev/null 2>&1; then
  log "already connected: $(device_summary)"
  exit 0
fi

PORT=""

if [[ -f "$PORT_FILE" ]]; then
  saved=$(tr -d '[:space:]' < "$PORT_FILE")
  if [[ -n "$saved" ]] && try_connect "$saved"; then
    PORT="$saved"
    log "connected on saved port $PORT"
  else
    log "saved port ${saved:-<empty>} did not answer"
  fi
fi

if [[ -z "$PORT" ]]; then
  found=$(discover_mdns || true)
  if [[ -n "$found" ]] && try_connect "$found"; then
    PORT="$found"
    log "connected via mDNS on port $PORT"
  fi
fi

if [[ -z "$PORT" ]]; then
  log "scanning $HOST:$SCAN_FROM-$SCAN_TO for the Wireless debugging port"
  while read -r candidate; do
    [[ -n "$candidate" ]] || continue
    if try_connect "$candidate"; then
      PORT="$candidate"
      log "connected via port scan on port $PORT"
      break
    fi
  done < <(scan_ports)
fi

if [[ -z "$PORT" ]]; then
  cat >&2 <<MSG
[adb-connect] Could not connect adb to this phone.

  1. Settings > Developer options > Wireless debugging must be ON. It turns itself off on every reboot.
  2. If you have never paired from this runner:
       adb pair $HOST:<pairing port>     (Wireless debugging > "Pair device with pairing code")
  3. To skip discovery next time, save the port shown on the Wireless debugging screen:
       echo <port> > $PORT_FILE
MSG
  exit 2
fi

echo "$PORT" > "$PORT_FILE"
adb wait-for-device
log "ready: $(device_summary)"
