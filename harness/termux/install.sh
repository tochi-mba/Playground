#!/data/data/com.termux/files/usr/bin/bash
# One-time setup of this phone as a GitHub Actions runner. Run inside Termux (F-Droid build):
#
#   RUNNER_TOKEN=<registration token> ~/playground/harness/termux/install.sh
#
# Get the token from https://github.com/tochi-mba/playground/settings/actions/runners/new
# (it expires after an hour). Re-running is safe; it skips what is already done.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$HERE/../.." && pwd)"
REPO_URL="${REPO_URL:-https://github.com/tochi-mba/playground}"
RUNNER_NAME="${RUNNER_NAME:-$(getprop ro.product.model 2>/dev/null | tr -c 'A-Za-z0-9\n' '-' | sed 's/-*$//')}"
RUNNER_NAME="${RUNNER_NAME:-phone}"
: "${RUNNER_TOKEN:?Set RUNNER_TOKEN to a registration token from $REPO_URL/settings/actions/runners/new}"

echo "==> Termux packages"
pkg update -y >/dev/null
pkg install -y proot-distro termux-api git >/dev/null

echo "==> Ubuntu rootfs"
if [[ ! -d "$PREFIX/var/lib/proot-distro/installed-rootfs/ubuntu" ]]; then
  proot-distro install ubuntu
fi

echo "==> Runner inside Ubuntu (this downloads ~150 MB)"
"$HERE/ubuntu.sh" bash /mnt/harness/harness/termux/ubuntu-setup.sh "$REPO_URL" "$RUNNER_TOKEN" "$RUNNER_NAME" "${RUNNER_VERSION:-}"

echo "==> Boot hook and home-screen widget"
mkdir -p ~/.termux/boot ~/.shortcuts ~/.harness
sed "s|__REPO_DIR__|$REPO_DIR|g" "$HERE/boot/start-harness" > ~/.termux/boot/start-harness
sed "s|__REPO_DIR__|$REPO_DIR|g" "$HERE/widget/Start Harness" > ~/.shortcuts/"Start Harness"
chmod +x ~/.termux/boot/start-harness ~/.shortcuts/"Start Harness"

cat <<MSG

Installed. Next:
  1. Settings > Developer options > Wireless debugging > "Pair device with pairing code", then:
       $REPO_DIR/harness/termux/ubuntu.sh adb pair 127.0.0.1:<pairing port>
  2. Start the runner:  $REPO_DIR/harness/termux/start-runner.sh   (or tap the "Start Harness" widget)
  3. Check it shows Online at $REPO_URL/settings/actions/runners
MSG
