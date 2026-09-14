#!/usr/bin/env bash
# Launch the built app, wait for the UI to report ready, then kill it.
# Proves the webview starts and the UI can reach the core on this platform.
#
# Usage: scripts/smoke-launch.sh path/to/tinkerpad [timeout-seconds]
# On Linux without a display, run it under xvfb-run.
set -uo pipefail

exe="$1"
timeout="${2:-60}"
log="$(mktemp)"

"$exe" >"$log" 2>&1 &
pid=$!

for _ in $(seq 1 $((timeout * 10))); do
  if grep -q "ui ready" "$log"; then
    grep "ui ready" "$log"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    exit 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "app exited before the UI was ready:"
    cat "$log"
    exit 1
  fi
  sleep 0.1
done

echo "timed out after ${timeout}s waiting for the UI; output:"
cat "$log"
kill "$pid" 2>/dev/null
exit 1
