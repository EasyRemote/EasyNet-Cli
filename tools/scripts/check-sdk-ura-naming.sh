#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

PATTERN='(^|[^[:alnum:]_])(URI|Uri|uri)([^[:alnum:]_]|$)|[[:alnum:]_]+_uri([^[:alnum:]_]|$)|(^|[^[:alnum:]_])uri_[[:alnum:]_]+'

collect_files() {
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
    if [[ -f "$ROOT/$root" ]]; then
      printf '%s\0' "$ROOT/$root"
    elif [[ -d "$ROOT/$root" ]]; then
      find "$ROOT/$root" \
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

scan_paths() {
  local root="$1"
  shift
  local bad=""
  if [[ "$#" -gt 0 ]]; then
    bad="$(grep -IEn "$PATTERN" "$@" 2>/dev/null || true)"
  fi
  if [[ -n "$bad" ]]; then
    echo "SDK surfaces still contain URI-era naming; use URA terminology:" >&2
    echo "$bad" >&2
    return 1
  fi
  return 0
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  good="$tmp/good.go"
  bad="$tmp/bad.go"
  cat >"$good" <<'EOF'
package good

const callerURA = "easynet:///r/example/agent/alice.sdk"
EOF
  cat >"$bad" <<'EOF'
package bad

const agent_uri = "easynet:///r/example/agent/alice.sdk"
EOF
  scan_paths "$tmp" "$good"
  if scan_paths "$tmp" "$bad" >"$tmp/out" 2>&1; then
    echo "self-test expected URI-era naming to fail" >&2
    exit 1
  fi
  grep -Fq "agent_uri" "$tmp/out"
  echo "check-sdk-ura-naming self-test ok"
  exit 0
fi

tmp_files="$(mktemp)"
trap 'rm -f "$tmp_files"' EXIT
collect_files >"$tmp_files"
if [[ -s "$tmp_files" ]]; then
  bad="$(xargs -0 grep -IEn "$PATTERN" <"$tmp_files" 2>/dev/null || true)"
else
  bad=""
fi
if [[ -n "$bad" ]]; then
  echo "SDK surfaces still contain URI-era naming; use URA terminology:" >&2
  echo "$bad" >&2
  exit 1
fi
echo "SDK URA naming ok"
