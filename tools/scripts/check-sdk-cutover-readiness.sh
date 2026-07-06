#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

run_gate() {
  local name="$1"
  shift
  echo "== $name =="
  if "$@"; then
    echo "ok: $name"
    return 0
  else
    local rc=$?
    echo "failed: $name (exit $rc)" >&2
    return "$rc"
  fi
}

make_easyremote_good() {
  local root="$1"
  mkdir -p "$root/easyremote"
  cat >"$root/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
dependencies = ["easynet-sdk>=0.91.30"]
EOF
  cat >"$root/easyremote/client.py" <<'EOF'
from easynet_sdk import AbilityInvocationClient, InvocationDraft


def invoke(client: AbilityInvocationClient, draft: InvocationDraft):
    return client.invoke(draft)
EOF
}

make_backend_bad() {
  local root="$1"
  mkdir -p "$root/backend/internal/service"
  cat >"$root/backend/go.mod" <<'EOF'
module easynet-backend
EOF
  cat >"$root/backend/internal/service/forbidden.go" <<'EOF'
package service

import axonsdk "easynet.run/axon/sdk/go/easynet"

var _ = axonsdk.ErrInvalidArgument
EOF
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  run_gate "EasyRemote boundary self-test" bash "$SELF_DIR/check-easyremote-sdk-boundary.sh" --self-test
  run_gate "backend SDK-only boundary self-test" bash "$SELF_DIR/check-backend-sdk-only-boundary.sh" --self-test
  run_gate "backend route-family coverage self-test" bash "$SELF_DIR/check-backend-route-family-coverage.sh" --self-test
  run_gate "product smoke self-test" bash "$SELF_DIR/check-sdk-product-smokes.sh" --self-test

  easyremote_good="$tmp/EasyRemoteGood"
  backend_bad="$tmp/EasyNetBad"
  make_easyremote_good "$easyremote_good"
  make_backend_bad "$backend_bad"

  if EASYNET_EASYREMOTE_ROOT="$easyremote_good" EASYNET_BACKEND_ROOT="$backend_bad" "$0" >"$tmp/cutover.out" 2>&1; then
    echo "self-test expected aggregate cutover readiness to fail on raw backend Axon import" >&2
    exit 1
  fi
  grep -Fq "failed: backend SDK-only boundary" "$tmp/cutover.out"
  grep -Fq "raw_axon_import" "$tmp/cutover.out"

  echo "check-sdk-cutover-readiness self-test ok"
  exit 0
fi

EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet}"

status=0

run_gate "SDK scaffold" bash "$SELF_DIR/check-sdk-scaffold.sh" || status=1
run_gate "SDK parity matrix" bash "$SELF_DIR/check-sdk-parity-matrix.sh" --self-test || status=1
run_gate "daemon Invocation migration" bash "$SELF_DIR/check-daemon-invocation-migration.sh" || status=1
run_gate "EasyRemote SDK boundary" bash "$SELF_DIR/check-easyremote-sdk-boundary.sh" "$EASYREMOTE_ROOT" || status=1
run_gate "backend route-family coverage" bash "$SELF_DIR/check-backend-route-family-coverage.sh" || status=1
run_gate "backend SDK-only boundary" bash "$SELF_DIR/check-backend-sdk-only-boundary.sh" "$BACKEND_ROOT" || status=1
run_gate "product smokes" bash "$SELF_DIR/check-sdk-product-smokes.sh" || status=1

if [[ "$status" -eq 0 ]]; then
  echo "SDK cutover readiness ok"
else
  echo "SDK cutover readiness failed" >&2
fi
exit "$status"
