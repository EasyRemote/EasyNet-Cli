#!/usr/bin/env bash
set -euo pipefail

SDK_URA_NAMING_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# URI is forbidden when it forms an identifier token. Besides delimited words,
# that includes lower-to-upper CamelCase transitions and acronym boundaries.
# Requiring one of those boundaries prevents substrings such as SECURITY and
# security from being treated as URI-era names.
SDK_URA_NAMING_PATTERN='(^|[^[:alnum:]])(URI|Uri|uri)([[:upper:][:digit:]]|[^[:alnum:]]|$)|[[:lower:][:digit:]](URI|Uri)([[:upper:][:digit:]]|[^[:alnum:]]|$)'

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
          -o -path '*/__pycache__/*' \
          -o -path '*/sdk/go/internal/axonpb/*' \
          -o -path '*/sdk/python/easynet_sdk/_axon_pb/*' \) -prune \
        -o -type f \
          ! -name '*.pyc' \
          ! -name '*.pb.go' \
          ! -name '*_pb2.py' \
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
    echo "SDK surfaces still contain URI-era naming; use URA terminology:" >&2
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
  cat >"$bad" <<'EOF'
package bad

const agent_uri = "easynet:///r/example/agent/alice.sdk"
const AgentURI = "easynet:///r/example/agent/alice.sdk"
const AbilityUri = "easynet:///r/example/ability/alice.echo"
const agentUri = "easynet:///r/example/agent/alice.sdk"
const DEVICE_URI = "easynet:///r/example/device/dev-a"
EOF
  sdk_ura_naming_scan_files "$good"
  if sdk_ura_naming_scan_files "$bad" >"$tmp/out" 2>&1; then
    echo "self-test expected URI-era naming to fail" >&2
    exit 1
  fi
  for expected in agent_uri AgentURI AbilityUri agentUri DEVICE_URI; do
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
  echo "SDK surfaces still contain URI-era naming; use URA terminology:" >&2
  echo "$bad" >&2
  exit 1
fi
echo "SDK URA naming ok"
