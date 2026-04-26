#!/usr/bin/env bash
# Baseline regression lock for AXON-RFC-001 conformance (EasyNet-Cli).
#
# Runs the conformance script in baseline mode, parses the current
# total, compares to docs/rfc/AXON-RFC-001-baseline-counts.txt.
# Exit 0 if current_total <= baseline_total. Exit 1 on regression.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CONFORMANCE_SCRIPT="$SCRIPT_DIR/check-rfc-001-conformance.sh"
BASELINE_FILE="$ROOT/docs/rfc/AXON-RFC-001-baseline-counts.txt"

if [[ ! -x "$CONFORMANCE_SCRIPT" ]]; then
  echo "FAIL: conformance script not executable: $CONFORMANCE_SCRIPT"
  exit 1
fi

if [[ ! -f "$BASELINE_FILE" ]]; then
  echo "FAIL: baseline file missing: $BASELINE_FILE"
  exit 1
fi

baseline_total="$(grep -E "^Total flagged occurrences:" "$BASELINE_FILE" | awk '{print $4}')"
if [[ -z "$baseline_total" ]]; then
  echo "FAIL: could not parse baseline total"
  exit 1
fi

current_output="$("$CONFORMANCE_SCRIPT" 2>&1)"
current_total="$(echo "$current_output" | grep -E "^Total flagged occurrences:" | awk '{print $4}')"
if [[ -z "$current_total" ]]; then
  echo "FAIL: could not parse current total"
  exit 1
fi

echo "AXON-RFC-001 baseline-lock — EasyNet-Cli"
echo "  baseline: $baseline_total"
echo "  current : $current_total"

if [[ "$current_total" -gt "$baseline_total" ]]; then
  delta=$((current_total - baseline_total))
  echo
  echo "FAIL: regression of $delta violations above baseline."
  echo "Either fix the new violations or update $BASELINE_FILE intentionally."
  exit 1
fi

if [[ "$current_total" -lt "$baseline_total" ]]; then
  delta=$((baseline_total - current_total))
  echo "PROGRESS: $delta violations removed since baseline."
fi

echo "PASS: no regression."
exit 0
