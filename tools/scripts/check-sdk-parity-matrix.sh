#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
VALIDATOR="$REPO_ROOT/sdk/conformance/sdk_matrix.py"
MATRIX="${1:-$REPO_ROOT/sdk/conformance/sdk-parity-matrix.json}"

export PYTHONDONTWRITEBYTECODE="${PYTHONDONTWRITEBYTECODE:-1}"

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-parity-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  python3 "$VALIDATOR" --self-test --tmp "$tmp"
  exit 0
fi

if [[ -z "${EASYNET_SDK_PARITY_RESULTS_DIR:-}" ]]; then
  echo "sdk_parity_matrix: live_results_required" >&2
  exit 1
fi

python3 "$VALIDATOR" \
  --validate \
  --matrix "$MATRIX" \
  --results-dir "$EASYNET_SDK_PARITY_RESULTS_DIR" \
  ${EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS:+--allow-snapshot-results}
