#!/usr/bin/env bash
# Launch the built app, wait for the UI to report ready, then kill it.
# Proves the webview starts and the UI can reach the core on this platform.
#
# If TINKERPAD_SELFTEST is set (for example to "layout"), also waits for the
# in-app self-test to finish and fails if any check failed. See
# ui/src/selftest.ts.
#
# Usage: scripts/smoke-launch.sh path/to/tinkerpad [timeout-seconds]
# On Linux without a display, run it under xvfb-run.
# Intended for macOS and Linux CI. Under Git Bash on Windows, `kill` does not
# reliably stop the native process; use taskkill there instead.
set -uo pipefail

exe="$1"
timeout="${2:-60}"
log="$(mktemp)"
want="ui ready"
if [ -n "${TINKERPAD_SELFTEST:-}" ]; then
  want="selftest: done"
fi

"$exe" >"$log" 2>&1 &
pid=$!

# Say how this script ended, so a failing CI step shows whether it was the
# script or whatever wraps it (xvfb-run) that returned non-zero. No line at
# all means the script was killed.
trap 'echo "smoke-launch: exit $?"' EXIT

finish() {
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

for _ in $(seq 1 $((timeout * 10))); do
  if grep -q "$want" "$log"; then
    grep -E "ui ready|selftest:" "$log"
    finish
    if [ -n "${TINKERPAD_SELFTEST:-}" ] && ! grep -q "selftest: done failures=0" "$log"; then
      echo "self-test reported failures"
      exit 1
    fi
    exit 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "app exited before '$want':"
    cat "$log"
    exit 1
  fi
  sleep 0.1
done

echo "timed out after ${timeout}s waiting for '$want'; output:"
cat "$log"
finish
exit 1
