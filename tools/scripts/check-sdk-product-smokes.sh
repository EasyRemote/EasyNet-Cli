#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

resolve_backend_root() {
  local candidate="$1"
  if [[ -f "$candidate/go.mod" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  if [[ -f "$candidate/backend/go.mod" ]]; then
    printf '%s\n' "$candidate/backend"
    return 0
  fi
  printf '%s\n' "$candidate"
}

require_easyremote_root() {
  local root="$1"
  if [[ ! -d "$root" ]]; then
    echo "EasyRemote root does not exist: $root" >&2
    return 2
  fi
  if [[ ! -f "$root/pyproject.toml" ]]; then
    echo "EasyRemote root is missing pyproject.toml: $root" >&2
    return 2
  fi
  if [[ ! -d "$root/easyremote" ]]; then
    echo "EasyRemote root is missing package directory: $root" >&2
    return 2
  fi
}

require_backend_root() {
  local root="$1"
  if [[ ! -d "$root" ]]; then
    echo "backend root does not exist: $root" >&2
    return 2
  fi
  if [[ ! -f "$root/go.mod" ]]; then
    echo "backend root is missing go.mod: $root" >&2
    return 2
  fi
}

run_smoke() {
  local name="$1"
  local root="$2"
  local command="$3"
  echo "== $name =="
  echo "root: $root"
  echo "cmd: $command"
  (cd "$root" && bash -lc "$command")
  local rc=$?
  if [[ "$rc" -ne 0 ]]; then
    echo "failed: $name (exit $rc)" >&2
    return "$rc"
  fi
  echo "ok: $name"
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  easyremote="$tmp/EasyRemote"
  backend_mono="$tmp/EasyNet"
  mkdir -p "$easyremote/easyremote" "$backend_mono/backend"
  cat >"$easyremote/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
version = "0.0.0"
EOF
  cat >"$backend_mono/backend/go.mod" <<'EOF'
module smoke-backend
EOF

  EASYNET_EASYREMOTE_ROOT="$easyremote" \
    EASYNET_BACKEND_ROOT="$backend_mono" \
    EASYNET_EASYREMOTE_SMOKE_CMD='test -f pyproject.toml && test -d easyremote' \
    EASYNET_BACKEND_SMOKE_CMD='test -f go.mod' \
    "$0" >/dev/null

  if EASYNET_EASYREMOTE_ROOT="$easyremote" \
    EASYNET_BACKEND_ROOT="$backend_mono" \
    EASYNET_EASYREMOTE_SMOKE_CMD='true' \
    EASYNET_BACKEND_SMOKE_CMD='echo backend-failed >&2; exit 17' \
    "$0" >"$tmp/fail.out" 2>&1; then
    echo "self-test expected backend product smoke to fail" >&2
    exit 1
  fi
  grep -Fq "backend product tests" "$tmp/fail.out"
  grep -Fq "backend-failed" "$tmp/fail.out"

  echo "check-sdk-product-smokes self-test ok"
  exit 0
fi

EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
BACKEND_INPUT_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet}"
BACKEND_ROOT="$(resolve_backend_root "$BACKEND_INPUT_ROOT")"

# The repository smoke must exercise the sibling SDK checkout. Resolving the
# published easynet-sdk wheel would pull its Axon dependency from the public
# index and hides local cutover failures behind registry version skew.
EASYREMOTE_SMOKE_CMD="${EASYNET_EASYREMOTE_SMOKE_CMD:-PYTHONPATH=.:../EasyNet-Cli/sdk/python pytest -q}"
BACKEND_SMOKE_CMD="${EASYNET_BACKEND_SMOKE_CMD:-go test ./...}"

status=0

require_easyremote_root "$EASYREMOTE_ROOT" || status=1
require_backend_root "$BACKEND_ROOT" || status=1

if [[ "$status" -eq 0 ]]; then
  run_smoke "EasyRemote product tests" "$EASYREMOTE_ROOT" "$EASYREMOTE_SMOKE_CMD" || status=1
  run_smoke "backend product tests" "$BACKEND_ROOT" "$BACKEND_SMOKE_CMD" || status=1
fi

if [[ "$status" -eq 0 ]]; then
  echo "SDK product smokes ok"
else
  echo "SDK product smokes failed" >&2
fi
exit "$status"
