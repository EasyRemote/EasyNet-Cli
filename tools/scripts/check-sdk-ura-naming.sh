#!/usr/bin/env bash
set -euo pipefail

SDK_URA_NAMING_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The retired address token is forbidden when it forms an identifier token.
# Besides delimited words, that includes lower-to-upper CamelCase transitions
# and acronym boundaries. Requiring one of those boundaries prevents substrings
# such as SECURITY and security from being treated as retired address names.
SDK_URA_NAMING_TOKEN_UPPER="U""RI"
SDK_URA_NAMING_TOKEN_TITLE="U""ri"
SDK_URA_NAMING_TOKEN_LOWER="u""ri"
SDK_URA_NAMING_PATTERN="(^|[^[:alnum:]])(${SDK_URA_NAMING_TOKEN_UPPER}|${SDK_URA_NAMING_TOKEN_TITLE}|${SDK_URA_NAMING_TOKEN_LOWER})([[:upper:][:digit:]]|[^[:alnum:]]|$)|[[:lower:][:digit:]](${SDK_URA_NAMING_TOKEN_UPPER}|${SDK_URA_NAMING_TOKEN_TITLE})([[:upper:][:digit:]]|[^[:alnum:]]|$)"

sdk_ura_naming_collect_files() {
  local roots=(
    "docs/spec/daemon-sdk-requirements-v1.md"
    "sdk/SDK_INTERFACE_SPEC.md"
    "sdk/SDK_PARITY.md"
    "sdk/CONFORMANCE_SUITE.md"
    "sdk/conformance"
    "sdk/schemas"
    "include"
    "src/ffi"
    "sdk/go"
    "sdk/python"
    "sdk/node"
    "sdk/java"
    "sdk/swift"
  )
  local root
  for root in "${roots[@]}"; do
    if [[ -f "$SDK_URA_NAMING_ROOT/$root" ]]; then
      printf '%s\0' "$SDK_URA_NAMING_ROOT/$root"
    elif [[ -d "$SDK_URA_NAMING_ROOT/$root" ]]; then
      find "$SDK_URA_NAMING_ROOT/$root" \
        \( -path '*/target/*' \
          -o -path '*/build/*' \
          -o -path '*/.build/*' \
          -o -path '*/dist/*' \
          -o -path '*/node_modules/*' \
          -o -path '*/.venv/*' \
          -o -path '*/venv/*' \
          -o -path '*/site-packages/*' \
          -o -path '*/.mypy_cache/*' \
          -o -path '*/.pytest_cache/*' \
          -o -path '*/__pycache__/*' \
          -o -path '*/.eggs/*' \
          -o -path '*/*.egg-info/*' \
          -o -path '*/sdk/conformance/*.py' \
          -o -path '*/sdk/conformance/*.mjs' \
          -o -path '*/sdk/conformance/canonical-public-api.json' \
          -o -path '*/sdk/conformance/sdk-parity-matrix.json' \
          -o -path '*/sdk/conformance/runner/*' \
          -o -path '*/sdk/go/internal/axonpb/*' \
          -o -path '*/sdk/python/easynet_sdk/_axon_pb/*' \) -prune \
        -o -type f \
          ! -name '*.pyc' \
          ! -name '*.pb.go' \
          ! -name '*_pb2.py' \
          ! -name '*_pb2_grpc.py' \
          ! -name '*_pb2.pyi' \
          ! -name '*_pb2_grpc.pyi' \
          -print0
    fi
  done
}

sdk_ura_naming_scan_files() {
  local bad=""
  if [[ "$#" -gt 0 ]]; then
    bad="$(grep -IEn "$SDK_URA_NAMING_PATTERN" "$@" 2>/dev/null || true)"
  fi
  if [[ -n "$bad" ]]; then
    echo "SDK surfaces still contain retired address-token naming; use URA terminology:" >&2
    echo "$bad" >&2
    return 1
  fi
  return 0
}

# The Node seam sources the scanner so every SDK surface uses one naming
# grammar. Sourcing must not run this script's standalone entrypoint.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  good="$tmp/good.go"
  bad="$tmp/bad.go"
  cat >"$good" <<'EOF'
package good

const callerURA = "easynet:///r/example/agent/alice.sdk"
const SECURITY = "identity"
const security = "identity"
const SecurityClass = "identity"
const SECURITY_CLASS = "identity"
const securityClass = "identity"
EOF
  cat >"$bad" <<EOF
package bad

const agent_${SDK_URA_NAMING_TOKEN_LOWER} = "easynet:///r/example/agent/alice.sdk"
const Agent${SDK_URA_NAMING_TOKEN_UPPER} = "easynet:///r/example/agent/alice.sdk"
const Ability${SDK_URA_NAMING_TOKEN_TITLE} = "easynet:///r/example/ability/alice.echo"
const agent${SDK_URA_NAMING_TOKEN_TITLE} = "easynet:///r/example/agent/alice.sdk"
const DEVICE_${SDK_URA_NAMING_TOKEN_UPPER} = "easynet:///r/example/device/dev-a"
EOF
  sdk_ura_naming_scan_files "$good"
  if sdk_ura_naming_scan_files "$bad" >"$tmp/out" 2>&1; then
    echo "self-test expected retired address-token naming to fail" >&2
    exit 1
  fi
  for expected in \
    "agent_${SDK_URA_NAMING_TOKEN_LOWER}" \
    "Agent${SDK_URA_NAMING_TOKEN_UPPER}" \
    "Ability${SDK_URA_NAMING_TOKEN_TITLE}" \
    "agent${SDK_URA_NAMING_TOKEN_TITLE}" \
    "DEVICE_${SDK_URA_NAMING_TOKEN_UPPER}"
  do
    grep -Fq "$expected" "$tmp/out"
  done
  echo "check-sdk-ura-naming self-test ok"
  exit 0
fi

tmp_files="$(mktemp)"
trap 'rm -f "$tmp_files"' EXIT
sdk_ura_naming_collect_files >"$tmp_files"
if [[ -s "$tmp_files" ]]; then
  bad="$(xargs -0 grep -IEn "$SDK_URA_NAMING_PATTERN" <"$tmp_files" 2>/dev/null || true)"
else
  bad=""
fi
if [[ -n "$bad" ]]; then
  echo "SDK surfaces still contain retired address-token naming; use URA terminology:" >&2
  echo "$bad" >&2
  exit 1
fi
echo "SDK URA naming ok"
