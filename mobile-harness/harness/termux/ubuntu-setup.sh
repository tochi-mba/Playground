#!/usr/bin/env bash
# Runs INSIDE the proot Ubuntu (called by install.sh). Installs adb + the GitHub Actions runner.
# args: <repo url> <runner registration token> <runner name> [runner version]
set -euo pipefail
REPO_URL="$1"; RUNNER_TOKEN="$2"; RUNNER_NAME="$3"; RUNNER_VERSION="${4:-}"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl git jq python3 adb >/dev/null

adb_version=$(adb --version 2>/dev/null | sed -n 's/.*version \([0-9]*\).*/\1/p' | head -1)
if [[ -z "$adb_version" || "$adb_version" -lt 30 ]]; then
  echo "WARNING: adb ${adb_version:-?} is older than 30 and cannot 'adb pair'. Use a newer Ubuntu image (24.04+)." >&2
fi

mkdir -p /root/actions-runner /root/.harness
cd /root/actions-runner

if [[ -z "$RUNNER_VERSION" ]]; then
  RUNNER_VERSION=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest | jq -r '.tag_name' | sed 's/^v//')
fi
if [[ ! -x run.sh ]]; then
  echo "downloading actions-runner ${RUNNER_VERSION} (linux-arm64)"
  curl -fL -o runner.tar.gz "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-arm64-${RUNNER_VERSION}.tar.gz"
  tar xzf runner.tar.gz && rm runner.tar.gz
  ./bin/installdependencies.sh >/dev/null || echo "installdependencies.sh failed; continuing" >&2
fi

export RUNNER_ALLOW_RUNASROOT=1 DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1
if [[ -f .runner ]]; then
  echo "runner already configured as $(jq -r .agentName .runner); skipping config.sh"
else
  ./config.sh --unattended --replace \
    --url "$REPO_URL" --token "$RUNNER_TOKEN" \
    --name "$RUNNER_NAME" --labels android-phone --work _work
fi
echo "ubuntu-setup: done"
