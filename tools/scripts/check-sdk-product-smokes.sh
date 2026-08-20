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

require_python_sdk_root() {
  local root="$1"
  local package="$2"
  local label="$3"
  if [[ ! -d "$root" ]]; then
    echo "$label root does not exist: $root" >&2
    return 2
  fi
  if [[ ! -f "$root/pyproject.toml" ]]; then
    echo "$label root is missing pyproject.toml: $root" >&2
    return 2
  fi
  if [[ ! -d "$root/$package" ]]; then
    echo "$label root is missing canonical package directory: $root/$package" >&2
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
  cli_python="$tmp/EasyNet-Cli/sdk/python"
  axon_python="$tmp/EasyNet-Axon/sdk/python"
  mkdir -p \
    "$easyremote/easyremote" \
    "$backend_mono/backend" \
    "$cli_python/easynet_sdk" \
    "$axon_python/axon_sdk"
  cat >"$easyremote/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
version = "0.0.0"
EOF
  cat >"$cli_python/pyproject.toml" <<'EOF'
[project]
name = "easynet-sdk"
version = "0.0.0"
EOF
  cat >"$cli_python/easynet_sdk/__init__.py" <<'EOF'
SDK_SOURCE = "cli"
EOF
  cat >"$easyremote/easyremote/__init__.py" <<'EOF'
PRODUCT_SOURCE = "easyremote"
EOF
  cat >"$axon_python/pyproject.toml" <<'EOF'
[project]
name = "axon-sdk"
version = "0.0.0"
EOF
  cat >"$axon_python/axon_sdk/__init__.py" <<'EOF'
SDK_SOURCE = "axon"
EOF
  cat >"$backend_mono/backend/go.mod" <<'EOF'
module smoke-backend
EOF

  EASYNET_EASYREMOTE_ROOT="$easyremote" \
    EASYNET_BACKEND_ROOT="$backend_mono" \
    EASYNET_CLI_PYTHON_SDK_ROOT="$cli_python" \
    EASYNET_AXON_PYTHON_SDK_ROOT="$axon_python" \
    EASYNET_EASYREMOTE_SMOKE_CMD='python3 -c "import axon_sdk, easynet_sdk, easyremote; assert axon_sdk.SDK_SOURCE == \"axon\"; assert easynet_sdk.SDK_SOURCE == \"cli\"; assert easyremote.PRODUCT_SOURCE == \"easyremote\""' \
    EASYNET_BACKEND_SMOKE_CMD='test -f go.mod' \
    "$0" >/dev/null

  mv "$axon_python/axon_sdk" "$axon_python/easynet_axon"
  if EASYNET_EASYREMOTE_ROOT="$easyremote" \
    EASYNET_BACKEND_ROOT="$backend_mono" \
    EASYNET_CLI_PYTHON_SDK_ROOT="$cli_python" \
    EASYNET_AXON_PYTHON_SDK_ROOT="$axon_python" \
    EASYNET_EASYREMOTE_SMOKE_CMD='echo legacy-fallback-ran >&2' \
    EASYNET_BACKEND_SMOKE_CMD='true' \
    "$0" >"$tmp/missing-axon.out" 2>&1; then
    echo "self-test expected missing canonical Axon Python package to fail" >&2
    exit 1
  fi
  grep -Fq "missing canonical package directory" "$tmp/missing-axon.out"
  if grep -Fq "legacy-fallback-ran" "$tmp/missing-axon.out"; then
    echo "self-test executed smoke through a legacy Axon package fallback" >&2
    exit 1
  fi
  mv "$axon_python/easynet_axon" "$axon_python/axon_sdk"

  if EASYNET_EASYREMOTE_ROOT="$easyremote" \
    EASYNET_BACKEND_ROOT="$backend_mono" \
    EASYNET_CLI_PYTHON_SDK_ROOT="$cli_python" \
    EASYNET_AXON_PYTHON_SDK_ROOT="$axon_python" \
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
CLI_PYTHON_SDK_ROOT="${EASYNET_CLI_PYTHON_SDK_ROOT:-$REPO_ROOT/sdk/python}"
AXON_PYTHON_SDK_ROOT="${EASYNET_AXON_PYTHON_SDK_ROOT:-$REPO_ROOT/../EasyNet-Axon/sdk/python}"

# The product smoke loads both canonical SDK layers from sibling checkouts.
# Registry resolution or a product-named Axon package would hide migration
# failures behind environment or publication skew.
EASYREMOTE_SMOKE_CMD="${EASYNET_EASYREMOTE_SMOKE_CMD:-pytest -q}"
BACKEND_SMOKE_CMD="${EASYNET_BACKEND_SMOKE_CMD:-go test ./...}"

status=0

require_easyremote_root "$EASYREMOTE_ROOT" || status=1
require_backend_root "$BACKEND_ROOT" || status=1
require_python_sdk_root "$CLI_PYTHON_SDK_ROOT" "easynet_sdk" "EasyNet-Cli Python SDK" || status=1
require_python_sdk_root "$AXON_PYTHON_SDK_ROOT" "axon_sdk" "Axon Python SDK" || status=1

if [[ "$status" -eq 0 ]]; then
  printf -v easyremote_pythonpath '%q' "$EASYREMOTE_ROOT:$CLI_PYTHON_SDK_ROOT:$AXON_PYTHON_SDK_ROOT"
  run_smoke \
    "EasyRemote product tests" \
    "$EASYREMOTE_ROOT" \
    "PYTHONPATH=$easyremote_pythonpath $EASYREMOTE_SMOKE_CMD" || status=1
  run_smoke "backend product tests" "$BACKEND_ROOT" "$BACKEND_SMOKE_CMD" || status=1
fi

if [[ "$status" -eq 0 ]]; then
  echo "SDK product smokes ok"
else
  echo "SDK product smokes failed" >&2
fi
exit "$status"
