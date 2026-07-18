#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
VALIDATOR="$REPO_ROOT/sdk/conformance/sdk_matrix.py"
MATRIX="${1:-$REPO_ROOT/sdk/conformance/sdk-parity-matrix.json}"
REQUESTED_LANGUAGES="${EASYNET_SDK_PARITY_LANGUAGES:-}"
source "$REPO_ROOT/sdk/conformance/python_toolchain.sh"
source "$REPO_ROOT/sdk/conformance/toolchain_path.sh"

export PYTHONDONTWRITEBYTECODE="${PYTHONDONTWRITEBYTECODE:-1}"
resolve_sdk_toolchain_path "$REPO_ROOT"
resolve_sdk_python_toolchain "$REPO_ROOT" pytest

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-parity-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  "$SDK_CONFORMANCE_PYTHON" "$VALIDATOR" --self-test --tmp "$tmp"
  if EASYNET_SDK_PARITY_LANGUAGES=go,go \
    EASYNET_SDK_PARITY_RESULTS_DIR="$tmp/missing-results" \
    bash "$0" >"$tmp/duplicate.out" 2>&1; then
    echo "self-test expected duplicate parity language slice to fail" >&2
    exit 1
  fi
  grep -Fq "duplicate language slice entry: go" "$tmp/duplicate.out"
  if EASYNET_SDK_PARITY_LANGUAGES=bogus \
    EASYNET_SDK_PARITY_RESULTS_DIR="$tmp/missing-results" \
    bash "$0" >"$tmp/unknown.out" 2>&1; then
    echo "self-test expected unknown parity language slice to fail" >&2
    exit 1
  fi
  grep -Fq "unknown language slice entry: bogus" "$tmp/unknown.out"
  if EASYNET_SDK_PARITY_LANGUAGES=go \
    EASYNET_SDK_PARITY_RESULTS_DIR="$tmp/missing-results" \
    bash "$0" >"$tmp/slice.out" 2>&1; then
    echo "self-test expected missing focused parity results to fail" >&2
    exit 1
  fi
  grep -Fq "missing_live_results:go" "$tmp/slice.out"
  exit 0
fi

if [[ -z "${EASYNET_SDK_PARITY_RESULTS_DIR:-}" ]]; then
  echo "sdk_parity_matrix: live_results_required" >&2
  exit 1
fi

validator_mode=(--validate)
if [[ -n "$REQUESTED_LANGUAGES" ]]; then
  IFS=',' read -r -a requested_language_list <<<"$REQUESTED_LANGUAGES"
  normalized=""
  for requested in "${requested_language_list[@]}"; do
    case "$requested" in
      rust|c_abi|go|python|node|java|swift) ;;
      *)
        echo "sdk_parity_matrix: unknown language slice entry: $requested" >&2
        exit 1
        ;;
    esac
    if [[ ",$normalized," == *",$requested,"* ]]; then
      echo "sdk_parity_matrix: duplicate language slice entry: $requested" >&2
      exit 1
    fi
    normalized="${normalized:+$normalized,}$requested"
  done
  validator_mode=(--validate-slice "${requested_language_list[@]}")
fi

"$SDK_CONFORMANCE_PYTHON" "$VALIDATOR" \
  "${validator_mode[@]}" \
  --matrix "$MATRIX" \
  --results-dir "$EASYNET_SDK_PARITY_RESULTS_DIR" \
  ${EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS:+--allow-snapshot-results}
