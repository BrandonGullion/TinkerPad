#!/usr/bin/env bash
# Fail if platform conditionals appear outside the allowed platform modules
# (listed in AGENTS.md; adding one needs a PLAN.md decision log entry).
# See docs/ARCHITECTURE.md §10 and work item 00-08.
set -euo pipefail

cd "$(dirname "$0")/.."

allowed='crates/tinkerpad-process/src/platform/|crates/tinkerpad-plugin/src/env|crates/tinkerpad-library/src/platform'

# Skip comment lines (//, //!, ///) so docs may mention the pattern.
hits=$(grep -rnE 'cfg\((target_os|windows|unix)' crates --include='*.rs' \
  | grep -vE "^($allowed)" \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)

if [ -n "$hits" ]; then
  echo "Platform cfg found outside the platform modules:"
  echo "$hits"
  exit 1
fi

echo "ok: no stray platform cfg"
