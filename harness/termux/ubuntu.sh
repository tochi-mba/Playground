#!/data/data/com.termux/files/usr/bin/bash
# Run a command inside the proot Ubuntu that hosts the runner, with this repo mounted at /mnt/harness.
# Examples:
#   harness/termux/ubuntu.sh adb pair 127.0.0.1:37123
#   harness/termux/ubuntu.sh /mnt/harness/harness/scripts/adb-connect.sh
set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
export PROOT_NO_SECCOMP="${PROOT_NO_SECCOMP:-1}"
exec proot-distro login ubuntu --shared-tmp --bind "$REPO_DIR:/mnt/harness" -- "$@"
