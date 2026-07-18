#!/usr/bin/env bash
# backend-live-principal-e2e.sh — Backend account flow against a live daemon

set -euo pipefail

# Public input contract: EASYNET_BACKEND_ROOT may name either the EasyNet mono
# repository root or its backend Go module root. Internally this E2E family uses
# only the normalized module root that directly contains go.mod.
principal_lifecycle_resolve_backend_module_root() {
  local input_root="$1"
  local direct_module="$input_root/go.mod"
  local nested_module="$input_root/backend/go.mod"

  if [[ -f "$direct_module" && -f "$nested_module" ]]; then
    echo "[backend-live-principal-e2e] ambiguous Backend root has both go.mod and backend/go.mod: $input_root" >&2
    return 2
  fi
  if [[ -f "$direct_module" ]]; then
    (cd "$input_root" && pwd -P)
    return 0
  fi
  if [[ -f "$nested_module" ]]; then
    (cd "$input_root/backend" && pwd -P)
    return 0
  fi

  echo "[backend-live-principal-e2e] Backend root must be a module root containing go.mod or a mono root containing backend/go.mod: $input_root" >&2
  return 2
}

backend_live_principal_cleanup() {
  local status="$1"
  local smoke_home="$2"

  if [[ "$status" -ne 0 ]]; then
    echo "[backend-live-principal-e2e] FAIL: dumping hermetic daemon log from $smoke_home" >&2
    if [[ -f "$smoke_home/.easynet/backend-live-daemon.log" ]]; then
      tail -n 180 "$smoke_home/.easynet/backend-live-daemon.log" >&2 || true
    else
      find "$smoke_home" -maxdepth 3 -type f -print >&2 || true
    fi
  fi
  rm -rf "$smoke_home"
}

backend_live_principal_self_test() (
  local script_path="$1"
  local backend_module_root="$2"
  local fixture
  local mono_root
  local module_root
  local resolved

  bash -n "$script_path"

  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' EXIT
  mono_root="$fixture/EasyNet"
  module_root="$mono_root/backend"
  mkdir -p "$module_root"
  printf 'module backend-root-contract-test\n' >"$module_root/go.mod"

  resolved="$(principal_lifecycle_resolve_backend_module_root "$mono_root")"
  test "$resolved" = "$(cd "$module_root" && pwd -P)"
  resolved="$(principal_lifecycle_resolve_backend_module_root "$module_root")"
  test "$resolved" = "$(cd "$module_root" && pwd -P)"

  if principal_lifecycle_resolve_backend_module_root "$fixture/missing" >"$fixture/missing.out" 2>&1; then
    echo "[backend-live-principal-e2e] self-test expected an invalid Backend root to fail" >&2
    return 1
  fi
  grep -Fq "must be a module root containing go.mod or a mono root containing backend/go.mod" "$fixture/missing.out"

  mkdir -p "$fixture/ambiguous/backend"
  printf 'module ambiguous-root\n' >"$fixture/ambiguous/go.mod"
  printf 'module ambiguous-backend\n' >"$fixture/ambiguous/backend/go.mod"
  if principal_lifecycle_resolve_backend_module_root "$fixture/ambiguous" >"$fixture/ambiguous.out" 2>&1; then
    echo "[backend-live-principal-e2e] self-test expected an ambiguous Backend root to fail" >&2
    return 1
  fi
  grep -Fq "ambiguous Backend root" "$fixture/ambiguous.out"

  test -f "$backend_module_root/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "backend_live_daemon" "$backend_module_root/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "TestRegisterUserSigningKey_BackendAccountFlowUsesLiveDaemonPrincipalLifecycle" "$backend_module_root/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "OpenCABIDaemonTransport" "$backend_module_root/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  grep -q "principalprofile.NewClient" "$backend_module_root/internal/logic/user/register_user_signing_key_live_daemon_test.go"
  echo "backend-live-principal-e2e self-test ok"
)

backend_live_principal_main() {
  local self_dir
  local repo_root
  local backend_input_root
  local backend_module_root
  local daemon_bin
  local lib_ext
  local lib_path
  local smoke_home

  self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "$self_dir/../.." && pwd)"
  backend_input_root="${EASYNET_BACKEND_ROOT:-$repo_root/../EasyNet}"
  backend_module_root="$(principal_lifecycle_resolve_backend_module_root "$backend_input_root")"

  if [[ "${1:-}" == "--self-test" ]]; then
    backend_live_principal_self_test "${BASH_SOURCE[0]}" "$backend_module_root"
    return
  fi

  case "$(uname -s)" in
    Darwin) lib_ext="dylib" ;;
    Linux) lib_ext="so" ;;
    *)
      echo "[backend-live-principal-e2e] unsupported OS: $(uname -s)" >&2
      return 2
      ;;
  esac

  daemon_bin="$repo_root/target/debug/easynet-daemon"
  lib_path="$repo_root/target/debug/libeasynet_cli.${lib_ext}"

  echo "[backend-live-principal-e2e] rebuilding libeasynet_cli + daemon process set..."
  "$repo_root/tools/scripts/build-daemon-process-set.sh" --lib

  smoke_home="$(mktemp -d "/tmp/easynet-backend-live-principal.XXXXXX")"
  trap "backend_live_principal_cleanup \$? '$smoke_home'" EXIT

  echo "[backend-live-principal-e2e] running Backend live daemon PrincipalLifecycle E2E..."
  (
    cd "$backend_module_root"
    CGO_ENABLED=1 \
    EASYNET_BACKEND_LIVE_DAEMON_LIB="$lib_path" \
    EASYNET_BACKEND_LIVE_DAEMON_BIN="$daemon_bin" \
    EASYNET_BACKEND_LIVE_DAEMON_HOME="$smoke_home" \
    go test -tags "easynet_cabi backend_live_daemon" ./internal/logic/user -run '^TestRegisterUserSigningKey_BackendAccountFlowUsesLiveDaemonPrincipalLifecycle$' -count=1 -v
  )

  echo "[backend-live-principal-e2e] PASS"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  backend_live_principal_main "$@"
fi
