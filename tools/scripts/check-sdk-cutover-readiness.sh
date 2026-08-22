#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
CONFORMANCE_REPORTS_SCRIPT="${SDK_CUTOVER_CONFORMANCE_REPORTS_SCRIPT:-$SELF_DIR/check-sdk-conformance-reports.sh}"
PARITY_MATRIX_SCRIPT="${SDK_CUTOVER_PARITY_MATRIX_SCRIPT:-$SELF_DIR/check-sdk-parity-matrix.sh}"
source "$REPO_ROOT/sdk/conformance/toolchain_path.sh"
source "$REPO_ROOT/sdk/conformance/python_toolchain.sh"
resolve_sdk_toolchain_path "$REPO_ROOT"
resolve_sdk_python_toolchain "$REPO_ROOT" pytest
PYTHON_BIN="$SDK_CONFORMANCE_PYTHON"

# Reuse the PrincipalLifecycle E2E root contract. Sourcing defines the resolver
# without executing the focused E2E.
source "$SELF_DIR/backend-live-principal-e2e.sh"

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

check_retired_edge_adapter_policy_absence() {
  local policy_script="$REPO_ROOT/sdk/conformance/edge_adapter_policy.py"
  local policy_manifest="$REPO_ROOT/sdk/conformance/edge-adapter-policy.v1.json"

  if [[ -e "$policy_script" ]]; then
    echo "retired edge-adapter policy script still exists: $policy_script" >&2
    return 1
  fi
  if [[ -e "$policy_manifest" ]]; then
    echo "retired edge-adapter policy manifest still exists: $policy_manifest" >&2
    return 1
  fi
}

run_sdk_conformance_live_gates() {
  local live_results_dir="$1"
  local parity_languages="${EASYNET_SDK_PARITY_LANGUAGES:-${SDK_CONFORMANCE_LANGUAGES:-}}"
  local status=0
  run_gate "SDK conformance reports" env \
    SDK_CONFORMANCE_RESULT_DIR="$live_results_dir" \
    bash "$CONFORMANCE_REPORTS_SCRIPT" || status=1
  run_gate "SDK live parity matrix" env \
    EASYNET_SDK_PARITY_RESULTS_DIR="$live_results_dir" \
    EASYNET_SDK_PARITY_LANGUAGES="$parity_languages" \
    EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 \
    bash "$PARITY_MATRIX_SCRIPT" || status=1
  return "$status"
}

allocate_live_results_dir() {
  if [[ -n "${SDK_CUTOVER_LIVE_RESULTS_DIR:-}" ]]; then
    mkdir -p "$SDK_CUTOVER_LIVE_RESULTS_DIR"
    printf '%s\n' "$SDK_CUTOVER_LIVE_RESULTS_DIR"
    return 0
  fi
  mkdir -p "$REPO_ROOT/target"
  mktemp -d "$REPO_ROOT/target/sdk-conformance-live-results.cutover.XXXXXX"
}

