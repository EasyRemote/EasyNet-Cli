#!/usr/bin/env bash
# Emit one selected target-process X11 observer plus an independent process
# observer as a single browser-E2E snapshot. The merger does not synthesize or
# correlate events; it only preserves both raw process-owned records.
set -euo pipefail

CONTAINER=""
SELECTED_STATE=""
UNRELATED_STATE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --container) CONTAINER="${2:?missing --container value}"; shift 2 ;;
    --selected-state) SELECTED_STATE="${2:?missing --selected-state value}"; shift 2 ;;
    --unrelated-state) UNRELATED_STATE="${2:?missing --unrelated-state value}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

[[ -n "$CONTAINER" && -n "$SELECTED_STATE" && -n "$UNRELATED_STATE" ]] || {
  echo "--container, --selected-state and --unrelated-state are required" >&2
  exit 64
}
[[ "$SELECTED_STATE" == /* && "$UNRELATED_STATE" == /* ]] || {
  echo "observer state paths must be absolute container paths" >&2
  exit 64
}

selected_json="$(docker exec "$CONTAINER" cat "$SELECTED_STATE")"
unrelated_json="$(docker exec "$CONTAINER" cat "$UNRELATED_STATE")"
jq -n \
  --argjson selected "$selected_json" \
  --argjson unrelated "$unrelated_json" \
  '$selected + {independent_observers: [$unrelated]}'