make_easyremote_good() {
  local root="$1"
  mkdir -p "$root/easyremote"
  cat >"$root/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
dependencies = ["easynet-sdk>=0.142.22,<0.143"]
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

import axonsdk "axon.run/sdk/go/axon"

var _ = axonsdk.ErrInvalidArgument
EOF
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  run_gate "EasyRemote boundary self-test" bash "$SELF_DIR/check-easyremote-sdk-boundary.sh" --self-test
  run_gate "backend SDK-only boundary self-test" bash "$SELF_DIR/check-backend-sdk-only-boundary.sh" --self-test
  run_gate "backend route-family coverage self-test" bash "$SELF_DIR/check-backend-route-family-coverage.sh" --self-test
  run_gate "SDK completion matrix self-test" bash "$SELF_DIR/check-sdk-completion-audit.sh" --self-test
  run_gate "SDK URA naming self-test" bash "$SELF_DIR/check-sdk-ura-naming.sh" --self-test
  run_gate "retired edge-adapter policy absence self-test" check_retired_edge_adapter_policy_absence
  run_gate "canonical runtime convergence V2 self-test" bash "$SELF_DIR/check-canonical-runtime-convergence-v2.sh" --self-test
  run_gate "SDK product-neutrality syntax" bash -n "$SELF_DIR/check-sdk-product-neutrality.sh"
  run_gate "SDK conformance reports self-test" bash "$SELF_DIR/check-sdk-conformance-reports.sh" --self-test
  run_gate "generic FFI ABI v7 exact-surface self-test" bash "$REPO_ROOT/tests/scripts/test_check_ffi_abi_v7_header.sh"
  run_gate "SDK package metadata self-test" bash "$SELF_DIR/check-sdk-package-metadata.sh" --self-test
  run_gate "downstream SDK consumer cutover self-test" bash "$SELF_DIR/check-downstream-sdk-consumer-cutover.sh" --self-test
  run_gate "product key-custody boundary self-test" bash "$SELF_DIR/check-product-key-custody-boundary.sh" --self-test
  run_gate "product smoke self-test" bash "$SELF_DIR/check-sdk-product-smokes.sh" --self-test
  run_gate "runtime events cross-repo gate self-test" bash "$SELF_DIR/runtime-events-cross-repo-e2e.sh" --self-test
  run_gate "runtime events live daemon E2E self-test" bash "$SELF_DIR/runtime-events-live-daemon-e2e.sh" --self-test
  run_gate "standalone Hub PrincipalLifecycle E2E self-test" bash "$SELF_DIR/standalone-hub-principal-lifecycle-e2e.sh" --self-test
  run_gate "CLI Hub/Device daemon E2E self-test" bash "$SELF_DIR/cli-hub-device-daemon-e2e.sh" --self-test
  run_gate "Docker two-node EasyRemote CLI E2E self-test" bash "$SELF_DIR/docker-two-node-easyremote-cli-e2e.sh" --self-test
  run_gate "Docker media/bidi E2E self-test" bash "$SELF_DIR/docker-media-bidi-e2e.sh" --self-test
  run_gate "Python SDK live smoke self-test" bash "$SELF_DIR/python-sdk-live-smoke.sh" --self-test
  run_gate "Go SDK live smoke self-test" bash "$SELF_DIR/go-sdk-live-smoke.sh" --self-test
  run_gate "Python SDK static contract self-test" bash "$SELF_DIR/check-python-sdk-static-contract.sh" --self-test
  run_gate "release package contract self-test" bash "$REPO_ROOT/tests/scripts/test_check_release_package_contract.sh"

  focused_live_results="$tmp/cutover-live-results"
  stale_default="$tmp/sdk-conformance-live-results"
  mkdir -p "$stale_default"
  printf 'stale\n' >"$stale_default/rust.json"
  cat >"$tmp/stub-conformance-reports.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ -z "${SDK_CONFORMANCE_RESULT_DIR:-}" ]]; then
  echo "missing SDK_CONFORMANCE_RESULT_DIR" >&2
  exit 1
fi
mkdir -p "$SDK_CONFORMANCE_RESULT_DIR"
printf 'fresh\n' >"$SDK_CONFORMANCE_RESULT_DIR/fresh-live-result"
EOF
  cat >"$tmp/stub-parity-matrix.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ -z "${EASYNET_SDK_PARITY_RESULTS_DIR:-}" ]]; then
  echo "missing EASYNET_SDK_PARITY_RESULTS_DIR" >&2
  exit 1
fi
test -f "$EASYNET_SDK_PARITY_RESULTS_DIR/fresh-live-result"
if [[ -n "${EXPECTED_PARITY_LANGUAGES:-}" ]]; then
  test "${EASYNET_SDK_PARITY_LANGUAGES:-}" = "$EXPECTED_PARITY_LANGUAGES"
fi
if [[ -f "$EASYNET_SDK_PARITY_RESULTS_DIR/rust.json" ]]; then
  echo "stale default live result was used" >&2
  exit 1
fi
EOF
  chmod +x "$tmp/stub-conformance-reports.sh" "$tmp/stub-parity-matrix.sh"
  CONFORMANCE_REPORTS_SCRIPT="$tmp/stub-conformance-reports.sh" \
    PARITY_MATRIX_SCRIPT="$tmp/stub-parity-matrix.sh" \
    run_sdk_conformance_live_gates "$focused_live_results"
  if [[ ! -f "$focused_live_results/fresh-live-result" ]]; then
    echo "self-test expected focused live result to be written" >&2
    exit 1
  fi
  if [[ ! -f "$stale_default/rust.json" ]]; then
    echo "self-test expected stale default fixture to remain outside focused run" >&2
    exit 1
  fi
  CONFORMANCE_REPORTS_SCRIPT="$tmp/stub-conformance-reports.sh" \
    PARITY_MATRIX_SCRIPT="$tmp/stub-parity-matrix.sh" \
    SDK_CONFORMANCE_LANGUAGES=go,python \
    EXPECTED_PARITY_LANGUAGES=go,python \
    run_sdk_conformance_live_gates "$tmp/cutover-live-results-slice"

  easyremote_good="$tmp/EasyRemoteGood"
  backend_bad="$tmp/EasyNetBad"
  make_easyremote_good "$easyremote_good"
  make_backend_bad "$backend_bad"

  if bash "$SELF_DIR/check-backend-sdk-only-boundary.sh" "$backend_bad" >"$tmp/cutover.out" 2>&1; then
    echo "self-test expected backend boundary to fail on raw backend Axon import" >&2
    exit 1
  fi
  grep -Fq "raw_axon_import" "$tmp/cutover.out"

  echo "check-sdk-cutover-readiness self-test ok"
  exit 0
fi

EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
BACKEND_INPUT_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet}"
BACKEND_MODULE_ROOT="$(principal_lifecycle_resolve_backend_module_root "$BACKEND_INPUT_ROOT")"
CUTOVER_LIVE_RESULTS_DIR="$(allocate_live_results_dir)"

status=0

run_gate "SDK scaffold" bash "$SELF_DIR/check-sdk-scaffold.sh" || status=1
run_gate "SDK parity matrix" bash "$SELF_DIR/check-sdk-parity-matrix.sh" --self-test || status=1
run_gate "SDK completion matrix" bash "$SELF_DIR/check-sdk-completion-audit.sh" --matrix-only || status=1
run_gate "SDK canonical public API" bash "$SELF_DIR/check-sdk-canonical-public-api.sh" || status=1
run_gate "retired edge-adapter policy absence" check_retired_edge_adapter_policy_absence || status=1
run_gate "SDK product neutrality" bash "$SELF_DIR/check-sdk-product-neutrality.sh" || status=1
run_sdk_conformance_live_gates "$CUTOVER_LIVE_RESULTS_DIR" || status=1
run_gate "generic FFI ABI v7 exact surface" bash "$SELF_DIR/check-ffi-abi-v7-header.sh" || status=1
run_gate "SDK package metadata" bash "$SELF_DIR/check-sdk-package-metadata.sh" || status=1
run_gate "SDK URA naming" bash "$SELF_DIR/check-sdk-ura-naming.sh" || status=1
run_gate "canonical runtime convergence V2" bash "$SELF_DIR/check-canonical-runtime-convergence-v2.sh" || status=1
run_gate "SDK receipt URA boundary" bash "$SELF_DIR/check-sdk-receipt-ura-boundary.sh" || status=1
run_gate "Python SDK static contract" bash "$SELF_DIR/check-python-sdk-static-contract.sh" || status=1
run_gate "daemon latest input boundary" bash "$SELF_DIR/check-daemon-latest-input-boundary.sh" || status=1
run_gate "daemon Invocation migration" bash "$SELF_DIR/check-daemon-invocation-migration.sh" || status=1
run_gate "release package contract" bash "$SELF_DIR/check-release-package-contract.sh" || status=1
run_gate "EasyRemote SDK boundary" bash "$SELF_DIR/check-easyremote-sdk-boundary.sh" "$EASYREMOTE_ROOT" || status=1
run_gate "backend route-family coverage" bash "$SELF_DIR/check-backend-route-family-coverage.sh" || status=1
run_gate "backend SDK-only boundary" bash "$SELF_DIR/check-backend-sdk-only-boundary.sh" "$BACKEND_MODULE_ROOT" || status=1
run_gate "downstream SDK consumer cutover" bash "$SELF_DIR/check-downstream-sdk-consumer-cutover.sh" "$BACKEND_MODULE_ROOT" "$EASYREMOTE_ROOT" || status=1
run_gate "product key-custody boundary" bash "$SELF_DIR/check-product-key-custody-boundary.sh" "$BACKEND_MODULE_ROOT" "$EASYREMOTE_ROOT" || status=1
run_gate "product smokes" env EASYNET_BACKEND_ROOT="$BACKEND_MODULE_ROOT" bash "$SELF_DIR/check-sdk-product-smokes.sh" || status=1
run_gate "runtime events cross-repo gate" env EASYNET_BACKEND_ROOT="$BACKEND_MODULE_ROOT" bash "$SELF_DIR/runtime-events-cross-repo-e2e.sh" || status=1
run_gate "runtime events live daemon E2E" bash "$SELF_DIR/runtime-events-live-daemon-e2e.sh" || status=1
run_gate "standalone Hub PrincipalLifecycle E2E" env EASYNET_BACKEND_ROOT="$BACKEND_MODULE_ROOT" bash "$SELF_DIR/standalone-hub-principal-lifecycle-e2e.sh" || status=1
run_gate "Docker two-node EasyRemote CLI E2E source contract" bash "$SELF_DIR/docker-two-node-easyremote-cli-e2e.sh" --self-test || status=1
run_gate "Docker media/bidi E2E source contract" bash "$SELF_DIR/docker-media-bidi-e2e.sh" --self-test || status=1
run_gate "Python SDK live smoke" bash "$SELF_DIR/python-sdk-live-smoke.sh" || status=1
run_gate "Go SDK live smoke" bash "$SELF_DIR/go-sdk-live-smoke.sh" || status=1

if [[ "$status" -eq 0 ]]; then
  echo "SDK cutover readiness ok"
else
  echo "SDK cutover readiness failed" >&2
fi
exit "$status"
