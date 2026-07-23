#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
AXON_ROOT="${EASYNET_AXON_ROOT:-$ROOT/../EasyNet-Axon}"
CANONICAL_LIFECYCLE_AXON_ROOT="$AXON_ROOT"
source "$ROOT/sdk/conformance/toolchain_path.sh"
source "$ROOT/sdk/conformance/python_toolchain.sh"
resolve_sdk_toolchain_path "$ROOT"
resolve_sdk_python_toolchain "$ROOT"
PYTHON_BIN="$SDK_CONFORMANCE_PYTHON"
MANIFEST="$ROOT/sdk/conformance/canonical-public-api.json"
MATRIX="$ROOT/sdk/conformance/sdk-parity-matrix.json"
EDGE_ADAPTER_POLICY="$ROOT/sdk/conformance/edge_adapter_policy.py"

fail() {
  echo "canonical-runtime-convergence-v2: $*" >&2
  exit 1
}

check_mcp_reflection_async_bridge_contract() {
  local cli_root="${1:-$ROOT}"
  local reflective="$cli_root/src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs"
  local bridge="$cli_root/src/support/async_bridge/mod.rs"
  local local_invoker="$cli_root/src/daemon/invocation/dispatch/local_runtime_invoker.rs"
  local device_ops="$cli_root/src/daemon/ability/builtins/device_control/ability_management/ops.rs"
  local real_smoke="$cli_root/src/bin/real-user-smoke.rs"
  local source_root="$cli_root/src"
  [[ -e "$reflective" ]] || fail "MCP reflective registry source not found"
  [[ -e "$bridge" ]] || fail "canonical async bridge source not found"
  [[ -e "$local_invoker" ]] || fail "LocalRuntime invoker source not found"

  if rg -n 'fn\s+run_blocking\s*<|tokio::runtime::Builder::new_current_thread\(\)' "$reflective"; then
    fail "MCP reflective registry must not own a private async runtime bridge"
  fi
  if ! rg -q 'try_run_blocking' "$reflective"; then
    fail "MCP reflective registry must use the canonical fallible async bridge"
  fi
  if ! rg -q 'spawn_current_thread_tokio' "$reflective"; then
    fail "MCP reflective registry lazy worker must use the canonical async bridge spawner"
  fi
  if ! rg -q 'pub fn try_run_blocking' "$bridge"; then
    fail "canonical async bridge must expose a fallible runtime bridge provider"
  fi
  if ! rg -q 'pub fn spawn_current_thread_tokio' "$bridge"; then
    fail "canonical async bridge must expose a detached current-thread runtime spawner"
  fi
  if ! rg -q 'pub enum SyncBridgeRuntimePolicy' "$bridge"; then
    fail "canonical async bridge must expose explicit sync bridge runtime policy"
  fi
  if rg -n 'NoRuntimeFallback' "$source_root"; then
    fail "canonical async bridge preserves retired NoRuntimeFallback policy type"
  fi
  if rg -n 'fallback policy|no-runtime fallback|build-tokio fallback|honors_build_tokio_fallback|BuildCurrentThreadTokio` fallback|BuildCurrentThreadTokio fallback' \
    "$bridge" "$local_invoker" "$reflective" "$device_ops" "$real_smoke"; then
    fail "canonical async bridge preserves retired fallback policy vocabulary"
  fi
}

check_runtime_session_projection_accessor_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local projection="$cli_root/src/daemon/boot/lifecycle/projection.rs"
  local lifecycle="$cli_root/src/daemon/boot/lifecycle"
  local status="$cli_root/src/cli/commands/status.rs"
  local mcp="$cli_root/src/cli/commands/groups/mcp.rs"
  [[ -f "$projection" ]] || fail "runtime session projection source is missing: ${projection#$cli_root/}"

  if rg -n 'as_runtime_state|legacy CLI renderers|on-disk compatibility shape|compatibility contract' \
    "$projection" "$lifecycle" "$status" "$mcp"; then
    fail "runtime session projection preserves retired legacy/compatibility accessor vocabulary"
  fi
  if ! rg -q 'pub fn state\(&self\) -> &config::RuntimeState' "$projection"; then
    fail "RuntimeSessionProjection must expose the persisted projection through state()"
  fi
  if ! rg -q 'pub fn into_runtime_state\(self\) -> config::RuntimeState' "$projection"; then
    fail "RuntimeSessionProjection must keep the explicit consuming persistence conversion"
  fi
}

check_ffi_runtime_sizing_policy_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local handle="$cli_root/src/ffi/client/handle.rs"
  [[ -f "$handle" ]] || fail "FFI client handle source is missing: ${handle#$cli_root/}"

  if rg -n 'FALLBACK_FFI_WORKER_THREADS|device_default_ffi_worker_threads|legacy blocking ABI' "$handle"; then
    fail "FFI runtime sizing preserves retired fallback/device ownership vocabulary"
  fi
  if ! rg -q 'const MIN_FFI_WORKER_THREADS: usize = 4;' "$handle"; then
    fail "FFI runtime sizing must name the automatic worker lower bound explicitly"
  fi
  if ! rg -q 'fn host_default_ffi_worker_threads\(\) -> usize' "$handle"; then
    fail "FFI runtime sizing must compute host-default worker count"
  fi
  if ! rg -q 'unwrap_or_else\(host_default_ffi_worker_threads\)' "$handle"; then
    fail "FFI worker override must fall through to host-default sizing"
  fi
}

check_ffi_init_typed_connect_error_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ffi_mod="$cli_root/src/ffi/mod.rs"
  local ipc="$cli_root/src/ffi/client/ipc.rs"
  [[ -f "$ffi_mod" ]] || fail "FFI module source is missing: ${ffi_mod#$cli_root/}"
  [[ -f "$ipc" ]] || fail "FFI IPC client source is missing: ${ipc#$cli_root/}"

  if rg -n 'msg\.contains\("version negotiation failed"\)|version negotiation failed.*ERR_VERSION_INCOMPATIBLE|Fall back to\s+ERR_DAEMON_DOWN' "$ffi_mod"; then
    fail "runtime_init preserves retired string-scanned IPC version fallback"
  fi
  if ! rg -q 'pub enum IpcConnectError' "$ipc"; then
    fail "FFI IPC connect must expose typed connect errors"
  fi
  if ! rg -q 'VersionIncompatible' "$ipc"; then
    fail "FFI IPC connect must expose typed version incompatibility"
  fi
  if ! rg -q 'IpcConnectError::VersionIncompatible' "$ffi_mod"; then
    fail "runtime_init must map typed IPC version incompatibility"
  fi
  if ! rg -q 'init_returns_typed_version_incompatible_without_message_fallback' "$ffi_mod"; then
    fail "runtime_init typed IPC version incompatibility regression test is missing"
  fi
}

check_failure_code_default_policy_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local classifier="$cli_root/src/daemon/execution/mission/failure_codes.rs"
  [[ -f "$classifier" ]] || fail "failure code classifier source is missing: ${classifier#$cli_root/}"

  if rg -n 'classify_or\(|explicit_or_reason\(|pub fn normalize\(|fallback' "$classifier"; then
    fail "failure code classifier preserves retired fallback/default API vocabulary"
  fi
  if ! rg -q 'pub fn classify_or_default\(reason: &str, default_code: &str\) -> String' "$classifier"; then
    fail "failure code classifier must expose classify_or_default"
  fi
  if ! rg -q 'pub fn explicit_or_reason_default' "$classifier"; then
    fail "failure code classifier must expose explicit_or_reason_default"
  fi
  if ! rg -q 'pub fn normalize_or_default\(candidate: &str, default_code: &str\) -> String' "$classifier"; then
    fail "failure code classifier must expose normalize_or_default"
  fi
}

check_bidi_dispatch_default_code_policy_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local dispatcher="$cli_root/src/daemon/invocation/bidi/bidi_dispatcher.rs"
  [[ -f "$dispatcher" ]] || fail "bidi dispatcher source is missing: ${dispatcher#$cli_root/}"

  if rg -n 'fallback_code|let fallback =|unary map as fallback' "$dispatcher"; then
    fail "bidi dispatch terminal failure path preserves retired fallback vocabulary"
  fi
  if ! rg -q 'default_code: &str' "$dispatcher"; then
    fail "bidi dispatch failure helper must name its caller-owned default_code"
  fi
  if ! rg -q 'default_code,' "$dispatcher"; then
    fail "bidi dispatch failure helper must forward default_code into SessionFailure"
  fi
}

check_bidi_reverse_unary_terminal_state_contract() {
  local cli_root="${1:-$ROOT}"
  local escalation="$cli_root/src/daemon/invocation/bidi/session_escalation.rs"
  [[ -f "$escalation" ]] || fail "bidi session escalation source is missing: ${escalation#$cli_root/}"

  "$PYTHON_BIN" - "$escalation" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
legacy = """.terminal_receipt
                .as_ref()
                .map(|receipt| receipt.state)
                .unwrap_or(axon_sdk::pb::axon::v1::InvocationState::Completed as i32)"""
if legacy in text:
    raise SystemExit("bidi_reverse_unary_terminal_state:completed_default_projection")
required = {
    "fn reverse_unary_terminal_state": "terminal_state_validator_missing",
    "CANONICAL_FINALIZATION_REQUIRED": "missing_checkpoint_error_missing",
    "CANONICAL_ADMISSION_INVALID": "admission_checkpoint_invalid_error_missing",
    "CANONICAL_TERMINAL_RECEIPT_INVALID": "terminal_receipt_invalid_error_missing",
    "InvocationState::Admitted.to_wire_i32()": "admission_state_machine_check_missing",
    "state.is_terminal()": "terminal_state_machine_check_missing",
    "reverse_unary_reply_rejects_missing_canonical_checkpoints": "missing_checkpoint_test_missing",
    "reverse_unary_reply_rejects_non_admitted_admission_checkpoint": "non_admitted_checkpoint_test_missing",
    "reverse_unary_reply_rejects_non_terminal_receipt_state": "non_terminal_state_test_missing",
}
for needle, label in required.items():
    if needle not in text:
        raise SystemExit(f"bidi_reverse_unary_terminal_state:{label}")
PY
}

check_cabi_bidi_cancel_reason_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local cabi="$cli_root/sdk/go/cabi_runtime.go"
  local test="$cli_root/sdk/go/cabi_runtime_test.go"
  [[ -f "$cabi" ]] || fail "Go C ABI runtime transport is missing"
  [[ -f "$test" ]] || fail "Go C ABI runtime transport tests are missing"

  "$PYTHON_BIN" - "$cabi" "$test" <<'PY'
import sys
from pathlib import Path

cabi = Path(sys.argv[1]).read_text(encoding="utf-8")
test = Path(sys.argv[2]).read_text(encoding="utf-8")

start = cabi.find("func (b *cabiBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {")
end = cabi.find("func (b *cabiBidiTransport) closeFromOwner", start)
if start < 0 or end < 0:
    raise SystemExit("go_cabi_bidi_cancel_function_missing")
body = cabi[start:end]
for retired in (
    "_ = reason",
    '"reason":"cancelled"',
    '`{"session_id":%q,"state":"CancelRequested","terminal":false,"reason":"cancelled"}`',
):
    if retired in body:
        raise SystemExit(f"go_cabi_bidi_cancel_reason_fallback:{retired}")
if "Reason    string `json:\"reason\"`" not in body or "Reason:    reason" not in body:
    raise SystemExit("go_cabi_bidi_cancel_reason_projection_missing")
if 'observation.cancel.Reason() != "client stop"' not in test:
    raise SystemExit("go_cabi_bidi_cancel_reason_regression_test_missing")
PY
}

check_terminal_lifecycle_args_contract() {
  local cli_root="${1:-$ROOT}"
  local lifecycle="$cli_root/src/daemon/ability/builtins/device_control/terminal/lifecycle.rs"
  [[ -f "$lifecycle" ]] || fail "terminal lifecycle source is missing: ${lifecycle#$cli_root/}"

  "$PYTHON_BIN" - "$lifecycle" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
for retired in (
    "drop unknown fields silently",
    "future schema addition mustn't break old",
    "unknown fields must be tolerated",
    "parse_create_spec_drops_unknown_fields_silently",
):
    if retired in text:
        raise SystemExit(f"terminal_lifecycle_args:retired_compat:{retired}")
for required in (
    "fn require_lifecycle_args",
    "terminal.create",
    "&[\"cols\", \"rows\", \"command\", \"command_args\", \"cwd\", \"env\"]",
    "terminal.list",
    "terminal.close",
    "unknown argument",
    "parse_create_spec_rejects_unknown_fields",
    "list_rejects_unknown_argument",
    "close_rejects_unknown_argument",
    "parse_create_spec_rejects_non_string_command_and_cwd",
    "`command` must be a string",
    "`cwd` must be a string",
):
    if required not in text:
        raise SystemExit(f"terminal_lifecycle_args:missing:{required}")
PY
}

check_session_failure_wire_facts_contract() {
  local cli_root="${1:-$ROOT}"
  local failure="$cli_root/src/daemon/invocation/bidi/state/session_failure.rs"
  [[ -f "$failure" ]] || fail "session failure source is missing: ${failure#$cli_root/}"

  "$PYTHON_BIN" - "$failure" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
match = re.search(r"pub struct SessionFailure\s*\{(?P<body>.*?)\n\}", text, re.S)
if not match:
    raise SystemExit("session_failure_wire_facts:struct_missing")
body = match.group("body")
if "#[serde(default)]" in body:
    raise SystemExit("session_failure_wire_facts:serde_default_present")
required = {
    "pub retryable: bool": "retryable_field_missing",
    "pub stage: i32": "stage_field_missing",
    "pub security_class: i32": "security_class_field_missing",
    "session_failure_wire_requires_retry_and_classification_facts": "missing_facts_test_missing",
    "session_failure_wire_round_trips_complete_facts": "complete_facts_roundtrip_test_missing",
}
for needle, label in required.items():
    if needle not in text:
        raise SystemExit(f"session_failure_wire_facts:{label}")
PY
}

check_manifest_contract() {
  "$PYTHON_BIN" - \
    "$MANIFEST" \
    "$MATRIX" \
    "$AXON_ROOT/sdk/conformance/lifecycle/capability-matrix.v1.json" \
    "$AXON_ROOT/sdk/conformance/lifecycle/lifecycle-vectors.v1.json" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
matrix = json.loads(Path(sys.argv[2]).read_text())
axon_matrix_path = Path(sys.argv[3])
axon_vectors_path = Path(sys.argv[4])
axon_matrix = json.loads(axon_matrix_path.read_text())
axon_vectors = json.loads(axon_vectors_path.read_text())
expected_status_names = {
    "unsupported": "Unsupported",
    "seam": "Seam",
    "provider-backed": "ProviderBacked",
    "cutover-ready": "CutoverReady",
}
matrix_contract = axon_matrix.get("provider_contract")
vector_contract = axon_vectors.get("provider_contract")
if (
    not isinstance(matrix_contract, dict)
    or not isinstance(vector_contract, dict)
    or {
        "id": matrix_contract.get("id"),
        "version": matrix_contract.get("version"),
    } != vector_contract
):
    raise SystemExit("axon:canonical_lifecycle_provider_contract")
expected_reference = {
    "owner_repository": "EasyNet-Axon",
    "provider_contract": vector_contract,
    "capability_matrix": {
        "path": "sdk/conformance/lifecycle/capability-matrix.v1.json",
        "sha256": hashlib.sha256(axon_matrix_path.read_bytes()).hexdigest(),
    },
    "transition_vectors": {
        "path": "sdk/conformance/lifecycle/lifecycle-vectors.v1.json",
        "sha256": hashlib.sha256(axon_vectors_path.read_bytes()).hexdigest(),
    },
}
for name, document in (("manifest", manifest), ("matrix", matrix)):
    if document.get("schema_version") != 5:
        raise SystemExit(f"{name}:schema_version")
    if document.get("status_canonical_names") != expected_status_names:
        raise SystemExit(f"{name}:status_canonical_names")
    if document.get("canonical_lifecycle_contract") != expected_reference:
        raise SystemExit(f"{name}:canonical_lifecycle_contract")
    if "lifecycle_actions" in document or "lifecycle_transition_contract" in document:
        raise SystemExit(f"{name}:duplicate_lifecycle_contract")
for cell in matrix.get("cells", []):
    capability_id = cell.get("capability_id")
    language = cell.get("language")
    duplicate = sorted(key for key in cell if key.startswith("lifecycle_"))
    if duplicate:
        raise SystemExit(
            f"matrix:duplicate_lifecycle_claim:{capability_id}:{language}:{','.join(duplicate)}"
        )
actions = matrix_contract.get("actions")
if not isinstance(actions, list) or set(actions) != set(axon_vectors.get("action_contracts", {})):
    raise SystemExit("axon:canonical_lifecycle_actions")
for action in actions:
    capability = axon_matrix.get("capabilities", {}).get(action)
    if not isinstance(capability, dict):
        raise SystemExit(f"axon:missing_lifecycle_capability:{action}")
    for language, row in capability.get("languages", {}).items():
        if row.get("state") != "CutoverReady":
            raise SystemExit(f"axon:lifecycle_not_cutover_ready:{action}:{language}")

plain_helpers = {
    "canonical_invocation_bytes",
    "run_admission",
    "sign_invocation",
    "verify_invocation_signature",
    "verify_phase",
    "verify_signature",
    "axiom.canonical_invocation_bytes",
    "axiom.sign_invocation",
    "axiom.verify_invocation_signature",
    "admission.run_admission",
    "admission.verify_phase",
    "admission.verify_signature",
}
fallback_signer_helpers = {
    "default_auth_for_subject",
    "GeneratedSubjectAuth",
    "generate_private_agent_auth",
    "generate_private_hub_auth",
    "generate_subject_auth",
    "DefaultAuthForSubject",
    "GenerateSubjectAuth",
    "ProcessLocalSigner",
    "PrivateKeyAuthenticator",
    "runtime_admin.GeneratedSubjectAuth",
    "runtime_admin.generate_private_agent_auth",
    "runtime_admin.generate_private_hub_auth",
    "runtime_admin.generate_subject_auth",
}
for section in ("languages", "members"):
    graph = manifest.get(section, {})
    for language, values in graph.items():
        leaked = sorted(plain_helpers & set(values))
        if leaked:
            raise SystemExit(f"canonical_plain_helper_leak:{language}:{section}:{','.join(leaked)}")
        fallback_leaked = sorted(fallback_signer_helpers & set(values))
        if fallback_leaked:
            raise SystemExit(
                f"fallback_signer_helper_leak:{language}:{section}:{','.join(fallback_leaked)}"
            )

quarantine = manifest.get("non_canonical", {})
metadata = manifest.get("legacy_quarantine", {})
for section in ("languages", "members"):
    graph = quarantine.get(section, {})
    for language, values in graph.items():
        legacy_plain = sorted(plain_helpers & set(values))
        if legacy_plain:
            raise SystemExit(
                f"plain_helper_legacy_export:{language}:{section}:{','.join(legacy_plain)}"
            )
for section in ("languages", "members"):
    graph = quarantine.get(section, {})
    for language, values in graph.items():
        for helper in sorted(fallback_signer_helpers & set(values)):
            reason = metadata.get(section, {}).get(language, {}).get(helper, {}).get("reason", "")
            if "Process-local signer fallback" not in reason:
                raise SystemExit(f"fallback_signer_reason_not_bound:{section}:{language}:{helper}")
PY
}

check_lifecycle_evidence_freshness_contract() {
  local checker="$AXON_ROOT/scripts/checks/check_lifecycle_convergence_contract.sh"
  if [[ ! -x "$checker" ]]; then
    fail "Axon lifecycle freshness checker is missing or not executable: $checker"
  fi
  bash "$checker" --require-cutover-ready >/dev/null
}

check_active_source_contract() {
  if rg -n 'default_auth_for_subject' "$ROOT/src" "$ROOT/sdk" "$ROOT/include" \
    --glob '!sdk/node/node_modules/**' \
    --glob '!sdk/conformance/**' \
    --glob '!target/**' \
    --glob '!sdk/go/internal/axonpb/**' \
    --glob '!sdk/python/easynet_sdk/_axon_pb/**'; then
    fail "process-local fallback signer path is present"
  fi

  if rg -n '\b(MissionState|MissionControl)\b' "$ROOT/src" "$ROOT/sdk" "$ROOT/include" \
    --glob '!sdk/node/node_modules/**' \
    --glob '!target/**' \
    --glob '!sdk/go/internal/axonpb/**' \
    --glob '!sdk/python/easynet_sdk/_axon_pb/**' \
    --glob '!src/eal/**'; then
    fail "Mission/EAL state leaked outside daemon-owned execution boundary"
  fi
}

check_sdk_product_neutrality_contract() {
  bash "$ROOT/tools/scripts/check-sdk-product-neutrality.sh" >/dev/null
}

check_python_sdk_bytecode_index_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local python_sdk="$cli_root/sdk/python/easynet_sdk"
  [[ -d "$python_sdk" ]] || return 0

  if git -C "$cli_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    local tracked
    tracked="$(
      git -C "$cli_root" ls-files sdk/python/easynet_sdk 2>/dev/null \
        | grep -E '(^|/)(__pycache__/.*|[^/]+\.pyc$)' \
        || true
    )"
    if [[ -n "$tracked" ]]; then
      printf '%s\n' "$tracked" >&2
      fail "Python SDK tracks generated bytecode"
    fi
  fi
}

check_sdk_root_runtime_description_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"

  "$PYTHON_BIN" - "$cli_root" <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
files = [
    root / "sdk/go/doc.go",
    root / "sdk/go/README.md",
    root / "sdk/python/easynet_sdk/__init__.py",
    root / "sdk/python/README.md",
]
forbidden = (
    "canonical EasyNet runtime SDK",
    "Product-neutral EasyNet runtime SDK",
    "product-neutral EasyNet runtime SDK",
    "EasyNet runtime SDK",
)

for path in files:
    if not path.exists():
        continue
    text = path.read_text(encoding="utf-8")
    for token in forbidden:
        if token in text:
            raise SystemExit(
                f"sdk_root_runtime_description_product_named:{path.relative_to(root)}:{token}"
            )
PY
}

check_go_sdk_public_ura_alias_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go_sdk="$cli_root/sdk/go"
  [[ -d "$go_sdk" ]] || return 0

  if rg -n '\btype\s+Ura\s*=' "$go_sdk" \
    --glob '!internal/axonpb/**' \
    --glob '!**/*_test.go'; then
    fail "Go SDK preserves retired Ura compatibility alias; canonical public API must expose URA only"
  fi
  if [[ -f "$cli_root/sdk/conformance/canonical-public-api.json" ]] \
    && rg -n '"Ura"' "$cli_root/sdk/conformance/canonical-public-api.json"; then
    fail "canonical public API inventory preserves retired Go Ura alias"
  fi
  if [[ -f "$cli_root/sdk/conformance/sdk-parity-matrix.json" ]] \
    && rg -n '"item": "Ura"' "$cli_root/sdk/conformance/sdk-parity-matrix.json"; then
    fail "SDK parity matrix preserves retired Go Ura alias evidence"
  fi
}

check_go_sdk_runtime_resource_namespace_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go_sdk="$cli_root/sdk/go"
  [[ -d "$go_sdk" ]] || return 0
  local namespace="$go_sdk/resource_namespace.go"
  [[ -f "$namespace" ]] || return 0

  if rg -n 'productResource|EasyNet'\''s provider namespace|product namespace' "$namespace" "$go_sdk/ura.go"; then
    fail "Go SDK root resource namespace projection preserves product-shaped vocabulary"
  fi
  if ! rg -q 'runtimeResourceNamespaces' "$namespace"; then
    fail "Go SDK resource namespace allowlist must be named as runtime state"
  fi
  if ! rg -q 'func runtimeResourceURA' "$namespace"; then
    fail "Go SDK resource URA helper must be named runtimeResourceURA"
  fi
  if ! rg -q 'func projectRuntimeResourcePath' "$namespace"; then
    fail "Go SDK parsed resource projection must be named projectRuntimeResourcePath"
  fi
}

check_python_sdk_runtime_addressing_kind_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local addressing="$cli_root/sdk/python/easynet_sdk/axon_addressing.py"
  [[ -f "$addressing" ]] || return 0

  if rg -n '_product_ura_kind|_product_ability_owner_kind' "$addressing"; then
    fail "Python SDK Addressing root preserves product-shaped kind projection helpers"
  fi
  if ! rg -q 'def _runtime_ura_kind\(canonical_kind: str\) -> str:' "$addressing"; then
    fail "Python SDK Addressing must name URA kind projection as runtime state"
  fi
  if ! rg -q 'def _runtime_ability_owner_kind\(canonical_kind: str\) -> str:' "$addressing"; then
    fail "Python SDK Addressing must name ability owner projection as runtime state"
  fi
}

check_advertise_agent_ingress_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local wrappers="$cli_root/src/daemon/invocation/dispatch/federation_wrappers.rs"
  [[ -f "$wrappers" ]] || return 0

  "$PYTHON_BIN" - "$wrappers" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
match = re.search(
    r"#\[derive\([^\]]*Deserialize[^\]]*\)\]\s*"
    r"#\[serde\(deny_unknown_fields\)\]\s*"
    r"pub struct AdvertiseAgentRequest\s*\{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if match is None:
    raise SystemExit("advertise_agent_request_not_strict")
body = match.group("body")
if "pub signing_authority: AdvertiseSigningAuthorityRequest" not in body:
    raise SystemExit("advertise_agent_signing_authority_not_required")
if "host_ura: Option" in body or re.search(r"\bpub\s+host_ura\b", body):
    raise SystemExit("advertise_agent_retired_host_ura_field")
if "self.host_ura" in text:
    raise SystemExit("advertise_agent_host_ura_fallback")
for test in (
    "advertise_agent_request_rejects_retired_top_level_host_ura",
    "advertise_agent_request_requires_signing_authority",
):
    if test not in text:
        raise SystemExit(f"missing_advertise_agent_negative_test:{test}")
PY
}

check_agent_start_model_intent_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local lifecycle="$cli_root/src/daemon/ability/builtins/agents/lifecycle.rs"
  [[ -f "$lifecycle" ]] || return 0

  "$PYTHON_BIN" - "$lifecycle" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
for retired in (
    'unwrap_or_else(|| args.get("model").is_some())',
    'unwrap_or(args.get("model").is_some())',
):
    if retired in text:
        raise SystemExit("agent_start_model_present_inferred_from_model")
if '"dependentRequired"' not in text or '"model": ["model_present"]' not in text:
    raise SystemExit("agent_start_schema_does_not_require_model_present_with_model")
if "agent.start: `model_present` is required when `model` is supplied" not in text:
    raise SystemExit("agent_start_missing_model_present_error_absent")
if "start_agent_rejects_model_without_explicit_model_present_intent" not in text:
    raise SystemExit("missing_agent_start_model_present_negative_test")
PY
}

check_invocation_history_get_key_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local history="$cli_root/src/daemon/ability/builtins/governance/invocation_history.rs"
  [[ -f "$history" ]] || return 0

  "$PYTHON_BIN" - "$history" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()

get_history = re.search(
    r"fn get_history\(&self, args: Value\) -> anyhow::Result<Value> \{(?P<body>.*?)\n    \}\n\n    fn get_record",
    text,
    re.DOTALL,
)
if get_history is None:
    raise SystemExit("invocation_history_get_not_found")
get_history_body = get_history.group("body")
for retired in (
    'key.get("attempt_id")',
    "InvocationAttemptLedger::open",
    '"diagnostic_record"',
):
    if retired in get_history_body:
        raise SystemExit(f"invocation_history_get_retired_attempt_path:{retired}")

key_schema = re.search(
    r"fn key_schema\(\) -> Value \{(?P<body>.*?)\n\}\n\nfn filter_schema",
    text,
    re.DOTALL,
)
if key_schema is None:
    raise SystemExit("invocation_history_key_schema_not_found")
key_schema_body = key_schema.group("body")
if '"attempt_id"' in key_schema_body:
    raise SystemExit("invocation_history_key_schema_exposes_attempt_id")
for required in ('"ura"', '"request_id"', '"trace_id"'):
    if required not in key_schema_body:
        raise SystemExit(f"invocation_history_key_schema_missing:{required}")
for test in (
    "history_key_schema_excludes_attempt_id",
    "get_history_rejects_attempt_id_key",
):
    if test not in text:
        raise SystemExit(f"missing_invocation_history_get_negative_test:{test}")
PY
}

check_invocation_history_ledger_ura_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local history="$cli_root/src/daemon/ability/builtins/governance/invocation_history.rs"
  [[ -f "$history" ]] || return 0

  "$PYTHON_BIN" - "$history" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
production = text.split("#[cfg(test)]", 1)[0]

ledger = re.search(
    r"fn ledger_resource_ura\(\) -> (?P<ret>[^\{]+)\{(?P<body>.*?)\n\}\n\nfn ledger_resource_ura_from_host_device_agent_ura",
    production,
    re.S,
)
if ledger is None:
    raise SystemExit("invocation_history_ledger_ura_function_not_found")
if "anyhow::Result<Option<String>>" not in ledger.group("ret"):
    raise SystemExit("invocation_history_ledger_ura_not_fallible")
ledger_body = ledger.group("body")
if "load_hosted_identity_status().ok()" in ledger_body:
    raise SystemExit("invocation_history_ledger_ura_aggregate_load_collapsed")
if "invocation.history ledger owner projection unavailable" not in ledger_body:
    raise SystemExit("invocation_history_ledger_ura_missing_projection_context")

projection = re.search(
    r"fn ledger_resource_ura_from_host_device_agent_ura\([^)]*\) -> (?P<ret>[^\{]+)\{(?P<body>.*?)\n\}\n\nfn fetch_records_from_path",
    production,
    re.S,
)
if projection is None:
    raise SystemExit("invocation_history_ledger_ura_projection_helper_not_found")
if "anyhow::Result<Option<String>>" not in projection.group("ret"):
    raise SystemExit("invocation_history_ledger_ura_projection_helper_not_fallible")
projection_body = projection.group("body")
for required in (
    "invalid host_device_agent_ura",
    "return Ok(None);",
    "resource_dot_ura",
):
    if required not in projection_body:
        raise SystemExit(f"invocation_history_ledger_ura_projection_missing:{required}")
if ".ok()?" in projection_body or "parse_ura(host_device_agent_ura).ok()" in projection_body:
    raise SystemExit("invocation_history_ledger_ura_parse_collapsed")

if "ledger_resource_ura_projection_distinguishes_unjoined_from_invalid_identity" not in text:
    raise SystemExit("missing_invocation_history_ledger_ura_projection_test")
PY
}

check_core_ura_realm_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local core="$cli_root/src/core/ura/mod.rs"
  local keyring_abilities="$cli_root/src/daemon/keyring/abilities.rs"
  local keyring_resolver="$cli_root/src/daemon/keyring/resolver.rs"
  local runtime_trust="$cli_root/src/daemon/invocation/admission/runtime_trust.rs"
  local register_pubkey="$cli_root/src/daemon/invocation/admission/register_device_pubkey.rs"
  local federated_resolver="$cli_root/src/daemon/invocation/admission/federated_key_resolver.rs"

  [[ -f "$core" ]] || fail "canonical core URA facade is missing: ${core#$cli_root/}"
  [[ -f "$keyring_abilities" ]] || fail "keyring abilities source is missing: ${keyring_abilities#$cli_root/}"
  [[ -f "$keyring_resolver" ]] || fail "keyring federated user resolver source is missing: ${keyring_resolver#$cli_root/}"
  [[ -f "$runtime_trust" ]] || fail "runtime trust source is missing: ${runtime_trust#$cli_root/}"
  [[ -f "$register_pubkey" ]] || fail "register-device pubkey source is missing: ${register_pubkey#$cli_root/}"
  [[ -f "$federated_resolver" ]] || fail "federated key resolver source is missing: ${federated_resolver#$cli_root/}"

  if ! rg -q 'pub fn realm_from_ura\(ura: &str\) -> Option<String>' "$core"; then
    fail "core URA facade must expose generic realm_from_ura projection"
  fi
  if ! rg -q 'pub fn user_realm_from_ura\(ura: &str\) -> Option<String>' "$core"; then
    fail "core URA facade must expose User-only user_realm_from_ura projection"
  fi
  if rg -n 'fn\s+parse_realm_from_user_ura' "$keyring_abilities" "$keyring_resolver"; then
    fail "keyring must not define duplicated user URA realm parser functions"
  fi
  if rg -n 'duplicated rather than re-exported|federated fallback' "$keyring_abilities" "$keyring_resolver"; then
    fail "keyring preserves retired duplicated/fallback URA projection vocabulary"
  fi
  if ! rg -q 'user_realm_from_ura\(' "$keyring_abilities"; then
    fail "keyring federated token issuance must consume core::ura::user_realm_from_ura"
  fi
  if ! rg -q 'user_realm_from_ura\(' "$keyring_resolver"; then
    fail "keyring federated user resolver must consume core::ura::user_realm_from_ura"
  fi
  if rg -n 'pub\(crate\)\s+fn\s+parse_realm_from_ura' "$runtime_trust" "$register_pubkey"; then
    fail "admission modules must not expose generic parse_realm_from_ura parser shims"
  fi
  if rg -n 'register_device_pubkey::parse_realm_from_ura' \
    "$federated_resolver" \
    "$cli_root/src/daemon/invocation/bidi/bidi_dispatcher.rs" \
    "$cli_root/src/daemon/invocation/admission/peer_envelope_signer.rs" \
    "$cli_root/src/daemon/invocation/admission/admission_facade.rs" \
    "$cli_root/src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"; then
    fail "runtime/admission callers must not depend on register-device parser shims"
  fi
}

check_federation_realm_resolver_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local resolver="$cli_root/src/daemon/federation/resolver.rs"
  [[ -f "$resolver" ]] || return 0

  "$PYTHON_BIN" - "$resolver" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
for required in (
    "pub enum RealmResolutionError",
    "EmptyRealm",
    "UnsupportedBareRealm { realm: String }",
    "impl std::error::Error for RealmResolutionError",
    "pub fn resolve(realm: &str, cfg: &ResolverConfig) -> Result<RealmResolution, RealmResolutionError>",
    "let realm = realm.trim();",
    "return Err(RealmResolutionError::EmptyRealm)",
    "Err(RealmResolutionError::UnsupportedBareRealm",
    "bare_realm_token_is_invalid_instead_of_local_fast_fallback",
    "empty_realm_is_invalid_instead_of_local_fast_fallback",
):
    if required not in text:
        raise SystemExit(f"federation_realm_resolver:missing:{required}")

for retired in (
    "anything else   → Local mode by default",
    "preserves pre-RFC-002 behaviour",
    "Bare token (legacy",
    "Backward-compat",
    "treat as Local-fast",
    "bare_token_falls_back_to_local",
):
    if retired in text:
        raise SystemExit(f"federation_realm_resolver:retired_fallback:{retired}")

fn = re.search(
    r"pub fn resolve\(realm: &str, cfg: &ResolverConfig\) -> Result<RealmResolution, RealmResolutionError> \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if fn is None:
    raise SystemExit("federation_realm_resolver:resolve_body_missing")
body = fn.group("body")
fqdn = body.find("if lower.contains('.')")
invalid = body.find("Err(RealmResolutionError::UnsupportedBareRealm")
if fqdn < 0 or invalid < 0 or invalid < fqdn:
    raise SystemExit("federation_realm_resolver:invalid_state_not_terminal_after_known_syntax")
tail = body[invalid:]
if "AdmissionMode::LocalFast" in tail or "RealmResolution {" in tail:
    raise SystemExit("federation_realm_resolver:bare_realm_tail_projects_resolution")
PY
}

check_resolve_key_request_dto_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local wire="$cli_root/src/daemon/federation/wire_contract.rs"
  local resolver="$cli_root/src/daemon/invocation/admission/federated_key_resolver.rs"
  local client_contract="$cli_root/src/daemon/federation/client/ability_contract.rs"
  local join="$cli_root/src/cli/commands/join.rs"

  [[ -f "$wire" ]] || fail "federation wire contract source is missing: ${wire#$cli_root/}"
  [[ -f "$resolver" ]] || fail "federated key resolver source is missing: ${resolver#$cli_root/}"
  [[ -f "$client_contract" ]] || fail "federation client ability contract source is missing: ${client_contract#$cli_root/}"
  [[ -f "$join" ]] || fail "join command source is missing: ${join#$cli_root/}"

  if ! rg -q 'impl ResolveKeyRequest' "$wire"; then
    fail "ResolveKeyRequest must own request construction and encoding"
  fi
  if ! rg -q 'pub fn new\(agent_ura: impl Into<String>\) -> Self' "$wire"; then
    fail "ResolveKeyRequest must expose a canonical constructor"
  fi
  if ! rg -q 'pub fn to_arguments_bytes\(&self\) -> serde_json::Result<Vec<u8>>' "$wire"; then
    fail "ResolveKeyRequest must expose deterministic argument encoding"
  fi
  if rg -n 'struct\s+ResolveKeyArgs|pub struct ResolveKeyArgs' "$client_contract"; then
    fail "federation client contract preserves duplicate ResolveKeyArgs DTO"
  fi
  if rg -n 'serde_json::json!\s*\(\s*\{\s*"agent_ura"\s*:' "$resolver"; then
    fail "federated key resolver must not hand-write resolve_key request JSON"
  fi
  if rg -n 'presented_pubkey_b64"\]\s*=|insert\("presented_pubkey_b64"' "$resolver"; then
    fail "federated key resolver must not mutate resolve_key presented-key JSON fields"
  fi
  if ! rg -q 'ResolveKeyRequest::new\(agent_ura\)' "$resolver"; then
    fail "federated key resolver must construct outbound requests through ResolveKeyRequest"
  fi
  if ! rg -q 'ResolveKeyRequest::new\(target\.hub_ura\.clone\(\)\)' "$join"; then
    fail "join command must construct resolve_key arguments through ResolveKeyRequest"
  fi
}

check_invocation_history_filter_scope_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local history="$cli_root/src/daemon/ability/builtins/governance/invocation_history.rs"
  [[ -f "$history" ]] || return 0

  "$PYTHON_BIN" - "$history" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
production = text.split("#[cfg(test)]", 1)[0]

for helper in (
    "fn canonical_ura(",
    "fn canonical_principal_ura(",
    "fn canonical_ability_ura(",
    "fn value_ability_ura_set(",
    "fn optional_principal_ura_filter_string(",
    "fn optional_ability_ura_filter_string(",
    "fn optional_state_filter_string(",
    "fn is_supported_history_state_filter(",
):
    if helper not in production:
        raise SystemExit(f"invocation_history_filter_scope_helper_missing:{helper}")

apply_filter = re.search(
    r"fn apply_filter_object\([^)]*\) -> anyhow::Result<InvocationLedgerQuery> \{(?P<body>.*?)\n\}\n\nfn validate_filter_keys",
    production,
    re.S,
)
if apply_filter is None:
    raise SystemExit("invocation_history_filter_scope_apply_filter_missing")
apply_body = apply_filter.group("body")
for required in (
    'optional_principal_ura_filter_string(object, "caller_ura")',
    'optional_ability_ura_filter_string(object, "ability_ura")',
    'optional_state_filter_string(object, "state")',
    "subject_filter_values(object)?",
):
    if required not in apply_body:
        raise SystemExit(f"invocation_history_filter_scope_apply_missing:{required}")
for retired in (
    'optional_filter_string(object, "caller_ura")',
    'optional_filter_string(object, "ability_ura")',
    'optional_filter_string(object, "state")',
):
    if retired in apply_body:
        raise SystemExit(f"invocation_history_filter_scope_apply_legacy:{retired}")

scoped_callee = re.search(
    r"fn scoped_callee_ura\([^)]*\) -> anyhow::Result<Option<String>> \{(?P<body>.*?)\n\}\n\nfn non_empty_str",
    production,
    re.S,
)
if scoped_callee is None:
    raise SystemExit("invocation_history_filter_scope_callee_missing")
scoped_body = scoped_callee.group("body")
for required in (
    'optional_principal_ura_filter_string(object, "callee_ura")',
    'optional_principal_ura_filter_string(object, "agent_ura")',
):
    if required not in scoped_body:
        raise SystemExit(f"invocation_history_filter_scope_callee_missing:{required}")

fetch_key = re.search(
    r"fn fetch_key_from_value\([^)]*\) -> anyhow::Result<InvocationLedgerFetchKey> \{(?P<body>.*?)\n\}\n\nfn apply_filter_object",
    production,
    re.S,
)
if fetch_key is None or "canonical_ura(" not in fetch_key.group("body"):
    raise SystemExit("invocation_history_filter_scope_key_ura_not_canonical")

subject_values = re.search(
    r"fn subject_filter_values\([^)]*\) -> anyhow::Result<Option<Vec<String>>> \{(?P<body>.*?)\n\}\n\nfn ledger_path_from_config",
    production,
    re.S,
)
if subject_values is None or "canonical_ura(" not in subject_values.group("body"):
    raise SystemExit("invocation_history_filter_scope_subjects_not_canonical")

for test in (
    "query_from_args_rejects_malformed_scope_uras_before_ledger_read",
    "query_from_args_rejects_malformed_key_ura_before_ledger_read",
    "list_history_rejects_malformed_ability_set_filters",
):
    if test not in text:
        raise SystemExit(f"missing_invocation_history_filter_scope_test:{test}")
PY
}

check_cli_invocation_history_read_model_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local invocation="$cli_root/src/cli/commands/groups/invocation.rs"
  [[ -f "$invocation" ]] || return 0

  "$PYTHON_BIN" - "$invocation" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
production = text.split("#[cfg(test)]", 1)[0]

for required in (
    "enum InvocationHistoryRead",
    "struct InvocationHistoryListQuery",
    "struct InvocationHistoryFilter",
    "enum InvocationHistoryKey",
    "fn invoke_invocation_history_read<T>(read: InvocationHistoryRead)",
    "InvocationHistoryRead::Path",
    "InvocationHistoryRead::List(InvocationHistoryListQuery::for_stats(args.limit))",
    "InvocationHistoryListQuery::from_list_args(args)",
    "InvocationHistoryKey::for_record_lookup(id)",
    "InvocationHistoryKey::TraceId(",
    "trace_id.to_string()",
):
    if required not in production:
        raise SystemExit(f"cli_invocation_history_read_model_missing:{required}")

for retired in (
    "fn history_list_args(",
    "fn insert_filter_value(",
    "fn history_key_for_id(",
    "invoke_invocation_ability(ABILITY_HISTORY_PATH, json!({}))",
    'invoke_invocation_ability(ABILITY_HISTORY_LIST, json!({ "limit": args.limit }))',
    'json!({ "key": history_key_for_id(id) })',
    'json!({ "key": { "trace_id": trace_id } })',
):
    if retired in production:
        raise SystemExit(f"cli_invocation_history_read_model_retired_json:{retired}")

if "LocalRuntimeStateReadIssuer::invoke(ability, args)" not in production:
    raise SystemExit("cli_invocation_history_read_model_not_using_runtime_state_read_issuer")
if re.search(r"\binvoke_local_ability\s*\(", production):
    raise SystemExit("cli_invocation_history_read_model_uses_generic_local_invoke")

for required_test in (
    "invocation_history_read_list_emits_explicit_ura_scope_fields",
    "invocation_history_read_list_omits_blank_filter_values",
    "invocation_history_read_projects_path_get_and_trace_queries",
    "invocation_history_stats_uses_list_query_without_scope_filter",
):
    if required_test not in text:
        raise SystemExit(f"cli_invocation_history_read_model_missing_test:{required_test}")
PY
}

check_local_runtime_state_read_subject_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local local_invoke="$cli_root/src/support/platform/local_invoke.rs"
  [[ -f "$local_invoke" ]] || fail "local invoke support source is missing: $local_invoke"

  "$PYTHON_BIN" - "$local_invoke" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

start = text.find("pub struct LocalRuntimeStateReadIssuer")
end = text.find("/// Invoke a canonical local target with public-ingress tuple facts.", start)
if start < 0 or end < 0:
    raise SystemExit("local_runtime_state_read_subject:issuer_section_missing")
body = text[start:end]

for required in (
    "struct LocalRuntimeStateReadSubject",
    "const RESOURCE_PATH: &'static str = \"runtime-state/read\"",
    "crate::daemon::persistence::config::load_credentials()",
    "crate::core::ura::resource_dot_ura(realm, &owner, Self::RESOURCE_PATH)",
    "runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity",
    "runtime_state_read_subject_rejects_missing_user_id_before_device_fallback",
):
    if required not in text:
        raise SystemExit(f"local_runtime_state_read_subject:missing:{required}")

if not re.search(r"\bcredentials\s*\.\s*user_id\s*\(\s*\)", body):
    raise SystemExit("local_runtime_state_read_subject:missing:credentials.user_id()")

for retired in (
    "local_daemon_ura()",
    "local_device_ura()",
    "control_discovery_daemon_ura()",
    "UNPAIRED_LOCAL_REALM",
    "UNPAIRED_LOCAL_DEVICE_ID",
    "resource/user.00000000-0000-0000-0000-000000000000",
):
    if retired in body:
        raise SystemExit(f"local_runtime_state_read_subject:retired_subject_fallback:{retired}")
PY
}

check_runtime_state_kind_required_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local config_rs="$cli_root/src/daemon/persistence/config.rs"
  [[ -f "$config_rs" ]] || fail "daemon persistence config source is missing: $config_rs"

  "$PYTHON_BIN" - "$config_rs" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

runtime_kind = re.search(
    r"(?P<derive>#\[derive\([^\]]*\)\]\s*)#\[serde\(rename_all = \"snake_case\"\)\]\s*pub enum RuntimeKind \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if runtime_kind is None:
    raise SystemExit("runtime_state_kind_required:runtime_kind_enum_missing")
if "Default" in runtime_kind.group("derive") or "#[default]" in runtime_kind.group("body"):
    raise SystemExit("runtime_state_kind_required:runtime_kind_default_retired")

state = re.search(
    r"pub struct RuntimeState \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if state is None:
    raise SystemExit("runtime_state_kind_required:runtime_state_missing")
state_body = state.group("body")
if "pub runtime_kind: RuntimeKind" not in state_body:
    raise SystemExit("runtime_state_kind_required:runtime_kind_field_missing")
runtime_kind_offset = state_body.find("pub runtime_kind: RuntimeKind")
field_prefix = state_body[max(0, runtime_kind_offset - 80):runtime_kind_offset]
if "#[serde(default)]" in field_prefix:
    raise SystemExit("runtime_state_kind_required:serde_default_retired")
if "runtime_state_defaults_to_daemon_when_kind_missing" in text:
    raise SystemExit("runtime_state_kind_required:legacy_default_test_retired")
for required in (
    "runtime_state_rejects_missing_runtime_kind",
    "missing field `runtime_kind`",
):
    if required not in text:
        raise SystemExit(f"runtime_state_kind_required:missing_test:{required}")
PY
}

check_daemon_config_mode_required_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local config_rs="$cli_root/src/daemon/persistence/daemon_config.rs"
  [[ -f "$config_rs" ]] || fail "daemon config source is missing: $config_rs"

  "$PYTHON_BIN" - "$config_rs" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
sync = re.search(
    r"fn sync_existing_device_config_toml\([^)]*\)\s*->\s*anyhow::Result<String>\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if sync is None:
    raise SystemExit("daemon_config_mode_required:sync_function_missing")
body = sync.group("body")
for retired in (
    '.unwrap_or("device")',
    '.unwrap_or_else(|| "device"',
    '.entry("daemon")',
    "or_insert_with",
):
    if retired in body:
        raise SystemExit(f"daemon_config_mode_required:retired_read_repair:{retired}")
for required in (
    '.get_mut("daemon")',
    "[daemon] is required in existing daemon-config.toml",
    "[daemon].mode is required in existing daemon-config.toml; refusing to infer device mode",
    "DaemonMode::parse_config_value(mode_raw)",
    "[daemon].mode has unsupported value",
):
    if required not in body:
        raise SystemExit(f"daemon_config_mode_required:missing:{required}")
for required_test in (
    "ensure_minimal_device_config_rejects_existing_config_without_explicit_mode",
    "ensure_minimal_device_config_rejects_existing_config_with_unknown_mode",
):
    if required_test not in text:
        raise SystemExit(f"daemon_config_mode_required:missing_test:{required_test}")
PY
}

check_chat_session_index_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local sessions_rs="$cli_root/src/daemon/persistence/chat_sessions.rs"
  [[ -f "$sessions_rs" ]] || fail "chat session persistence source is missing: $sessions_rs"

  "$PYTHON_BIN" - "$sessions_rs" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
index = re.search(
    r"pub struct SessionIndex \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if index is None:
    raise SystemExit("chat_session_index_schema:session_index_missing")
body = index.group("body")
for field in ("pub latest: String", "pub lifelong: String", "pub sessions: Vec<SessionDescriptor>"):
    if field not in body:
        raise SystemExit(f"chat_session_index_schema:missing_field:{field}")
if "#[serde(default)]" in body:
    raise SystemExit("chat_session_index_schema:field_default_retired")
for retired in (
    "index_without_lifelong_field_deserializes",
    "back-compat parse",
    "pre-existing index files",
    "serde default = empty string",
):
    if retired in text:
        raise SystemExit(f"chat_session_index_schema:retired_compat:{retired}")
for required in (
    "existing_index_without_lifelong_field_fails_closed",
    "missing field `lifelong`",
    "existing_index_without_latest_field_fails_closed",
    "missing field `latest`",
    "existing_index_without_sessions_field_fails_closed",
    "missing field `sessions`",
    "Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SessionIndex::default())",
):
    if required not in text:
        raise SystemExit(f"chat_session_index_schema:missing:{required}")
PY
}

check_local_agents_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local local_agents="$cli_root/src/daemon/persistence/local_agents.rs"
  [[ -f "$local_agents" ]] || fail "local agents persistence source is missing: $local_agents"

  "$PYTHON_BIN" - "$local_agents" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def struct_with_attrs(name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub struct {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"local_agents_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

file_attrs, file_body = struct_with_attrs("LocalAgentsFile")
entry_attrs, entry_body = struct_with_attrs("HostedAgentEntry")
for name, attrs in (
    ("LocalAgentsFile", file_attrs),
    ("HostedAgentEntry", entry_attrs),
):
    if "#[serde(deny_unknown_fields)]" not in attrs:
        raise SystemExit(f"local_agents_schema:{name}:missing_deny_unknown_fields")
for retired in ("#[serde(default)]",):
    if retired in file_body:
        raise SystemExit(f"local_agents_schema:LocalAgentsFile:retired_default:{retired}")
for field in (
    "pub host_device_agent_ura: String",
    "pub hosted_agents: Vec<HostedAgentEntry>",
):
    if field not in file_body:
        raise SystemExit(f"local_agents_schema:LocalAgentsFile:missing_field:{field}")
for field in (
    "pub profile: String",
    "pub name: String",
    "pub agent_ura: String",
    "pub signing_authority: String",
    "pub first_seen_at: String",
):
    if field not in entry_body:
        raise SystemExit(f"local_agents_schema:HostedAgentEntry:missing_field:{field}")
for retired in (
    "deserialize_tolerates_unknown_fields_for_forward_compat",
    "forward_compat",
    "Older daemons must still parse",
    "ignore unknown fields",
):
    if retired in text:
        raise SystemExit(f"local_agents_schema:retired_compat:{retired}")
for required in (
    "deserialize_rejects_unknown_fields",
    "unknown local-agents fields must fail closed",
    "deserialize_rejects_missing_host_device_agent_ura",
    "missing field `host_device_agent_ura`",
    "deserialize_rejects_missing_hosted_agents",
    "missing field `hosted_agents`",
    "return Ok(LocalAgentsFile::default())",
):
    if required not in text:
        raise SystemExit(f"local_agents_schema:missing:{required}")
PY
}

check_profile_store_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local profile="$cli_root/src/cli/commands/profile.rs"
  [[ -f "$profile" ]] || return 0

  "$PYTHON_BIN" - "$profile" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def struct_with_attrs(name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub\(crate\) struct {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"profile_store_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

entry_attrs, entry_body = struct_with_attrs("ProfileEntry")
store_attrs, store_body = struct_with_attrs("ProfileStore")
for name, attrs in (
    ("ProfileEntry", entry_attrs),
    ("ProfileStore", store_attrs),
):
    if "#[serde(deny_unknown_fields)]" not in attrs:
        raise SystemExit(f"profile_store_schema:{name}:missing_deny_unknown_fields")
if "Default" in entry_attrs:
    raise SystemExit("profile_store_schema:ProfileEntry:retired_default_derive")
for name, body in (
    ("ProfileEntry", entry_body),
    ("ProfileStore", store_body),
):
    if "#[serde(default" in body:
        raise SystemExit(f"profile_store_schema:{name}:retired_serde_default")
for field in (
    "pub current_profile: Option<String>",
    "pub profiles: BTreeMap<String, ProfileEntry>",
):
    if field not in store_body:
        raise SystemExit(f"profile_store_schema:ProfileStore:missing_field:{field}")
for field in (
    "pub profile_name: String",
    "pub realm_alias: String",
    "pub realm_id: Option<String>",
    "pub issuer: String",
    "pub login_hint: Option<String>",
    "pub subject: Option<String>",
    "pub credential_ref: Option<String>",
    "pub trust_anchor: Option<String>",
    "pub account_session: ProfileAccountSessionState",
    "pub device_membership: String",
):
    if field not in entry_body:
        raise SystemExit(f"profile_store_schema:ProfileEntry:missing_field:{field}")
if "impl Default for ProfileAccountSessionState" in text:
    raise SystemExit("profile_store_schema:retired_account_session_default_impl")
for required in (
    "missing_profile_store_is_fresh_install_empty_state",
    "return Ok(ProfileStore::default())",
    "existing_profile_store_requires_profiles_field",
    "missing field `profiles`",
    "existing_profile_store_rejects_unknown_fields",
    "unknown field `legacy_current_user`",
    "existing_profile_entry_requires_account_session",
    "missing field `account_session`",
    "existing_profile_entry_requires_device_membership",
    "missing field `device_membership`",
    "existing_profile_entry_rejects_unknown_fields",
    "unknown field `legacy_device_id`",
):
    if required not in text:
        raise SystemExit(f"profile_store_schema:missing:{required}")
PY
}

check_auth_session_owner_fact_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local auth="$cli_root/src/cli/commands/auth.rs"
  [[ -f "$auth" ]] || return 0

  "$PYTHON_BIN" - "$auth" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def item_with_attrs(kind: str, name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"{kind} {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"auth_session_owner_fact:{name}:missing")
    return match.group("attrs"), match.group("body")

session_attrs, session_body = item_with_attrs("pub struct", "AuthSession")
if "#[serde(deny_unknown_fields)]" not in session_attrs:
    raise SystemExit("auth_session_owner_fact:AuthSession:missing_deny_unknown_fields")
for required in (
    "pub token: String",
    "pub refresh_token: Option<String>",
    "pub hub_url: String",
    "pub email: String",
    "pub user_id: String",
    "pub nickname: Option<String>",
    "pub username: String",
):
    if required not in session_body:
        raise SystemExit(f"auth_session_owner_fact:AuthSession:missing_field:{required}")
for retired in (
    "pub user_id: Option<String>",
    "pub username: Option<String>",
    "#[serde(default",
):
    if retired in session_body:
        raise SystemExit(f"auth_session_owner_fact:AuthSession:retired_default:{retired}")

auth_attrs, auth_body = item_with_attrs("struct", "AuthResp")
user_attrs, user_body = item_with_attrs("struct", "UserResp")
refresh_attrs, refresh_body = item_with_attrs("struct", "RefreshResp")
if "user: UserResp" not in auth_body:
    raise SystemExit("auth_session_owner_fact:AuthResp:user_not_required")
for retired in (
    "user: Option<UserResp>",
    "#[serde(default)]\n    user",
):
    if retired in auth_body:
        raise SystemExit(f"auth_session_owner_fact:AuthResp:retired_user_fallback:{retired}")
for required in (
    "id: String",
    "username: String",
):
    if required not in user_body:
        raise SystemExit(f"auth_session_owner_fact:UserResp:missing_owner_fact:{required}")
for retired in (
    "id: Option<String>",
    "username: Option<String>",
    "#[serde(default)]\n    id",
    "#[serde(default)]\n    username",
):
    if retired in user_body:
        raise SystemExit(f"auth_session_owner_fact:UserResp:retired_optional_fact:{retired}")
if "token: String" not in refresh_body:
    raise SystemExit("auth_session_owner_fact:RefreshResp:missing_token")

refresh_fn = re.search(r"fn refresh_session\(session: &mut AuthSession\) -> anyhow::Result<\(\)> \{(?P<body>.*?)\n\}", text, re.S)
if refresh_fn is None:
    raise SystemExit("auth_session_owner_fact:refresh_session_missing")
if "let auth: RefreshResp" not in refresh_fn.group("body"):
    raise SystemExit("auth_session_owner_fact:refresh_uses_login_response")

for required in (
    "fn validated(self) -> anyhow::Result<Self>",
    "validate_non_blank(\"user_id\", &self.user_id)?",
    "ALL_ZERO_PRINCIPAL_ID",
    "auth session carries all-zero user_id",
    "validate_non_blank(\"username\", &self.username)?",
    "validate auth session before save",
    "auth_session_rejects_missing_user_id_owner_fact",
    "missing field `user_id`",
    "auth_session_rejects_missing_username_owner_fact",
    "missing field `username`",
    "auth_session_rejects_all_zero_user_id_owner_fact",
    "all-zero user_id",
    "auth_session_rejects_unknown_legacy_fields",
    "unknown field `legacy_subject`",
    "login_response_requires_user_owner_facts",
    "refresh_response_does_not_require_user_owner_facts",
):
    if required not in text:
        raise SystemExit(f"auth_session_owner_fact:missing:{required}")
PY
}

check_resources_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local resources="$cli_root/src/daemon/persistence/resources.rs"
  [[ -f "$resources" ]] || fail "resources persistence source is missing: $resources"

  "$PYTHON_BIN" - "$resources" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def struct_with_attrs(name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub struct {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"resources_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

file_attrs, file_body = struct_with_attrs("ResourcesFile")
entry_attrs, entry_body = struct_with_attrs("ResourceEntry")
for name, attrs in (
    ("ResourcesFile", file_attrs),
    ("ResourceEntry", entry_attrs),
):
    if "#[serde(deny_unknown_fields)]" not in attrs:
        raise SystemExit(f"resources_schema:{name}:missing_deny_unknown_fields")
if "#[serde(default)]" in file_body:
    raise SystemExit("resources_schema:ResourcesFile:retired_default")
if "#[serde(default)]" in entry_body:
    raise SystemExit("resources_schema:ResourceEntry:retired_default")
for field in (
    "pub resources: Vec<ResourceEntry>",
):
    if field not in file_body:
        raise SystemExit(f"resources_schema:ResourcesFile:missing_field:{field}")
for field in (
    "pub resource_ura: String",
    "pub owner_agent: String",
    "pub kind: ResourceType",
    "pub binding: ResourceBinding",
    "pub hardware_id: String",
    "pub display_name: String",
    "pub metadata: Value",
    "pub first_seen_at: String",
):
    if field not in entry_body:
        raise SystemExit(f"resources_schema:ResourceEntry:missing_field:{field}")
for retired in (
    "forward-compat: a future deployment",
    "invents `gpu` lands without a schema migration",
    "resources.json must tolerate missing owner_agent",
    "resources.json must tolerate missing metadata",
):
    if retired in text:
        raise SystemExit(f"resources_schema:retired_compat:{retired}")
for required in (
    "existing_resources_file_requires_resources_field",
    "missing field `resources`",
    "existing_resource_entry_requires_owner_agent_display_name_and_metadata",
    "missing field `owner_agent`",
    "missing field `display_name`",
    "missing field `metadata`",
    "existing_resources_file_rejects_unknown_fields",
    "unknown field `legacy_owner`",
    "unknown field `legacy_subject`",
    "return Ok(ResourcesFile::default())",
):
    if required not in text:
        raise SystemExit(f"resources_schema:missing:{required}")
PY
}

check_agent_spec_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local spec="$cli_root/src/core/agent/spec.rs"
  [[ -f "$spec" ]] || fail "agent spec source is missing: $spec"

  "$PYTHON_BIN" - "$spec" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

match = re.search(
    r"(?P<attrs>(?:#\[[^\n]+\]\n)*)pub struct AgentSpec \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if match is None:
    raise SystemExit("agent_spec_schema:AgentSpec:missing")
attrs = match.group("attrs")
body = match.group("body")
if "#[serde(deny_unknown_fields)]" not in attrs:
    raise SystemExit("agent_spec_schema:AgentSpec:missing_deny_unknown_fields")
for field in (
    "pub schema_version: Option<String>",
    "pub name: String",
    "pub runtime: RuntimeKind",
    "pub model: Option<String>",
    "pub mode: Option<String>",
    "pub system_prompt: Option<String>",
    "pub allowed_tools: Option<Vec<String>>",
    "pub description: Option<String>",
    "pub owner: Option<String>",
    "pub timeout_secs: Option<u64>",
    "pub env: BTreeMap<String, String>",
):
    if field not in body:
        raise SystemExit(f"agent_spec_schema:AgentSpec:missing_field:{field}")
for retired in (
    "unknown_top_level_keys_are_ignored_for_forward_compat",
    "unknown keys must be tolerated for forward compat",
    "forward compat",
    "operator later downgrades",
):
    if retired in text:
        raise SystemExit(f"agent_spec_schema:retired_compat:{retired}")
for required in (
    "unknown_top_level_keys_fail_closed",
    "unknown agent.toml fields must fail closed",
    "unknown field `runtmie`",
    "schema_version_absent_is_rejected",
    "schema_version_unknown_value_is_rejected",
):
    if required not in text:
        raise SystemExit(f"agent_spec_schema:missing:{required}")
PY
}

check_control_discovery_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local discovery="$cli_root/src/daemon/control/discovery.rs"
  [[ -f "$discovery" ]] || fail "control discovery source is missing: $discovery"

  "$PYTHON_BIN" - "$discovery" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def item_with_attrs(kind: str, name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub {kind} {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"control_discovery_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

for kind, name in (
    ("struct", "ControlDiscovery"),
    ("struct", "DaemonIdentity"),
    ("struct", "IpcVersionRange"),
):
    attrs, _ = item_with_attrs(kind, name)
    if "#[serde(deny_unknown_fields)]" not in attrs:
        raise SystemExit(f"control_discovery_schema:{name}:missing_deny_unknown_fields")

_, discovery_body = item_with_attrs("struct", "ControlDiscovery")
for field in (
    "pub socket_path: Option<PathBuf>",
    "pub pipe_name: Option<String>",
    "pub invocation_endpoint: Option<PathBuf>",
    "pub daemon_identity: Option<DaemonIdentity>",
    "pub pid: u32",
    "pub daemon_version: String",
    "pub supported_ipc_versions: IpcVersionRange",
    "pub capability_flags: Vec<String>",
    "pub pages_port: Option<u16>",
):
    if field not in discovery_body:
        raise SystemExit(f"control_discovery_schema:ControlDiscovery:missing_field:{field}")
for retired in (
    "adding a field later must use `#[serde(default)]`",
    "so old libs ignore it",
    "old libs ignore",
):
    if retired in text:
        raise SystemExit(f"control_discovery_schema:retired_compat:{retired}")
for required in (
    "control_discovery_rejects_unknown_fields",
    "unknown field `legacy_attach_hint`",
    "control_discovery_rejects_unknown_nested_identity_and_version_fields",
    "unknown field `legacy_role`",
    "unknown field `legacy_version`",
    "malformed_control_json_is_a_hard_error_not_silent_none",
    "read_missing_file_returns_none_not_error",
):
    if required not in text:
        raise SystemExit(f"control_discovery_schema:missing:{required}")
PY
}

check_control_frame_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local frames="$cli_root/src/daemon/control/frames.rs"
  [[ -f "$frames" ]] || fail "control frame source is missing: $frames"

  "$PYTHON_BIN" - "$frames" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def enum_with_attrs(name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub enum {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"control_frame_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

incoming_attrs, incoming_body = enum_with_attrs("IncomingFrame")
outgoing_attrs, outgoing_body = enum_with_attrs("OutgoingFrame")
for name, attrs in (
    ("IncomingFrame", incoming_attrs),
    ("OutgoingFrame", outgoing_attrs),
):
    if "deny_unknown_fields" not in attrs:
        raise SystemExit(f"control_frame_schema:{name}:missing_deny_unknown_fields")
for required in (
    "Subscribe {",
    "subscription_id: String",
    "ability: String",
    "args: Value",
    "Cancel { subscription_id: String }",
):
    if required not in incoming_body:
        raise SystemExit(f"control_frame_schema:IncomingFrame:missing:{required}")
if "#[serde(default)]\n        args: Value" in incoming_body:
    raise SystemExit("control_frame_schema:IncomingFrame:retired_default_args")
for required in (
    "Frame {",
    "Terminal {",
    "Error {",
    "subscription_id: Option<String>",
    "code: String",
    "message: String",
):
    if required not in outgoing_body:
        raise SystemExit(f"control_frame_schema:OutgoingFrame:missing:{required}")
for required in (
    "retired_product_incoming_variant_fails_to_parse",
    "incoming_frame_rejects_unknown_fields_and_missing_subscribe_args",
    "unknown field `legacy_route`",
    "missing field `args`",
    "outgoing_frame_rejects_unknown_fields",
    "unknown field `legacy_status`",
):
    if required not in text:
        raise SystemExit(f"control_frame_schema:missing:{required}")
PY
}

check_sdk_history_authority_subject_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go="$cli_root/sdk/go/authorized_runtime_session.go"
  local go_test="$cli_root/sdk/go/authorized_runtime_session_test.go"
  local py="$cli_root/sdk/python/easynet_sdk/authorized_runtime_session.py"
  local py_helper="$cli_root/sdk/python/easynet_sdk/_session_authority_subjects.py"
  local py_test="$cli_root/sdk/python/tests/test_authorized_runtime_session.py"

  "$PYTHON_BIN" - "$go" "$go_test" "$py" "$py_helper" "$py_test" <<'PY'
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_helper_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    return path.read_text() if path.exists() else ""

def require(path: Path, token: str, code: str) -> None:
    if token not in read(path):
        raise SystemExit(f"{code}:{path}:{token}")

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_history_authority_subject_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go = read(go_path)
if go:
    body = section(
        go,
        "func validateSessionHistorySessionBinding(",
        "func runtimeCallDetails(",
    )
    if "runtimeSessionAuthorityAdmitsSubject(authority, subjectURA)" not in body:
        raise SystemExit("sdk_go_history_authority_not_using_canonical_subject_admission")
    if "sessionHistoryAuthoritySubjectMatches(" in go:
        raise SystemExit("sdk_go_history_authority_exact_subject_helper_retired")
    require(
        go_test_path,
        "TestAuthorizedRuntimeSessionHistoryAllowsUserOwnedResourceSubjectBeforeReceiptProvider",
        "sdk_go_history_authority_owner_admission_test_missing",
    )
    require(
        go_test_path,
        "TestAuthorizedRuntimeSessionHistoryRejectsPathSubstringOwnerSubjectBeforeReceiptProvider",
        "sdk_go_history_authority_path_substring_test_missing",
    )
    require(
        go_test_path,
        "TestAuthorizedRuntimeSessionRejectsPathSubstringOwnerSubjectBeforeDispatch",
        "sdk_go_authority_path_substring_regression_test_missing",
    )

py = read(py_path)
if py:
    py_helper = read(py_helper_path)
    require(
        py_path,
        "from ._session_authority_subjects import session_authority_admits_subject",
        "sdk_python_authority_subject_shared_helper_import_missing",
    )
    require(
        py_helper_path,
        "def session_authority_admits_subject(",
        "sdk_python_authority_subject_shared_helper_missing",
    )
    for token in (
        "subject.components.get(\"owner_id\")",
        "parse_ura(subject_ura.strip())",
        "owner_id == f\"user.{owner_user_id}\"",
        "owner_id.startswith(\"agent.\")",
    ):
        if token not in py_helper:
            raise SystemExit(f"sdk_python_authority_subject_structured_owner_missing:{token}")
    for forbidden in (
        "f\"resource/user.{owner_user_id}/\" in subject_ura",
        "f\"resource/agent.{owner_user_id}.\" in subject_ura",
        "'resource/user.' in subject_ura",
        "'resource/agent.' in subject_ura",
    ):
        if forbidden in py or forbidden in py_helper:
            raise SystemExit("sdk_python_authority_subject_substring_expansion")
    body = section(
        py,
        "def _validate_session_history_authority_binding(",
        "def _validate_runtime_call_required(",
    )
    if "session_authority_admits_subject(authority, subject_ura)" not in body:
        raise SystemExit("sdk_python_history_authority_not_using_canonical_subject_admission")
    if "def _session_authority_admits_subject(" in py:
        raise SystemExit("sdk_python_history_authority_private_wrapper_retired")
    if "_session_history_authority_subject_matches(" in py:
        raise SystemExit("sdk_python_history_authority_exact_subject_helper_retired")
    require(
        py_test_path,
        "test_history_allows_user_owned_resource_subject_before_receipt_provider",
        "sdk_python_history_authority_owner_admission_test_missing",
    )
    require(
        py_test_path,
        "test_history_rejects_path_substring_owner_subject_before_receipt_provider",
        "sdk_python_history_authority_path_substring_test_missing",
    )
    require(
        py_test_path,
        "test_rejects_path_substring_owner_subject_before_dispatch",
        "sdk_python_authority_path_substring_regression_test_missing",
    )
PY
}

check_sdk_descriptor_resolution_error_vocabulary_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go="$cli_root/sdk/go/authorized_runtime_session.go"
  local go_test="$cli_root/sdk/go/authorized_runtime_session_test.go"
  local py="$cli_root/sdk/python/easynet_sdk/authorized_runtime_session.py"
  local py_test="$cli_root/sdk/python/tests/test_authorized_runtime_session.py"

  "$PYTHON_BIN" - "$go" "$go_test" "$py" "$py_test" <<'PY'
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_descriptor_resolution_error_vocabulary_source_missing:{path}")
    return path.read_text()

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_descriptor_resolution_error_vocabulary_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go = read(go_path)
go_body = section(go, "func descriptorResolutionFromError(", "func sessionIntentDetails(")
for legacy in ("ErrAbilityNotFound", "ErrNotFound"):
    if legacy in go_body:
        raise SystemExit(f"sdk_go_descriptor_resolution_legacy_not_found_projection:{legacy}")
for classifier in ("strings.Contains", "strings.ToLower", '"offline"'):
    if classifier in go_body:
        raise SystemExit(f"sdk_go_descriptor_resolution_message_classifier:{classifier}")
if "ErrDescriptorNotFound" not in go_body:
    raise SystemExit("sdk_go_descriptor_resolution_descriptor_not_found_missing")
if "ErrDescriptorOwnerOffline" not in go_body:
    raise SystemExit("sdk_go_descriptor_resolution_owner_offline_missing")
go_tests = read(go_test_path)
if "TestAuthorizedRuntimeDescriptorResolutionRequiresDescriptorVocabulary" not in go_tests:
    raise SystemExit("sdk_go_descriptor_resolution_vocabulary_test_missing")
if "TestAuthorizedRuntimeDescriptorResolutionRequiresTypedOwnerOffline" not in go_tests:
    raise SystemExit("sdk_go_descriptor_resolution_typed_owner_offline_test_missing")

py = read(py_path)
py_body = section(py, "def _descriptor_resolution_from_error(", "def _intent_details(")
for legacy in ("ErrorCode.ABILITY_NOT_FOUND", "ErrorCode.NOT_FOUND"):
    if legacy in py_body:
        raise SystemExit(f"sdk_python_descriptor_resolution_legacy_not_found_projection:{legacy}")
if '"offline" in text.lower()' in py_body or "'offline' in text.lower()" in py_body:
    raise SystemExit("sdk_python_descriptor_resolution_message_classifier")
if "ErrorCode.DESCRIPTOR_NOT_FOUND" not in py_body:
    raise SystemExit("sdk_python_descriptor_resolution_descriptor_not_found_missing")
if "ErrorCode.DESCRIPTOR_OWNER_OFFLINE" not in py_body:
    raise SystemExit("sdk_python_descriptor_resolution_owner_offline_missing")
py_tests = read(py_test_path)
if "test_descriptor_resolution_requires_descriptor_vocabulary" not in py_tests:
    raise SystemExit("sdk_python_descriptor_resolution_vocabulary_test_missing")
if "test_descriptor_resolution_requires_typed_owner_offline" not in py_tests:
    raise SystemExit("sdk_python_descriptor_resolution_typed_owner_offline_test_missing")
PY
}

check_sdk_ability_descriptor_not_found_vocabulary_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go="$cli_root/sdk/go/ability_descriptor.go"
  local go_test="$cli_root/sdk/go/ability_descriptor_test.go"
  local py="$cli_root/sdk/python/easynet_sdk/ability_descriptor.py"
  local py_test="$cli_root/sdk/python/tests/test_ability_descriptor.py"

  "$PYTHON_BIN" - "$go" "$go_test" "$py" "$py_test" <<'PY'
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_ability_descriptor_not_found_source_missing:{path}")
    return path.read_text()

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_ability_descriptor_not_found_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go = read(go_path)
go_body = section(go, "func abilityDescriptorNotFound(", "\n}")
if "ErrNotFound" in go_body:
    raise SystemExit("sdk_go_ability_descriptor_generic_not_found_projection")
if "ErrDescriptorNotFound" not in go_body:
    raise SystemExit("sdk_go_ability_descriptor_descriptor_not_found_missing")
go_tests = read(go_test_path)
if "TestRuntimeAbilityDescriptorProviderGetReportsDescriptorNotFound" not in go_tests:
    raise SystemExit("sdk_go_ability_descriptor_not_found_test_missing")

py = read(py_path)
py_body = section(py, "def _not_found(", "\n\n")
if "ErrorCode.NOT_FOUND" in py_body:
    raise SystemExit("sdk_python_ability_descriptor_generic_not_found_projection")
if "ErrorCode.DESCRIPTOR_NOT_FOUND" not in py_body:
    raise SystemExit("sdk_python_ability_descriptor_descriptor_not_found_missing")
py_tests = read(py_test_path)
if "test_runtime_ability_descriptor_provider_get_reports_descriptor_not_found" not in py_tests:
    raise SystemExit("sdk_python_ability_descriptor_not_found_test_missing")
PY
}

check_sdk_runtime_identity_signer_not_found_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go="$cli_root/sdk/go/runtime_identity.go"
  local go_test="$cli_root/sdk/go/runtime_identity_test.go"
  local py="$cli_root/sdk/python/easynet_sdk/providers/easynet/keyring.py"
  local py_test="$cli_root/sdk/python/tests/test_runtime_identity.py"

  "$PYTHON_BIN" - "$go" "$go_test" "$py" "$py_test" <<'PY'
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_runtime_identity_signer_not_found_source_missing:{path}")
    return path.read_text()

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_runtime_identity_signer_not_found_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go = read(go_path)
go_body = section(go, "func runtimeIdentityError(", "func (c runtimeKeyringClient) sign(")
if "ErrNotFound" not in go_body or "ErrCallerSignerUnavailable" not in go_body:
    raise SystemExit("sdk_go_runtime_identity_not_found_projection_missing")
if go.count("runtimeIdentityError(err)") < 3:
    raise SystemExit("sdk_go_runtime_identity_operations_not_using_projection")
go_tests = read(go_test_path)
if "TestRuntimeSigningIdentityProjectsMissingKeyAsCallerSignerUnavailable" not in go_tests:
    raise SystemExit("sdk_go_runtime_identity_not_found_test_missing")

py = read(py_path)
py_body = section(py, "def _runtime_identity_error(", "\n\n")
if "ErrorCode.NOT_FOUND" not in py_body or "ErrorCode.CALLER_SIGNER_UNAVAILABLE" not in py_body:
    raise SystemExit("sdk_python_runtime_identity_not_found_projection_missing")
py_tests = read(py_test_path)
if "test_rejection_projects_missing_runtime_identity_to_caller_signer_unavailable" not in py_tests:
    raise SystemExit("sdk_python_runtime_identity_not_found_test_missing")
PY
}

check_sdk_easynet_provider_identity_alias_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go="$cli_root/sdk/go/provider/easynet/identity.go"
  local go_test="$cli_root/sdk/go/provider/easynet/lifecycle_test.go"
  local py="$cli_root/sdk/python/easynet_sdk/providers/easynet/identity.py"
  local py_test="$cli_root/sdk/python/tests/test_runtime_environment.py"

  "$PYTHON_BIN" - "$go" "$go_test" "$py" "$py_test" <<'PY'
import re
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_easynet_provider_identity_alias_source_missing:{path}")
    return path.read_text()

def section(text: str, pattern: str, label: str) -> str:
    match = re.search(pattern, text, re.DOTALL)
    if match is None:
        raise SystemExit(f"sdk_easynet_provider_identity_alias_section_missing:{label}")
    return match.group("body")

go = read(go_path)
go_body = section(
    go,
    r"func providerRuntimeInstanceID\(decoded map\[string\]any\) \(string, error\) \{(?P<body>.*?)\n\}",
    "go_provider_runtime_instance_id",
)
for required in (
    'providerIdentityString(decoded, "node_id")',
    "retired node_id identity alias",
    'providerIdentityString(decoded, "device_id")',
    "return deviceID, nil",
):
    if required not in go_body:
        raise SystemExit(f"sdk_go_easynet_provider_identity_alias_required_missing:{required}")
for forbidden in (
    "return nodeID",
    "deviceID or nodeID",
    "device_id and node_id",
    "conflicting",
):
    if forbidden in go_body:
        raise SystemExit(f"sdk_go_easynet_provider_identity_alias_fallback_present:{forbidden}")
go_tests = read(go_test_path)
for required_test in (
    "TestProviderRejectsRetiredDaemonNodeIDAlias",
    "TestProviderRejectsNodeIDEvenWhenDeviceIDIsPresent",
):
    if required_test not in go_tests:
        raise SystemExit(f"sdk_go_easynet_provider_identity_alias_test_missing:{required_test}")
if "TestProviderMapsDaemonNodeIDAliasToCanonicalRuntimeIdentity" in go_tests:
    raise SystemExit("sdk_go_easynet_provider_identity_alias_mapping_test_present")

py = read(py_path)
py_body = section(
    py,
    r"def _runtime_instance_id\(raw: Mapping\[str, object\]\) -> str:\n(?P<body>.*?)(?=\n\ndef |\Z)",
    "python_runtime_instance_id",
)
for required in (
    '_text(raw, "node_id")',
    "retired node_id identity alias",
    '_text(raw, "device_id")',
    "return device_id",
):
    if required not in py_body:
        raise SystemExit(f"sdk_python_easynet_provider_identity_alias_required_missing:{required}")
for forbidden in (
    "return device_id or node_id",
    "device_id and node_id",
    "conflicting",
):
    if forbidden in py_body:
        raise SystemExit(f"sdk_python_easynet_provider_identity_alias_fallback_present:{forbidden}")
py_tests = read(py_test_path)
for required_test in (
    "test_easynet_provider_rejects_retired_daemon_node_id_alias",
    "test_easynet_provider_rejects_node_id_even_when_device_id_is_present",
):
    if required_test not in py_tests:
        raise SystemExit(f"sdk_python_easynet_provider_identity_alias_test_missing:{required_test}")
if "test_easynet_provider_maps_daemon_node_id_alias_to_canonical_projection" in py_tests:
    raise SystemExit("sdk_python_easynet_provider_identity_alias_mapping_test_present")
PY
}

check_sdk_python_transport_stream_event_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local transport="$cli_root/sdk/python/easynet_sdk/transport.py"
  local tests="$cli_root/sdk/python/tests/test_transport.py"

  "$PYTHON_BIN" - "$transport" "$tests" <<'PY'
import re
import sys
from pathlib import Path

transport_path, tests_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_python_transport_stream_event_projection_source_missing:{path}")
    return path.read_text()

transport = read(transport_path)
match = re.search(
    r"def _stream_event_dict\(event: StreamEvent\) -> dict\[str, object\]:\n(?P<body>.*?)(?=\n\ndef |\Z)",
    transport,
    re.DOTALL,
)
if match is None:
    raise SystemExit("sdk_python_transport_stream_event_projection_helper_missing")
body = match.group("body")
if '"payload_content_type": event.payload_content_type' not in body:
    raise SystemExit("sdk_python_transport_stream_event_payload_content_type_missing")
if '"content_type": event.payload_content_type' in body:
    raise SystemExit("sdk_python_transport_stream_event_legacy_content_type_projection")
tests = read(tests_path)
if 'self.assertNotIn("content_type", event)' not in tests:
    raise SystemExit("sdk_python_transport_stream_event_legacy_content_type_test_missing")
if 'self.assertIn("payload_content_type", event)' not in tests:
    raise SystemExit("sdk_python_transport_stream_event_payload_content_type_test_missing")
PY
}

check_sdk_python_invocation_result_adapter_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local transport="$cli_root/sdk/python/easynet_sdk/transport.py"
  local tests="$cli_root/sdk/python/tests/test_transport.py"

  "$PYTHON_BIN" - "$transport" "$tests" <<'PY'
import re
import sys
from pathlib import Path

transport_path, tests_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_python_invocation_result_adapter_projection_source_missing:{path}")
    return path.read_text()

transport = read(transport_path)
match = re.search(
    r"def _result_response_dict\(result: Mapping\[str, object\]\) -> dict\[str, object\]:\n(?P<body>.*?)(?=\n\ndef |\Z)",
    transport,
    re.DOTALL,
)
if match is None:
    raise SystemExit("sdk_python_invocation_result_adapter_projection_helper_missing")
body = match.group("body")
if "if result.get(\"ok\") is not True:" not in body or "raise SDKError(" not in body:
    raise SystemExit("sdk_python_invocation_result_adapter_failure_projection_missing")
if "return dict(result)" not in body:
    raise SystemExit("sdk_python_invocation_result_adapter_canonical_passthrough_missing")
for forbidden in (
    '"result_content_type"',
    '"result_base64"',
    '"result_json"',
    '"sdk_runtime_result"',
    '"state": _terminal_state_code',
    "_terminal_state_name(",
    "_terminal_state_code(",
    "_TERMINAL_STATE_CODES",
):
    if forbidden in body or forbidden in transport:
        raise SystemExit(f"sdk_python_invocation_result_adapter_legacy_wrapper:{forbidden}")
tests = read(tests_path)
for required in (
    'self.assertEqual(result["output_content_type"], "application/json")',
    'self.assertNotIn("result_content_type", result)',
    'self.assertNotIn("result_base64", result)',
    'self.assertNotIn("result_json", result)',
    'self.assertNotIn("sdk_runtime_result", result)',
):
    if required not in tests:
        raise SystemExit(f"sdk_python_invocation_result_adapter_test_missing:{required}")
PY
}

check_sdk_runtime_failure_code_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go_errors="$cli_root/sdk/go/errors.go"
  local go_direct="$cli_root/sdk/go/direct_runtime.go"
  local go_errors_test="$cli_root/sdk/go/errors_test.go"
  local go_direct_test="$cli_root/sdk/go/direct_runtime_codec_test.go"
  local py_errors="$cli_root/sdk/python/easynet_sdk/errors.py"
  local py_direct="$cli_root/sdk/python/easynet_sdk/direct_runtime.py"
  local py_errors_test="$cli_root/sdk/python/tests/test_errors.py"
  local py_direct_test="$cli_root/sdk/python/tests/test_direct_runtime.py"

  "$PYTHON_BIN" - \
    "$go_errors" \
    "$go_direct" \
    "$go_errors_test" \
    "$go_direct_test" \
    "$py_errors" \
    "$py_direct" \
    "$py_errors_test" \
    "$py_direct_test" <<'PY'
import re
import sys
from pathlib import Path

(
    go_errors_path,
    go_direct_path,
    go_errors_test_path,
    go_direct_test_path,
    py_errors_path,
    py_direct_path,
    py_errors_test_path,
    py_direct_test_path,
) = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    return path.read_text() if path.exists() else ""

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_runtime_failure_code_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go_errors = read(go_errors_path)
if go_errors:
    if re.search(r"func\s+runtimeFailureCode\s*\(\s*code\s+string\s*,\s*fallback\s+ErrorCode", go_errors):
        raise SystemExit("sdk_go_runtime_failure_code_fallback_parameter")
    body = section(go_errors, "func runtimeFailureCode(", "func isCanonicalExtensionErrorCode(")
    if "return fallback" in body:
        raise SystemExit("sdk_go_runtime_failure_code_return_fallback")
    if "return ErrProtocolMismatch" not in body:
        raise SystemExit("sdk_go_runtime_failure_code_missing_blank_protocol_mismatch")
    go_direct = read(go_direct_path)
    if "runtimeFailureCode(errorValue.GetCode()," in go_direct:
        raise SystemExit("sdk_go_direct_runtime_failure_code_call_uses_fallback")
    if re.search(r"func\s+directErrorStage\s*\([^)]*fallback\s+string", go_direct):
        raise SystemExit("sdk_go_direct_runtime_error_stage_fallback_parameter")
    if re.search(r"directErrorStage\s*\([^)]*,", go_direct):
        raise SystemExit("sdk_go_direct_runtime_error_stage_call_uses_fallback")
    direct_body = section(go_direct, "func directAxonFailure(", "func directErrorStage(")
    if "code == \"\"" in direct_body or "code == \"\" ||" in direct_body:
        raise SystemExit("sdk_go_direct_runtime_failure_code_preserves_empty_branch")
    go_tests = read(go_errors_test_path) + "\n" + read(go_direct_test_path)
    for token in (
        'ErrProtocolMismatch',
        '"   ":',
        "TestDirectAxonFailureProjectsMissingErrorCodeToProtocolMismatch",
        "TestDirectErrorStageUsesCanonicalProviderProjection",
    ):
        if token not in go_tests:
            raise SystemExit(f"sdk_go_runtime_failure_code_test_missing:{token}")

py_errors = read(py_errors_path)
if py_errors:
    py_body = section(py_errors, "def canonical_failure_code(", "def canonical_terminal_state_code(")
    if "return ErrorCode.ADMISSION_DENIED" in py_body:
        raise SystemExit("sdk_python_runtime_failure_code_blank_admission_fallback")
    if "return ErrorCode.PROTOCOL_MISMATCH" not in py_body:
        raise SystemExit("sdk_python_runtime_failure_code_missing_blank_protocol_mismatch")
    py_direct = read(py_direct_path)
    response_body = section(py_direct, "def _response_error_code(", "def _failure_code_value(")
    if "return ErrorCode.ADMISSION_DENIED" in response_body or "if code:" in response_body:
        raise SystemExit("sdk_python_direct_runtime_failure_code_preserves_empty_branch")
    py_tests = read(py_errors_test_path) + "\n" + read(py_direct_test_path)
    for token in (
        '"   ": ErrorCode.PROTOCOL_MISMATCH',
        'self.assertEqual(_response_error_code(""), ErrorCode.PROTOCOL_MISMATCH)',
    ):
        if token not in py_tests:
            raise SystemExit(f"sdk_python_runtime_failure_code_test_missing:{token}")
PY
}

check_sdk_direct_runtime_state_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go_direct="$cli_root/sdk/go/direct_runtime.go"
  local go_direct_test="$cli_root/sdk/go/direct_runtime_codec_test.go"
  local py_direct="$cli_root/sdk/python/easynet_sdk/direct_runtime.py"
  local py_direct_test="$cli_root/sdk/python/tests/test_direct_runtime.py"

  "$PYTHON_BIN" - "$go_direct" "$go_direct_test" "$py_direct" "$py_direct_test" <<'PY'
import re
import sys
from pathlib import Path

go_path, go_test_path, py_path, py_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_direct_runtime_state_projection_source_missing:{path}")
    return path.read_text()

def section(text: str, pattern: str, label: str) -> str:
    match = re.search(pattern, text, re.DOTALL)
    if match is None:
        raise SystemExit(f"sdk_direct_runtime_state_projection_section_missing:{label}")
    return match.group("body")

go = read(go_path)
go_state = section(
    go,
    r"func directStateName\(state axonpb\.InvocationState, stage string\) \(string, error\) \{(?P<body>.*?)\n\}",
    "go_direct_state_name",
)
if 'return "Unspecified"' in go_state:
    raise SystemExit("sdk_go_direct_runtime_state_unspecified_fallback")
if "directRuntimeProtocolError(stage" not in go_state:
    raise SystemExit("sdk_go_direct_runtime_state_not_fail_closed")
for retired_call in (
    "directStateName(response.GetState())",
    "directStateName(chunk.GetState())",
    "directStateName(receipt.GetState())",
    "directStateName(terminal.GetState())",
    "directStateName(down.GetReceipt().GetState())",
):
    if retired_call in go:
        raise SystemExit(f"sdk_go_direct_runtime_state_call_without_stage:{retired_call}")
if "stateName, err := directStateName(response.GetState(), \"direct_runtime.invoke\")" not in go:
    raise SystemExit("sdk_go_direct_runtime_unary_state_not_fallible")
if "stateName, err := directStateName(chunk.GetState(), \"direct_runtime.stream\")" not in go:
    raise SystemExit("sdk_go_direct_runtime_stream_state_not_fallible")
if "stateName, err := directStateName(receipt.GetState(), stage)" not in go:
    raise SystemExit("sdk_go_direct_runtime_receipt_state_not_fallible")
go_tests = read(go_test_path)
for required in (
    "TestDirectRuntimeUnaryRejectsUnsupportedInvocationState",
    "INVOCATION_STATE_UNSPECIFIED",
    "unsupported stream state error",
):
    if required not in go_tests:
        raise SystemExit(f"sdk_go_direct_runtime_state_test_missing:{required}")

py = read(py_path)
py_state = section(
    py,
    r"def _state_name\(value: int, stage: str\) -> str:\n(?P<body>.*?)(?=\n\ndef |\Z)",
    "python_state_name",
)
if '\"Unspecified\"' in py_state:
    raise SystemExit("sdk_python_direct_runtime_state_unspecified_fallback")
if "raise _direct_error(" not in py_state or "ErrorCode.PROTOCOL" not in py_state:
    raise SystemExit("sdk_python_direct_runtime_state_not_fail_closed")
if re.search(r"_state_name\([^,\n)]+\)", py):
    raise SystemExit("sdk_python_direct_runtime_state_call_without_stage")
for required in (
    '_state_name(response.state, "direct_runtime.invoke")',
    '_state_name(chunk.state, "direct_runtime.stream")',
    '_state_name(receipt.state, "direct_runtime.receipt")',
):
    if required not in py:
        raise SystemExit(f"sdk_python_direct_runtime_state_call_missing:{required}")
py_tests = read(py_test_path)
for required in (
    "test_direct_runtime_unary_rejects_unsupported_invocation_state",
    "test_direct_runtime_stream_rejects_unsupported_invocation_state",
    "INVOCATION_STATE_UNSPECIFIED",
):
    if required not in py_tests:
        raise SystemExit(f"sdk_python_direct_runtime_state_test_missing:{required}")
PY
}

check_sdk_direct_runtime_descriptor_not_found_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local go_direct="$cli_root/sdk/go/direct_runtime.go"
  local go_direct_test="$cli_root/sdk/go/direct_runtime_test.go"
  local py_direct="$cli_root/sdk/python/easynet_sdk/direct_runtime.py"
  local py_direct_test="$cli_root/sdk/python/tests/test_direct_runtime.py"

  "$PYTHON_BIN" - \
    "$go_direct" \
    "$go_direct_test" \
    "$py_direct" \
    "$py_direct_test" <<'PY'
import re
import sys
from pathlib import Path

go_direct_path, go_direct_test_path, py_direct_path, py_direct_test_path = map(Path, sys.argv[1:])

def read(path: Path) -> str:
    if not path.exists():
        raise SystemExit(f"sdk_direct_runtime_descriptor_not_found_source_missing:{path}")
    return path.read_text()

def section(text: str, start: str, end: str) -> str:
    offset = text.find(start)
    if offset < 0:
        raise SystemExit(f"sdk_direct_runtime_descriptor_not_found_missing_section:{start}")
    stop = text.find(end, offset + len(start))
    return text[offset : stop if stop >= 0 else len(text)]

go_direct = read(go_direct_path)
go_body = section(go_direct, "func directRuntimeGRPCError(", "func directRuntimeError(")
if re.search(r"case\s+codes\.NotFound:\s*code,\s*retry,\s*retryable\s*=\s*ErrAbilityNotFound", go_body):
    raise SystemExit("sdk_go_direct_runtime_not_found_legacy_ability_projection")
if "case codes.NotFound:" not in go_body or "ErrDescriptorNotFound" not in go_body:
    raise SystemExit("sdk_go_direct_runtime_not_found_descriptor_projection_missing")
go_tests = read(go_direct_test_path)
if "TestDirectRuntimeGRPCErrorProjectsProviderNotFoundAsDescriptorNotFound" not in go_tests:
    raise SystemExit("sdk_go_direct_runtime_not_found_descriptor_test_missing")

py_direct = read(py_direct_path)
py_body = section(py_direct, "def _grpc_error(", "def _direct_error(")
if re.search(r"grpc\.StatusCode\.NOT_FOUND:\s*\(\s*ErrorCode\.ABILITY_NOT_FOUND", py_body, re.S):
    raise SystemExit("sdk_python_direct_runtime_not_found_legacy_ability_projection")
if "grpc.StatusCode.NOT_FOUND" not in py_body or "ErrorCode.DESCRIPTOR_NOT_FOUND" not in py_body:
    raise SystemExit("sdk_python_direct_runtime_not_found_descriptor_projection_missing")
py_tests = read(py_direct_test_path)
if "test_direct_runtime_grpc_not_found_projects_descriptor_not_found" not in py_tests:
    raise SystemExit("sdk_python_direct_runtime_not_found_descriptor_test_missing")
PY
}

check_principal_lifecycle_cli_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local principal="$cli_root/src/cli/commands/groups/principal.rs"
  [[ -f "$principal" ]] || return 0

  "$PYTHON_BIN" - "$principal" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
if '.or_else(|| args.get("principal_ura"))' in text:
    raise SystemExit("principal_lifecycle_route_uses_top_level_fallback")
if "fn principal_get_request(principal_ura: &str) -> Value" not in text:
    raise SystemExit("principal_lifecycle_get_request_helper_missing")
extractor = re.search(
    r"fn principal_ability_realm_source<'a>\(ability: &str, args: &'a Value\) -> anyhow::Result<&'a str> \{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if extractor is None:
    raise SystemExit("principal_lifecycle_schema_aware_extractor_missing")
body = extractor.group("body")
for required in (
    "ability == routes::PRINCIPAL_ABILITY_GET",
    'args.get("principal_ura")',
    'args.pointer("/request/principal_ura")',
):
    if required not in body:
        raise SystemExit(f"principal_lifecycle_extractor_missing:{required}")
for test in (
    "principal_get_target_uses_explicit_top_level_read_schema",
    "principal_mutation_target_rejects_top_level_principal_ura_fallback",
    "principal_get_target_rejects_mutation_request_envelope",
):
    if test not in text:
        raise SystemExit(f"missing_principal_lifecycle_schema_test:{test}")
PY
}

check_principal_lifecycle_store_idempotency_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local lifecycle="$cli_root/src/daemon/invocation/admission/principal_lifecycle.rs"
  [[ -f "$lifecycle" ]] || return 0

  "$PYTHON_BIN" - "$lifecycle" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
store = re.search(r"(?s)struct PrincipalStore \{(?P<body>.*?)\n\}", text)
if store is None:
    raise SystemExit("principal_lifecycle_store_schema_missing")
store_body = store.group("body")
principals = re.search(r"(?s)((?:#\[[^\]]*\]\s*)*)principals: BTreeMap<String, PrincipalRecord>,", store_body)
if principals is None:
    raise SystemExit("principal_lifecycle_store_principals_missing")
if "#[serde(default" in principals.group(0):
    raise SystemExit("principal_lifecycle_store_principals_legacy_default")

record = re.search(r"(?s)struct PrincipalRecord \{(?P<body>.*?)\n\}", text)
if record is None:
    raise SystemExit("principal_lifecycle_record_schema_missing")
body = record.group("body")
command_log = re.search(r"(?s)((?:#\[[^\]]*\]\s*)*)command_log: BTreeMap<String, u64>,", body)
if command_log is None:
    raise SystemExit("principal_lifecycle_command_log_missing")
if "#[serde(default" in command_log.group(0):
    raise SystemExit("principal_lifecycle_command_log_legacy_default")

enrollment_proof = re.search(r"(?s)((?:#\[[^\]]*\]\s*)*)enrollment_proof: Option<PrincipalProofRef>,", body)
if enrollment_proof is None:
    raise SystemExit("principal_lifecycle_enrollment_proof_missing")
if "#[serde(default" in enrollment_proof.group(0):
    raise SystemExit("principal_lifecycle_enrollment_proof_legacy_default")
if "skip_serializing_if" in enrollment_proof.group(0):
    raise SystemExit("principal_lifecycle_enrollment_proof_skip_optional_fact")
if 'deserialize_with = "deserialize_required_option"' not in enrollment_proof.group(0):
    raise SystemExit("principal_lifecycle_enrollment_proof_not_required_option")

for field, ty in (
    ("consumed_recovery_proofs", "BTreeMap<String, i64>"),
    ("enrollments", "Vec<EnrollmentCapability>"),
    ("grants", "Vec<AuthorizationGrant>"),
):
    pattern = rf"(?s)((?:#\[[^\]]*\]\s*)*){field}: {re.escape(ty)},"
    match = re.search(pattern, body)
    if match is None:
        raise SystemExit(f"principal_lifecycle_collection_fact_missing:{field}")
    if "#[serde(default" in match.group(0):
        raise SystemExit(f"principal_lifecycle_collection_legacy_default:{field}")
    if field == "consumed_recovery_proofs" and "skip_serializing_if" in match.group(0):
        raise SystemExit("principal_lifecycle_consumed_recovery_proofs_skip_empty")

for required in (
    "existing_principal_store_requires_principals_fact",
    "missing field `principals`",
    "existing principal store without principals must fail closed",
    "principal_record_requires_enrollment_proof_fact",
    "missing field `enrollment_proof`",
    "principal record without enrollment_proof must fail closed",
    "principal_record_requires_lifecycle_collection_facts",
    "missing field `{field}`",
    "principal record without lifecycle collections must fail closed",
    "principal_record_requires_idempotency_command_log_fact",
    "missing field `command_log`",
    "principal record without command_log must fail closed",
):
    if required not in text:
        raise SystemExit(f"principal_lifecycle_command_log_test_missing:{required}")
PY
}

check_auth_agents_backend_shape_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local auth="$cli_root/src/cli/commands/auth.rs"
  [[ -f "$auth" ]] || return 0

  "$PYTHON_BIN" - "$auth" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text()
run_agents = re.search(
    r"pub fn run_agents\(args: AgentsArgs\) -> anyhow::Result<\(\)> \{(?P<body>.*?)\n\}\n\n// ── device remove",
    text,
    re.DOTALL,
)
if run_agents is None:
    raise SystemExit("auth_agents_run_not_found")
body = run_agents.group("body")
for retired in (
    'a.get("ura")',
    'a.get("name")',
    '.or_else(|| a.get("ura"))',
    '.or_else(|| a.get("name"))',
):
    if retired in body:
        raise SystemExit(f"auth_agents_retired_row_alias:{retired}")
if "AgentTableProjection::from_backend_row" not in body:
    raise SystemExit("auth_agents_table_projection_not_used")
projection = re.search(
    r"impl AgentTableProjection \{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if projection is None:
    raise SystemExit("auth_agents_table_projection_missing")
projection_body = projection.group("body")
for required in (
    '"agent_id"',
    '"display_name"',
    '"node_id"',
    '"skills"',
):
    if required not in projection_body:
        raise SystemExit(f"auth_agents_projection_missing:{required}")
for retired in ('"ura"', '"name"'):
    if retired in projection_body:
        raise SystemExit(f"auth_agents_projection_uses_retired_alias:{retired}")
for test in (
    "auth_agents_table_uses_canonical_backend_fields",
    "auth_agents_table_rejects_legacy_row_aliases",
):
    if test not in text:
        raise SystemExit(f"missing_auth_agents_projection_test:{test}")
PY
}

check_pages_identity_credentials_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local identity="$cli_root/src/daemon/ability/builtins/resources/pages/identity.rs"
  [[ -f "$identity" ]] || return 0
  local config="$cli_root/src/daemon/persistence/config.rs"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local smoke="$cli_root/src/bin/real-user-smoke.rs"
  local build="$cli_root/src/daemon/ability/catalog/build.rs"
  local api_key="$cli_root/src/daemon/ability/builtins/governance/api_key.rs"
  local openai="$cli_root/src/daemon/ability/builtins/integrations/openai_compat.rs"
  local assembly_tests="$cli_root/src/daemon/ability/catalog/assembly_tests.rs"
  local cli_pages="$cli_root/src/cli/commands/pages.rs"

  "$PYTHON_BIN" - "$identity" "$config" "$daemon" "$smoke" "$build" "$api_key" "$openai" "$assembly_tests" "$cli_pages" <<'PY'
import sys
from pathlib import Path

identity = Path(sys.argv[1]).read_text()
config = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""
daemon = Path(sys.argv[3]).read_text() if Path(sys.argv[3]).exists() else ""
smoke = Path(sys.argv[4]).read_text() if Path(sys.argv[4]).exists() else ""
build = Path(sys.argv[5]).read_text() if Path(sys.argv[5]).exists() else ""
api_key = Path(sys.argv[6]).read_text() if Path(sys.argv[6]).exists() else ""
openai = Path(sys.argv[7]).read_text() if Path(sys.argv[7]).exists() else ""
assembly_tests = Path(sys.argv[8]).read_text() if Path(sys.argv[8]).exists() else ""
cli_pages = Path(sys.argv[9]).read_text() if Path(sys.argv[9]).exists() else ""

if "pub fn from_env() -> Self" in identity:
    raise SystemExit("pages_identity_retains_infallible_from_env")
if "pub fn try_from_env() -> anyhow::Result<Self>" not in identity:
    raise SystemExit("pages_identity_missing_fallible_env_resolver")
if "pub fn user_root_identity(&self) -> anyhow::Result<Option<PagesUserRootIdentity>>" not in identity:
    raise SystemExit("pages_identity_missing_explicit_user_root_projection")
if "pub struct PagesUserRootIdentity" not in identity:
    raise SystemExit("pages_identity_user_root_projection_type_missing")
for retired in (
    "load_credentials()\n                    .ok()",
    "load_credentials().ok()",
    "credentials.as_ref().and_then(|c| c.username.clone())",
    ".and_then(|c| c.username.clone())",
    ".username.clone()",
    'parse::<u16>().ok()',
    ".and_then(|s| s.parse::<u16>().ok())",
):
    if retired in identity:
        raise SystemExit(f"pages_identity_retired_fallback:{retired}")
for required in (
    "load_credentials_optional()?",
    "pub(crate) fn pages_user_from_env_or_credentials(",
    "fn pages_realm_from_env_or_credentials(",
    "fn non_blank_env(key: &str) -> Option<String>",
    ".username_slug()",
    "credentials.join_receipt_hash().is_some()",
    "pages_listener_port_from_env()?",
    "EASYNET_PAGES_PORT must be greater than 0",
):
    if required not in identity:
        raise SystemExit(f"pages_identity_missing_fail_closed_path:{required}")
if "pub fn load_credentials_optional() -> anyhow::Result<Option<Credentials>>" not in config:
    raise SystemExit("credentials_optional_loader_missing")
if "PagesIdentity::try_from_env()" not in daemon:
    raise SystemExit("daemon_boot_not_using_fallible_pages_identity")
if smoke and "PagesIdentity::try_from_env()" not in smoke:
    raise SystemExit("real_user_smoke_not_using_fallible_pages_identity")
for test in (
    "pages_identity_missing_credentials_is_unpaired_state",
    "pages_identity_rejects_malformed_credentials_instead_of_defaulting",
    "pages_identity_rejects_invalid_port_instead_of_defaulting",
    "load_credentials_optional_rejects_malformed_existing_file",
    "pages_identity_trims_env_user_and_realm_overrides",
    "pages_identity_projects_federation_native_credentials_as_device_only",
    "pages_identity_rejects_blank_credential_username_instead_of_defaulting",
    "pages_identity_user_root_projection_requires_realm",
    "pages_identity_user_root_projection_accepts_complete_identity",
):
    if test not in identity and test not in config:
        raise SystemExit(f"missing_pages_identity_credentials_test:{test}")

if build:
    for required in (
        "pages_identity.user_root_identity()?",
        "api_key_ability::register(&mut reg, &user, &pages_realm)",
        "openai_compat_ability::set_identity(pages_identity.clone())?",
    ):
        if required not in build:
            raise SystemExit(f"pages_identity_consumer_missing_explicit_projection:{required}")
    for retired in (
        ".realm\n                .clone()\n                .unwrap_or_else(|| crate::core::ura::REALM_EASYNET.to_string())",
        ".realm\n        .as_deref()\n        .unwrap_or(crate::core::ura::REALM_EASYNET)",
        "api_key_ability::register(&mut reg, &user);",
        "openai_compat_ability::set_identity(pages_identity.clone());",
    ):
        if retired in build:
            raise SystemExit(f"pages_identity_consumer_retains_default_realm:{retired}")

if api_key:
    for required in (
        "pub fn register(reg: &mut AxonAbilityCatalog, user: &str, realm: &str)",
        "handle_create(&u1, &r1, args)",
        "handle_list(&u2, &r2, args)",
        "handle_revoke(&u3, &r3, args)",
        "create_stamps_registered_realm_without_product_default_lookup",
    ):
        if required not in api_key:
            raise SystemExit(f"api_key_user_root_realm_projection_missing:{required}")
    for retired in (
        "fn realm() -> String",
        "EASYNET_PAGES_REALM",
        "REALM_EASYNET",
        "pub fn handle_create(user: &str, args: Value)",
        "pub fn handle_list(user: &str, _args: Value)",
        "pub fn handle_revoke(user: &str, args: Value)",
        "pub fn register(reg: &mut AxonAbilityCatalog, user: &str)",
    ):
        if retired in api_key:
            raise SystemExit(f"api_key_retains_product_default_realm:{retired}")

if openai:
    for required in (
        "ProcessSingleton<Option<OpenAICompatIdentity>>",
        "OpenAICompatIdentity::from_pages_identity(identity)?",
        "fn openai_file_user_root_identity(",
        "openai_runtime_rejects_partial_user_identity_without_realm",
    ):
        if required not in openai:
            raise SystemExit(f"openai_user_root_realm_projection_missing:{required}")
    for retired in (
        "ProcessSingleton<OpenAICompatIdentity>",
        "fn compatibility_file_identity",
        "unwrap_or_else(|| crate::core::ura::REALM_EASYNET.to_string())",
        "unwrap_or(crate::core::ura::REALM_EASYNET)",
    ):
        if retired in openai:
            raise SystemExit(f"openai_retains_product_default_realm:{retired}")

if assembly_tests and "user_rooted_registry_rejects_paired_identity_without_realm" not in assembly_tests:
    raise SystemExit("pages_identity_consumer_missing_registry_rejection_test")

if cli_pages:
    for required in (
        "PagesIdentity::try_from_env()?",
        ".user_root_identity()?",
        "fn current_pages_user_root_identity() -> anyhow::Result<PagesUserRootIdentity>",
        "pages_cli_identity_projects_credentials_user_and_realm",
        "pages_cli_identity_rejects_env_user_without_realm",
        "pages_cli_identity_rejects_malformed_credentials_instead_of_defaulting",
    ):
        if required not in cli_pages:
            raise SystemExit(f"cli_pages_identity_projection_missing:{required}")
    for retired in (
        "fn current_user()",
        "fn current_realm()",
        "load_credentials()\n        .ok()",
        "load_credentials().ok()",
        "REALM_EASYNET",
        "or set EASYNET_PAGES_USER for dev rigs",
    ):
        if retired in cli_pages:
            raise SystemExit(f"cli_pages_identity_retired_fallback:{retired}")
PY
}

check_local_api_key_cache_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local api_key="$cli_root/src/daemon/ability/builtins/governance/api_key.rs"
  [[ -f "$api_key" ]] || return 0
  local llm_api="$cli_root/src/cli/commands/llm_api.rs"

  "$PYTHON_BIN" - "$api_key" "$llm_api" <<'PY'
import re
import sys
from pathlib import Path

api_key = Path(sys.argv[1]).read_text()
llm_api = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""

fn = re.search(
    r"pub fn read_local_default_token\(\) -> (?P<ret>[^{]+)\{(?P<body>.*?)\n\}",
    api_key,
    re.DOTALL,
)
if fn is None:
    raise SystemExit("local_api_key_cache_reader_missing")
if "anyhow::Result<Option<String>>" not in fn.group("ret"):
    raise SystemExit("local_api_key_cache_reader_not_fallible")
body = fn.group("body")
for retired in (
    "std::env::var(\"HOME\").ok()",
    "fs::read_to_string(path).ok()",
    "fs::read_to_string(&path).ok()",
    "toml::from_str(&text).ok()",
    "parsed.default_token",
):
    if retired in body and "Ok(Some(token.to_string()))" not in body:
        raise SystemExit(f"local_api_key_cache_retired_fallback:{retired}")
for required in (
    "local_default_token_path()?",
    "ErrorKind::NotFound",
    "return Ok(None)",
    "parse local API key cache",
    "blank default_token",
    "Ok(Some(token.to_string()))",
):
    if required not in body:
        raise SystemExit(f"local_api_key_cache_missing_fail_closed_path:{required}")
if "#[serde(deny_unknown_fields)]" not in api_key:
    raise SystemExit("local_api_key_cache_missing_unknown_field_rejection")
if "fn local_default_token_path() -> anyhow::Result<PathBuf>" not in api_key:
    raise SystemExit("local_api_key_cache_path_helper_missing")
if "pub fn write_local_default_token(token: &str) -> anyhow::Result<()>" not in api_key:
    raise SystemExit("local_api_key_cache_writer_missing")
if "let path = local_default_token_path()?" not in api_key:
    raise SystemExit("local_api_key_cache_writer_not_using_shared_path")
if llm_api:
    if "fn pick_token(arg: Option<String>) -> anyhow::Result<Option<String>>" not in llm_api:
        raise SystemExit("llm_api_pick_token_not_fallible")
    if "let token = pick_token(args.key)?" not in llm_api:
        raise SystemExit("llm_api_not_propagating_local_cache_error")
for test in (
    "missing_local_default_token_cache_is_no_default_token_state",
    "local_default_token_cache_rejects_malformed_toml",
    "local_default_token_cache_rejects_unknown_fields",
    "local_default_token_cache_rejects_blank_token",
):
    if test not in api_key:
        raise SystemExit(f"missing_local_api_key_cache_test:{test}")
PY
}

check_api_key_cli_identity_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local api_key_cli="$cli_root/src/cli/commands/api_key_cli.rs"
  [[ -f "$api_key_cli" ]] || return 0

  "$PYTHON_BIN" - "$api_key_cli" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
fn = re.search(
    r"fn current_user\(\) -> anyhow::Result<String> \{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if fn is None:
    raise SystemExit("api_key_cli_identity:current_user_missing")
body = fn.group("body")
for retired in (
    "load_credentials().ok()",
    ".and_then(|c| c.username)",
    ".and_then(|credentials| credentials.username)",
    "unwrap_or_default()",
):
    if retired in body:
        raise SystemExit(f"api_key_cli_identity:retired_credential_fallback:{retired}")
for required in (
    "load_credentials_optional()?",
    "credentials.username_slug()?",
    "no user identity bound to this daemon",
    "EASYNET_PAGES_USER",
    ".map(|s| s.trim().to_string())",
):
    if required not in body:
        raise SystemExit(f"api_key_cli_identity:missing_strict_path:{required}")
for test in (
    "current_user_accepts_explicit_dev_override",
    "current_user_reads_valid_paired_credentials",
    "current_user_reports_unpaired_only_when_credentials_file_is_absent",
    "current_user_rejects_malformed_existing_credentials",
    "current_user_rejects_credentials_without_username",
):
    if test not in text:
        raise SystemExit(f"api_key_cli_identity:missing_test:{test}")
PY
}

check_api_key_store_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local api_key="$cli_root/src/daemon/ability/builtins/governance/api_key.rs"
  [[ -f "$api_key" ]] || return 0

  "$PYTHON_BIN" - "$api_key" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

def struct_with_attrs(name: str) -> tuple[str, str]:
    pattern = (
        r"(?P<attrs>(?:#\[[^\n]+\]\n)*)"
        + rf"pub struct {name} \{{(?P<body>.*?)\n\}}"
    )
    match = re.search(pattern, text, re.S)
    if match is None:
        raise SystemExit(f"api_key_store_schema:{name}:missing")
    return match.group("attrs"), match.group("body")

entry_attrs, entry_body = struct_with_attrs("ApiKeyEntry")
store_attrs, store_body = struct_with_attrs("ApiKeyStore")
for name, attrs in (
    ("ApiKeyEntry", entry_attrs),
    ("ApiKeyStore", store_attrs),
):
    if "#[serde(deny_unknown_fields)]" not in attrs:
        raise SystemExit(f"api_key_store_schema:{name}:missing_deny_unknown_fields")

if "#[serde(default)]" in store_body:
    raise SystemExit("api_key_store_schema:ApiKeyStore:retired_default_keys")
if "pub keys: Vec<ApiKeyEntry>" not in store_body:
    raise SystemExit("api_key_store_schema:ApiKeyStore:missing_keys")
for field in (
    "pub id_prefix: String",
    "pub token_hash: String",
    "pub user_ura: String",
    "pub label: Option<String>",
    "pub created_at: u64",
    "pub revoked_at: Option<u64>",
    "pub last_used_at: Option<u64>",
):
    if field not in entry_body:
        raise SystemExit(f"api_key_store_schema:ApiKeyEntry:missing_field:{field}")

load_store = re.search(
    r"fn load_store\(\) -> anyhow::Result<ApiKeyStore> \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if load_store is None:
    raise SystemExit("api_key_store_schema:load_store:missing")
load_body = load_store.group("body")
for required in (
    "Err(error) if error.kind() == std::io::ErrorKind::NotFound",
    "return Ok(ApiKeyStore::default())",
    "parse API key store",
):
    if required not in load_body:
        raise SystemExit(f"api_key_store_schema:load_store:missing:{required}")
for retired in (
    "toml::from_str(&text).unwrap_or_default()",
    "toml::from_str(&text).unwrap_or_else",
    ".unwrap_or_default()",
    ".ok().unwrap_or_default()",
):
    if retired in load_body:
        raise SystemExit(f"api_key_store_schema:load_store:retired_compat:{retired}")

for required in (
    "missing_store_is_fresh_install_empty_state",
    "api_key_store_rejects_existing_file_without_keys",
    "missing field `keys`",
    "api_key_store_rejects_unknown_top_level_fields",
    "unknown field `legacy`",
    "api_key_store_rejects_unknown_entry_fields",
    "unknown field `legacy_scope`",
    "bearer_resolution_rejects_malformed_store_instead_of_unknown_token",
    "malformed credential authority must not be projected as unknown token",
):
    if required not in text:
        raise SystemExit(f"api_key_store_schema:missing:{required}")
PY
}

check_cli_credentials_optional_read_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local src_root="$cli_root/src"
  [[ -d "$src_root" ]] || return 0

  "$PYTHON_BIN" - "$src_root" <<'PY'
import sys
from pathlib import Path

src_root = Path(sys.argv[1])
retired = (
    "load_credentials().ok()",
    "load_credentials().ok()?",
    "user_ura().ok()",
    "if let Ok(user_ura) =",
)
violations = []
for path in src_root.rglob("*.rs"):
    text = path.read_text()
    for token in retired:
        if token in text:
            violations.append(f"{path.relative_to(src_root.parent)}:{token}")
if violations:
    raise SystemExit(
        "cli_credentials_optional_read_retired_fallback:" + ",".join(violations)
    )

identity = src_root / "cli" / "presentation" / "identity.rs"
if not identity.exists():
    raise SystemExit("cli_identity_projection_shared_projector_missing")
identity_text = identity.read_text()
for token in (
    "pub enum RuntimeUserBindingDisplayState",
    "pub struct RuntimeUserBindingDisplay",
    "pub fn runtime_user_binding_display(creds: &config::Credentials)",
    "RuntimeUserBindingDisplayState::Bound",
    "RuntimeUserBindingDisplayState::Unbound",
    "RuntimeUserBindingDisplayState::Invalid",
    "display_projects_bound_user_ura",
    "display_projects_unbound_federation_native_state",
    "display_projects_invalid_user_binding_state",
):
    if token not in identity_text:
        raise SystemExit(f"cli_identity_projection_shared_projector_missing:{token}")
for rel in (
    "cli/commands/status.rs",
    "cli/commands/auth.rs",
    "cli/presentation/banner.rs",
):
    path = src_root / rel
    if path.exists() and "runtime_user_binding_display" not in path.read_text():
        raise SystemExit(f"cli_identity_projection_surface_not_using_projector:{rel}")
PY
}

check_credentials_user_binding_validation_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local config="$cli_root/src/daemon/persistence/config.rs"
  [[ -f "$config" ]] || return 0

  "$PYTHON_BIN" - "$config" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
fn = re.search(
    r"fn validate_complete\(&self\) -> anyhow::Result<\(\)> \{(?P<body>.*?)\n    \}",
    text,
    re.DOTALL,
)
if fn is None:
    raise SystemExit("credentials_validate_complete_missing")
body = fn.group("body")
for required in (
    "pub enum RuntimeUserBinding",
    "Bound { user_ura: String }",
    "Unbound { reason: &'static str }",
    "pub fn runtime_user_binding(&self) -> anyhow::Result<RuntimeUserBinding>",
):
    if required not in text:
        raise SystemExit(f"credentials_user_binding_projection_missing:{required}")
for required in (
    "self.join_receipt_hash().is_none()",
    "self.username_slug()?",
    "self.user_id()?",
    "else if self.user_id.is_some()",
):
    if required not in body:
        raise SystemExit(f"credentials_user_binding_validation_missing:{required}")
for test in (
    "save_credentials_accepts_federation_join_receipt_without_user_binding",
    "save_credentials_rejects_join_receipt_with_all_zero_user_id",
    "load_credentials_rejects_join_receipt_with_all_zero_user_id",
    "runtime_user_binding_projects_bound_user_ura",
    "runtime_user_binding_makes_federation_native_device_only_explicit",
    "runtime_user_binding_rejects_blank_join_user_id_instead_of_hiding_it",
):
    if test not in text:
        raise SystemExit(f"credentials_user_binding_validation_missing_test:{test}")
PY
}

check_target_gate_credential_state_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local target_gate="$cli_root/src/daemon/invocation/admission/target_gate.rs"
  [[ -f "$target_gate" ]] || return 0

  "$PYTHON_BIN" - "$target_gate" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
for retired in (
    "load_credentials()\n        .ok()",
    "load_credentials().ok()",
    "creds.user_id().ok()",
    "user_id().ok()",
):
    if retired in text:
        raise SystemExit(f"target_gate_credential_state_retired_fallback:{retired}")
for required in (
    "enum LocalCredentialIdentityState",
    "Available(LocalCredentialIdentity)",
    "Unpaired",
    "Unavailable { reason: String }",
    "load_credentials_optional()?",
    "creds.user_id()?",
    "target_gate_credential_identity_load_failed",
    "local_agent_target_index_unpaired_credentials_do_not_match_targets",
    "local_agent_target_index_unavailable_credentials_fail_closed",
):
    if required not in text:
        raise SystemExit(f"target_gate_credential_state_missing:{required}")

start = text.find("fn local_runtime_authority_ura(")
end = text.find("// ── Route-outcome wire mapping", start)
if start < 0 or end < 0:
    raise SystemExit("target_gate_local_runtime_authority_missing")
body = text[start:end]
for retired in (
    ".map(crate::core::ura::hub_ura)",
    "local_runtime_authority_falls_back_to_hub_ura_without_daemon_identity",
):
    if retired in body or retired in text:
        raise SystemExit(f"target_gate_local_runtime_authority_retired_fallback:{retired}")
if "local_runtime_authority_rejects_session_realm_without_daemon_identity" not in text:
    raise SystemExit("target_gate_local_runtime_authority_missing_fail_closed_test")
PY
}

check_runtime_trust_revoke_credentials_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local invalidator="$cli_root/src/daemon/invocation/admission/runtime_trust_invalidator.rs"
  local dispatcher="$cli_root/src/daemon/invocation/dispatch/unary_dispatcher.rs"
  [[ -f "$invalidator" ]] || return 0

  "$PYTHON_BIN" - "$invalidator" "$dispatcher" <<'PY'
import re
import sys
from pathlib import Path

invalidator = Path(sys.argv[1]).read_text()
dispatcher = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""

fn = re.search(
    r"pub\(crate\) fn from_local_credentials\((?P<sig>.*?)\) -> (?P<ret>[^{]+)\{(?P<body>.*?)\n    \}",
    invalidator,
    re.DOTALL,
)
if fn is None:
    raise SystemExit("runtime_trust_local_credentials_projector_missing")
if "anyhow::Result<Option<Self>>" not in fn.group("ret"):
    raise SystemExit("runtime_trust_local_credentials_projector_not_fallible")
body = fn.group("body")
for retired in (
    "load_credentials().ok()",
    "load_credentials().ok()?",
    "credentials.user_ura().ok()",
):
    if retired in body:
        raise SystemExit(f"runtime_trust_projector_retired_fallback:{retired}")
for required in (
    "load_credentials_optional()?",
    "return Ok(None)",
    "Self::from_credentials(credentials, source).map(Some)",
):
    if required not in body:
        raise SystemExit(f"runtime_trust_projector_missing_fail_closed_path:{required}")
if "pub(crate) fn from_credentials(" not in invalidator:
    raise SystemExit("runtime_trust_projector_from_credentials_missing")
if "-> anyhow::Result<Self>" not in invalidator:
    raise SystemExit("runtime_trust_projector_from_credentials_not_fallible")
if "let current_user_ura = credentials.user_ura()?;" not in invalidator:
    raise SystemExit("runtime_trust_projector_user_ura_not_fail_closed")
for test in (
    "local_connection_state_projector_returns_none_when_credentials_missing",
    "local_connection_state_projector_rejects_malformed_credentials",
):
    if test not in invalidator:
        raise SystemExit(f"missing_runtime_trust_projector_test:{test}")

if dispatcher:
    preflight = dispatcher.find("let connection_state_projector =")
    mutation = dispatcher.find("handle_revoke_user_pubkey_with_outcome(")
    if preflight < 0:
        raise SystemExit("runtime_trust_revoke_preflight_missing")
    if mutation < 0:
        raise SystemExit("runtime_trust_revoke_mutation_missing")
    if preflight > mutation:
        raise SystemExit("runtime_trust_revoke_preflight_after_mutation")
    for required in (
        "RuntimeTrustConnectionStateProjector::from_local_credentials(\"daemon.runtime_trust\")",
        ".with_connection_state_projector(connection_state_projector)",
    ):
        if required not in dispatcher:
            raise SystemExit(f"runtime_trust_revoke_dispatch_missing:{required}")
    if "local credentials unavailable for runtime" not in dispatcher or "trust projection" not in dispatcher:
        raise SystemExit("runtime_trust_revoke_dispatch_missing:credential_projection_error")
PY
}

check_runtime_trust_user_key_inventory_scope_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local list_handler="$cli_root/src/daemon/invocation/admission/list_user_pubkeys.rs"
  local trust="$cli_root/src/daemon/invocation/admission/runtime_trust.rs"
  local contracts="$cli_root/src/daemon/ability/catalog/daemon_invocation_contracts.rs"
  local cli_user="$cli_root/src/cli/commands/user_signing_identity.rs"
  local doctor="$cli_root/src/cli/commands/doctor.rs"
  [[ -f "$list_handler" ]] || fail "identity.list_user_pubkeys handler is missing: $list_handler"
  [[ -f "$trust" ]] || fail "runtime trust aggregate is missing: $trust"
  [[ -f "$contracts" ]] || fail "daemon invocation contracts source is missing: $contracts"

  "$PYTHON_BIN" - "$list_handler" "$trust" "$contracts" "$cli_user" "$doctor" <<'PY'
import re
import sys
from pathlib import Path

handler = Path(sys.argv[1]).read_text(encoding="utf-8")
trust = Path(sys.argv[2]).read_text(encoding="utf-8")
contracts = Path(sys.argv[3]).read_text(encoding="utf-8")
cli_user = Path(sys.argv[4]).read_text(encoding="utf-8") if Path(sys.argv[4]).exists() else ""
doctor = Path(sys.argv[5]).read_text(encoding="utf-8") if Path(sys.argv[5]).exists() else ""

production_handler = handler.split("\n#[cfg(test)]\nmod tests", 1)[0]
if "struct ListArgs" not in production_handler or "user_ura: String" not in production_handler:
    raise SystemExit("runtime_trust_user_key_inventory:list_args_not_user_scoped")
if "agent_ura: String" in production_handler:
    raise SystemExit("runtime_trust_user_key_inventory:list_args_preserve_agent_ura")
if "#[serde(deny_unknown_fields)]" not in production_handler:
    raise SystemExit("runtime_trust_user_key_inventory:unknown_fields_not_rejected")
for required in (
    "fn required_user_ura(",
    "parse_ura(user_ura)",
    "URAKind::User",
    "identity.list_user_pubkeys: user_ura is required",
    "identity.list_user_pubkeys: user_ura must be a canonical User URA",
    "identity.list_user_pubkeys: user_ura must identify a User",
):
    if required not in production_handler:
        raise SystemExit(f"runtime_trust_user_key_inventory:missing_user_scope_guard:{required}")
if '"agent_ura"' in production_handler:
    raise SystemExit("runtime_trust_user_key_inventory:production_handler_mentions_retired_agent_field")
if "pub(crate) fn user_snapshot(&self, user_ura: &str)" not in trust:
    raise SystemExit("runtime_trust_user_key_inventory:user_snapshot_not_user_named")
if "pub(crate) user_ura: String" not in trust:
    raise SystemExit("runtime_trust_user_key_inventory:snapshot_field_not_user_ura")
if re.search(r"RuntimeTrustUserSnapshot\s*\{[^}]*agent_ura\s*:", trust, re.S):
    raise SystemExit("runtime_trust_user_key_inventory:snapshot_constructs_agent_ura")
contracts_section = contracts.split("ABILITY_IDENTITY_LIST_USER_PUBKEYS => object_schema(", 1)
if len(contracts_section) != 2:
    raise SystemExit("runtime_trust_user_key_inventory:contract_schema_missing")
schema_body = contracts_section[1].split("),", 1)[0]
if '"user_ura"' not in schema_body or '"agent_ura"' in schema_body:
    raise SystemExit("runtime_trust_user_key_inventory:contract_schema_not_user_scoped")
def list_user_pubkey_calls(source: str):
    for match in re.finditer(r'"identity\.list_user_pubkeys"', source):
        start = match.start()
        end = source.find(";", match.end())
        yield source[start : end if end >= 0 else min(len(source), match.end() + 240)]

for source, label in ((cli_user, "cli_user"), (doctor, "doctor")):
    for call in list_user_pubkey_calls(source):
        if '"agent_ura"' in call:
            raise SystemExit(f"runtime_trust_user_key_inventory:{label}_uses_retired_agent_field")
        if '"user_ura"' not in call:
            raise SystemExit(f"runtime_trust_user_key_inventory:{label}_missing_user_field")
for required_test in (
    "list_rejects_retired_agent_ura_request_field",
    "list_rejects_non_user_ura_scope",
):
    if required_test not in handler:
        raise SystemExit(f"runtime_trust_user_key_inventory:missing_test:{required_test}")
PY
}

check_runtime_trust_user_key_write_scope_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local register_handler="$cli_root/src/daemon/invocation/admission/register_device_pubkey.rs"
  local revoke_handler="$cli_root/src/daemon/invocation/admission/revoke_user_pubkey.rs"
  local trust="$cli_root/src/daemon/invocation/admission/runtime_trust.rs"
  local gate="$cli_root/src/daemon/invocation/admission/identity_write_gate.rs"
  local dispatcher="$cli_root/src/daemon/invocation/dispatch/unary_dispatcher.rs"
  local contracts="$cli_root/src/daemon/ability/catalog/daemon_invocation_contracts.rs"
  local cli_user="$cli_root/src/cli/commands/user_signing_identity.rs"
  local prelude="$cli_root/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  local device_sync="$cli_root/src/daemon/invocation/admission/device_trust_sync.rs"
  [[ -f "$register_handler" ]] || fail "identity.register_pubkey handler is missing: $register_handler"
  [[ -f "$revoke_handler" ]] || fail "identity.revoke_user_pubkey handler is missing: $revoke_handler"
  [[ -f "$trust" ]] || fail "runtime trust aggregate is missing: $trust"
  [[ -f "$contracts" ]] || fail "daemon invocation contracts source is missing: $contracts"

  "$PYTHON_BIN" - "$register_handler" "$revoke_handler" "$trust" "$gate" "$dispatcher" "$contracts" "$cli_user" "$prelude" "$device_sync" <<'PY'
import re
import sys
from pathlib import Path

register = Path(sys.argv[1]).read_text(encoding="utf-8")
revoke = Path(sys.argv[2]).read_text(encoding="utf-8")
trust = Path(sys.argv[3]).read_text(encoding="utf-8")
gate = Path(sys.argv[4]).read_text(encoding="utf-8") if Path(sys.argv[4]).exists() else ""
dispatcher = Path(sys.argv[5]).read_text(encoding="utf-8") if Path(sys.argv[5]).exists() else ""
contracts = Path(sys.argv[6]).read_text(encoding="utf-8")
cli_user = Path(sys.argv[7]).read_text(encoding="utf-8") if Path(sys.argv[7]).exists() else ""
prelude = Path(sys.argv[8]).read_text(encoding="utf-8") if Path(sys.argv[8]).exists() else ""
device_sync = Path(sys.argv[9]).read_text(encoding="utf-8") if Path(sys.argv[9]).exists() else ""

register_prod = register.split("\n#[cfg(test)]\nmod tests", 1)[0]
revoke_prod = revoke.split("\n#[cfg(test)]\nmod tests", 1)[0]

for required in (
    "#[serde(deny_unknown_fields)]",
    "principal_ura: String",
    "pub(crate) fn principal_ura(&self) -> &str",
    "identity.register_pubkey: principal_ura is required",
    "register_pubkey_with_owner(",
    "pub(crate) struct RegisterPubkeyRequest",
    "pub(crate) fn to_arguments_bytes(&self) -> serde_json::Result<Vec<u8>>",
    "fn role_wire(role: TrustedAgentRole) -> &'static str",
    "args.principal_ura",
):
    if required not in register_prod:
        raise SystemExit(f"runtime_trust_write_scope:register_missing:{required}")
for retired in (
    "agent_ura: String",
    "pub(crate) fn agent_ura(&self) -> &str",
    "args.agent_ura",
    '"agent_ura"',
    "agent_ura is required",
):
    if retired in register_prod:
        raise SystemExit(f"runtime_trust_write_scope:register_retired:{retired}")

for required in (
    "#[serde(deny_unknown_fields)]",
    "user_ura: String",
    "pub(crate) fn user_ura(&self) -> &str",
    "fn decode_revoke_args(",
    "identity.revoke_user_pubkey: user_ura is required",
    "args.user_ura",
):
    if required not in revoke_prod:
        raise SystemExit(f"runtime_trust_write_scope:revoke_missing:{required}")
for retired in (
    "agent_ura: String",
    "pub(crate) fn agent_ura(&self) -> &str",
    "args.agent_ura",
    '"agent_ura"',
    "agent_ura is required",
):
    if retired in revoke_prod:
        raise SystemExit(f"runtime_trust_write_scope:revoke_retired:{retired}")

for required in (
    "pub(crate) fn register_pubkey(",
    "principal_ura: String",
    "pub(crate) fn revoke_user_pubkey(",
    "user_ura: &str",
    "identity.revoke_user_pubkey: user_ura must identify a User",
    "URAKind::User",
):
    if required not in trust:
        raise SystemExit(f"runtime_trust_write_scope:runtime_trust_missing:{required}")

if "intent.agent_ura()" in gate or "intent.agent_ura()" in dispatcher:
    raise SystemExit("runtime_trust_write_scope:intent_uses_retired_agent_accessor")
for required in (
    "intent.principal_ura()",
    "intent.user_ura()",
):
    if required not in gate + dispatcher:
        raise SystemExit(f"runtime_trust_write_scope:intent_missing:{required}")

def ability_schema(source: str, marker: str) -> str:
    parts = source.split(marker, 1)
    if len(parts) != 2:
        raise SystemExit(f"runtime_trust_write_scope:schema_missing:{marker}")
    body = parts[1]
    next_marker = body.find("\n        ABILITY_")
    if next_marker >= 0:
        return body[:next_marker]
    return body

register_schema = ability_schema(contracts, "ABILITY_IDENTITY_REGISTER_PUBKEY => object_schema(")
if '"principal_ura"' not in register_schema or '"agent_ura"' in register_schema:
    raise SystemExit("runtime_trust_write_scope:register_schema_not_principal_scoped")
if '&["principal_ura", "public_key_b64", "role"]' not in register_schema:
    raise SystemExit("runtime_trust_write_scope:register_schema_required_tuple_not_principal_scoped")

revoke_schema = ability_schema(contracts, "ABILITY_IDENTITY_REVOKE_USER_PUBKEY => object_schema(")
if '"user_ura"' not in revoke_schema or '"agent_ura"' in revoke_schema:
    raise SystemExit("runtime_trust_write_scope:revoke_schema_not_user_scoped")
if '&["user_ura", "public_key_b64"]' not in revoke_schema:
    raise SystemExit("runtime_trust_write_scope:revoke_schema_required_tuple_not_user_scoped")

for label, source in (("cli_user", cli_user), ("prelude", prelude)):
    if '"identity.register_pubkey"' not in source:
        continue
    if '"principal_ura"' not in source:
        raise SystemExit(f"runtime_trust_write_scope:{label}_register_missing_principal_field")
    if re.search(r'"agent_ura"\s*:\s*user_ura\b', source):
        raise SystemExit(f"runtime_trust_write_scope:{label}_register_uses_retired_agent_field")

if '"identity.register_pubkey"' in prelude and "RegisterPubkeyRequest::new(" not in prelude:
    raise SystemExit("runtime_trust_write_scope:prelude_register_not_using_dto")
if "import_caller_trust" in device_sync:
    if "RegisterPubkeyRequest::new(" not in device_sync:
        raise SystemExit("runtime_trust_write_scope:device_trust_sync_register_not_using_dto")
    if re.search(r'"agent_ura"\s*:\s*caller_ura', device_sync):
        raise SystemExit("runtime_trust_write_scope:device_trust_sync_uses_retired_agent_field")
    if "request.to_arguments_bytes()" not in device_sync:
        raise SystemExit("runtime_trust_write_scope:device_trust_sync_register_not_deterministic")

for required_test, source in (
    ("register_rejects_retired_agent_ura_request_field", register),
    ("register_pubkey_request_encodes_principal_scoped_tuple", register),
    ("revoke_rejects_retired_agent_ura_request_field", revoke),
    ("revoke_rejects_non_user_ura_scope", revoke),
):
    if required_test not in source:
        raise SystemExit(f"runtime_trust_write_scope:missing_test:{required_test}")
PY
}

check_product_e2e_invocation_history_exact_scope_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local script="$cli_root/tools/scripts/docker-two-node-easyremote-cli-e2e.sh"
  [[ -f "$script" ]] || fail "docker EasyRemote CLI e2e script is missing: $script"

  "$PYTHON_BIN" - "$script" <<'PY'
import re
import sys
from pathlib import Path

script = Path(sys.argv[1]).read_text(encoding="utf-8")

for retired in (
    "fallback_name",
    "provider-invocation-list-all-after",
    "all_invocation_records",
):
    if retired in script:
        raise SystemExit(f"product_e2e_invocation_history_exact_scope:retired_fallback:{retired}")

if re.search(r'provider_cli\s+"invocation list --format json"', script):
    raise SystemExit("product_e2e_invocation_history_exact_scope:unscoped_provider_history_read")

helper = re.search(
    r"def ability_invocation_records\(exact_name: str\):\n(?P<body>(?:    .*\n)+)",
    script,
)
if helper is None:
    raise SystemExit("product_e2e_invocation_history_exact_scope:exact_helper_missing")
if "return invocation_records(exact_name)" not in helper.group("body"):
    raise SystemExit("product_e2e_invocation_history_exact_scope:helper_not_exact_only")

for expected in (
    "provider-invocation-list-provider-agent-after-cli.json",
    "provider-invocation-list-user-plugin-after-cli.json",
    "provider-invocation-list-add-after-cli.json",
    "provider-invocation-list-native-after-easyremote.json",
):
    if expected not in script:
        raise SystemExit(f"product_e2e_invocation_history_exact_scope:missing_exact_artifact:{expected}")
PY
}

check_device_trust_sync_caller_classification_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local device_sync="$cli_root/src/daemon/invocation/admission/device_trust_sync.rs"
  [[ -f "$device_sync" ]] || fail "device trust sync source is missing: ${device_sync#$cli_root/}"

  "$PYTHON_BIN" - "$device_sync" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

match = re.search(
    r"fn syncable_caller\s*\([^)]*\)\s*->\s*(?P<ret>[^{]+)\{(?P<body>.*?)\n    \}",
    text,
    re.S,
)
if match is None:
    raise SystemExit("device_trust_sync_caller_classification:syncable_caller_missing")
body = match.group("body")
ret = match.group("ret")
if "Result<Option<SyncableCaller>, String>" not in ret:
    raise SystemExit("device_trust_sync_caller_classification:not_typed_result")
for retired in (
    "parse_ura(caller_ura).ok()?",
    "parse_ura(caller_ura).ok()",
):
    if retired in body:
        raise SystemExit(f"device_trust_sync_caller_classification:retired_parse_fallback:{retired}")
for required in (
    "MalformedCaller(String)",
    "DeviceTrustSyncStatus::MalformedCaller(err)",
    "invalid caller_ura",
    "malformed_caller_ura_is_not_reported_as_non_syncable",
):
    if required not in text:
        raise SystemExit(f"device_trust_sync_caller_classification:missing:{required}")
PY
}

check_observe_health_contract_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local health="$cli_root/src/daemon/ability/builtins/governance/health.rs"
  [[ -f "$health" ]] || fail "observe.health handler is missing: $health"

  "$PYTHON_BIN" - "$health" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
production = text.split("\n#[cfg(test)]", 1)[0]

for retired in (
    "Back-compat diagnostics",
    "smoke diagnostics",
    '"echo"',
    '"echo":',
):
    if retired in production:
        raise SystemExit(f"observe_health_contract_projection:retired_diagnostic:{retired}")

handler = re.search(r"fn handler\([^)]*\) -> anyhow::Result<Value> \{(?P<body>.*?)\n\}", production, re.S)
if handler is None:
    raise SystemExit("observe_health_contract_projection:handler_missing")
body = handler.group("body")
for required in (
    '"status": "healthy"',
    '"details"',
    '"replied_at_unix_ms": ts',
    '"components"',
):
    if required not in body:
        raise SystemExit(f"observe_health_contract_projection:missing_contract_field:{required}")
if re.search(r"\bargs\b", body):
    raise SystemExit("observe_health_contract_projection:handler_still_echoes_input_args")
description = re.search(r"pub fn description\(\) -> &'static str \{(?P<body>.*?)\n\}", production, re.S)
if description is None or "Returns Axon observe.health status fields." not in description.group("body"):
    raise SystemExit("observe_health_contract_projection:description_not_contract_only")
PY
}

check_admission_owner_credentials_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local policy="$cli_root/src/daemon/invocation/admission/policy_gate.rs"
  [[ -f "$policy" ]] || return 0

  "$PYTHON_BIN" - "$policy" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()

resolve = re.search(
    r"pub\(crate\) fn resolve_owner\((?P<sig>.*?)\) -> (?P<ret>[^{]+)\{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if resolve is None:
    raise SystemExit("admission_owner_resolve_owner_missing")
if "Result<OwnerResolution, Status>" not in resolve.group("ret"):
    raise SystemExit("admission_owner_resolve_owner_not_fallible")
if "let owner = resolve_owner(" not in text or ")?" not in text[text.find("let owner = resolve_owner("):text.find("let principal = principal_for")]:
    raise SystemExit("admission_policy_gate_not_propagating_owner_resolution")

local = re.search(
    r"fn owner_fact_from_local_device\((?P<sig>.*?)\) -> (?P<ret>[^{]+)\{(?P<body>.*?)\n\}",
    text,
    re.DOTALL,
)
if local is None:
    raise SystemExit("admission_local_device_owner_fact_missing")
if "Result<Option<OwnerFact>, Status>" not in local.group("ret"):
    raise SystemExit("admission_local_device_owner_fact_not_fallible")
body = local.group("body")
for retired in (
    "load_credentials().ok()",
    "load_credentials().ok()?",
    "credentials.user_id().ok()",
    "parse_ura(ura).ok()",
):
    if retired in body:
        raise SystemExit(f"admission_local_owner_retired_fallback:{retired}")
for required in (
    "load_credentials_optional()",
    "return Ok(None)",
    "LOCAL_OWNER_CREDENTIALS_UNAVAILABLE",
    "LOCAL_OWNER_URA_INVALID",
):
    if required not in body:
        raise SystemExit(f"admission_local_owner_missing_fail_closed_path:{required}")
for test in (
    "local_device_owner_resolution_rejects_malformed_credentials",
    "paired_device_subject_projects_credentials_owner",
):
    if test not in text:
        raise SystemExit(f"missing_admission_owner_credentials_test:{test}")
PY
}

check_shared_local_device_owner_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local owner="$cli_root/src/daemon/invocation/admission/owner_resolution.rs"
  local policy="$cli_root/src/daemon/invocation/admission/policy_gate.rs"
  local bootstrap="$cli_root/src/daemon/invocation/admission/bootstrap_authority.rs"
  local facade="$cli_root/src/daemon/invocation/admission/admission_facade.rs"
  [[ -f "$owner" ]] || return 0

  "$PYTHON_BIN" - "$owner" "$policy" "$bootstrap" "$facade" <<'PY'
import re
import sys
from pathlib import Path

owner = Path(sys.argv[1]).read_text()
policy = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""
bootstrap = Path(sys.argv[3]).read_text() if Path(sys.argv[3]).exists() else ""
facade = Path(sys.argv[4]).read_text() if Path(sys.argv[4]).exists() else ""

fn = re.search(
    r"pub\(crate\) fn local_device_owner_fact\((?P<sig>.*?)\) -> (?P<ret>[^{]+)\{(?P<body>.*?)\n\}",
    owner,
    re.DOTALL,
)
if fn is None:
    raise SystemExit("shared_local_device_owner_fact_missing")
if "anyhow::Result<Option<OwnerFact>>" not in fn.group("ret"):
    raise SystemExit("shared_local_device_owner_fact_not_fallible")
body = fn.group("body")
for retired in (
    "parse_ura(ura).ok()",
    "load_credentials().ok()",
    "load_credentials().ok()?",
    "credentials.user_id().ok()",
):
    if retired in body:
        raise SystemExit(f"shared_local_device_owner_retired_fallback:{retired}")
for required in (
    "load_credentials_optional()?",
    "return Ok(None)",
    "local device owner URA invalid",
    "credentials.user_id()?",
):
    if required not in body:
        raise SystemExit(f"shared_local_device_owner_missing_fail_closed_path:{required}")
for test in (
    "local_device_owner_fact_returns_none_when_credentials_missing",
    "local_device_owner_fact_projects_saved_credentials",
    "local_device_owner_fact_rejects_malformed_credentials",
):
    if test not in owner:
        raise SystemExit(f"missing_shared_local_device_owner_test:{test}")

if policy:
    if "pub(crate) fn principal_for(" not in policy or "-> Result<PrincipalProjection, Status>" not in policy:
        raise SystemExit("device_principal_projection_not_fallible")
    for required in (
        "LOCAL_DEVICE_PRINCIPAL_OWNER_UNAVAILABLE",
        "principal_for(context.trusted_role, &caller_ura, context.trust_anchor)?",
        "device_principal_projection_rejects_malformed_local_credentials",
    ):
        if required not in policy:
            raise SystemExit(f"device_principal_projection_missing:{required}")

if bootstrap:
    if "Unavailable { message: String }" not in bootstrap:
        raise SystemExit("bootstrap_authority_unavailable_state_missing")
    for required in (
        "LOCAL_BOOTSTRAP_OWNER_UNAVAILABLE",
        "malformed_local_credentials_make_bootstrap_owner_unavailable",
    ):
        if required not in bootstrap:
            raise SystemExit(f"bootstrap_owner_projection_missing:{required}")

if facade and "BootstrapAuthorityDecision::Unavailable { message }" not in facade:
    raise SystemExit("admission_facade_not_mapping_bootstrap_unavailable")
PY
}

check_node_session_authority_subject_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local node="$cli_root/sdk/node/index.js"
  local test="$cli_root/sdk/node/test/runtime-core.test.mjs"
  [[ -f "$node" ]] || return 0

  "$PYTHON_BIN" - "$node" "$test" <<'PY'
import re
import sys
from pathlib import Path

node = Path(sys.argv[1]).read_text()
test = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""

for required in (
    "function validateSessionAuthoritySubjectBinding(",
    "function canonicalAuthoritySubject(",
    "session authority subject_ura must be a canonical user or session subject",
    "session authority user subject must match session_owner_user_id",
    "session authority subject_ura owner/session must match session_owner_user_id and session_id",
):
    if required not in node:
        raise SystemExit(f"node_session_authority_subject_contract_missing:{required}")

def body_after(marker: str, next_marker: str) -> str:
    start = node.find(marker)
    if start < 0:
        raise SystemExit(f"node_session_authority_subject_contract_missing:{marker}")
    end = node.find(next_marker, start + len(marker))
    return node[start : end if end >= 0 else len(node)]

authority_body = body_after(
    "function validateSessionAuthority(authority)",
    "function validateDelegationRequest",
)
request_body = body_after(
    "function validateSessionAuthorityRequest(request)",
    "function validateSessionAuthoritySubjectBinding",
)
for name, body in (
    ("authority", authority_body),
    ("request", request_body),
):
    if "validateSessionAuthoritySubjectBinding(" not in body:
        raise SystemExit(f"node_session_authority_{name}_does_not_validate_subject_binding")
    if "rejectAllZeroAuthorityFields(" in body and "validateSessionAuthoritySubjectBinding(" not in body:
        raise SystemExit(f"node_session_authority_{name}_stops_at_all_zero_guard")

subject_body = body_after(
    "function canonicalAuthoritySubject(subjectURA)",
    "function rejectAllZeroAuthorityFields",
)
for token in (
    '"user/"',
    '"resource/user."',
    '"/session/"',
):
    if token not in subject_body:
        raise SystemExit(f"node_session_authority_subject_classifier_missing:{token}")

for required_test in (
    "authority metadata binds session subject to owner and session id",
    "easynet:///r/example/user/bob",
    "easynet:///r/example/resource/user.alice/session/session-2",
):
    if required_test not in test:
        raise SystemExit(f"missing_node_session_authority_subject_test:{required_test}")
PY
}

check_runtime_authority_metadata_key_neutrality_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local paths=(
    "$cli_root/src/daemon/invocation/admission/authority_metadata.rs"
    "$cli_root/sdk/go/authority.go"
    "$cli_root/sdk/python/easynet_sdk/authority.py"
    "$cli_root/sdk/node/index.js"
    "$cli_root/sdk/node/index.d.ts"
    "$cli_root/sdk/java/src/main/java/run/runtime/sdk/AuthoritySupport.java"
    "$cli_root/sdk/swift/Sources/RuntimeSDK/Authority.swift"
    "$cli_root/sdk/schemas/authority.schema.json"
    "$cli_root/sdk/conformance/fixtures/authority-metadata.v4.json"
    "$cli_root/sdk/conformance/cases/authority-mutual-exclusion.yaml"
  )

  for path in "${paths[@]}"; do
    [[ -f "$path" ]] || fail "runtime authority metadata key source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "${paths[@]}" <<'PY'
import sys
from pathlib import Path

texts = {Path(path).as_posix(): Path(path).read_text(encoding="utf-8", errors="replace") for path in sys.argv[1:]}
combined = "\n".join(texts.values())

for retired in ("x-easynet-delegation", "x-easynet-session-authority"):
    if retired in combined:
        raise SystemExit(f"runtime_authority_metadata_key_neutrality:retired_product_key:{retired}")

for required in ("x-runtime-delegation", "x-runtime-session-authority"):
    for path, text in texts.items():
        if required not in text:
            raise SystemExit(
                f"runtime_authority_metadata_key_neutrality:missing:{required}:{path}"
            )

PY
}

check_admission_authority_raw_wire_strict_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local facade="$cli_root/src/daemon/invocation/admission/admission_facade.rs"
  [[ -f "$facade" ]] || fail "admission facade source is missing: ${facade#$cli_root/}"

  "$PYTHON_BIN" - "$facade" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

for struct_name in ("DelegationProofRaw", "SessionAuthorityRaw"):
    match = re.search(
        rf"struct {struct_name} \{{(?P<body>.*?)\n\}}",
        text,
        re.S,
    )
    if match is None:
        raise SystemExit(f"admission_authority_raw_wire_strict:missing:{struct_name}")
    body = match.group("body")
    if "#[serde(default)]" in body:
        raise SystemExit(f"admission_authority_raw_wire_strict:serde_default_retired:{struct_name}")
    for required in ("payload:", "signature:"):
        if required not in body:
            raise SystemExit(f"admission_authority_raw_wire_strict:field_missing:{struct_name}:{required}")

for required in (
    "admission_authority_raw_wire_requires_payload_and_signature",
    "missing raw fields must not be reinterpreted as payload/signature defaults",
    "parse_and_verify_delegation_proof(",
    "parse_and_verify_session_authority(",
):
    if required not in text:
        raise SystemExit(f"admission_authority_raw_wire_strict:test_missing:{required}")
PY
}

check_admission_authority_ability_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local facade="$cli_root/src/daemon/invocation/admission/admission_facade.rs"
  [[ -f "$facade" ]] || fail "admission facade source is missing: ${facade#$cli_root/}"

  "$PYTHON_BIN" - "$facade" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

match = re.search(
    r"impl AuthorityAbilityView\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if match is None:
    raise SystemExit("admission_authority_ability_projection:view_impl_missing")
body = match.group("body")
for retired in (
    "owner_local_ability_name",
    "unwrap_or_else(|_| owner_local_ability_name",
):
    if retired in body:
        raise SystemExit(f"admission_authority_ability_projection:retired_owner_local_fallback:{retired}")
for required in (
    "public_name_from_authority_ability_ura(&ability_ura)?",
    "fn public_name_from_authority_ability_ura(ability_ura: &str) -> Result<String, Status>",
    "authority ability projection derived non-canonical ability URA",
    "authority_ability_projection_rejects_non_canonical_ability_ura",
):
    if required not in text:
        raise SystemExit(f"admission_authority_ability_projection:missing:{required}")
PY
}

check_peer_envelope_signer_subject_profile_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local signer="$cli_root/src/daemon/invocation/admission/peer_envelope_signer.rs"
  [[ -f "$signer" ]] || fail "peer envelope signer source is missing: ${signer#$cli_root/}"

  "$PYTHON_BIN" - "$signer" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

match = re.search(
    r"pub\(crate\) async fn sign_peer_request_envelope\s*\([^)]*\)\s*->\s*Result<String, Status>\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if match is None:
    raise SystemExit("peer_envelope_signer_subject_profile:signer_body_missing")
body = match.group("body")
for retired in (
    "DEFAULT_URA_PROFILE",
    "unwrap_or_else(|| crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string())",
):
    if retired in body:
        raise SystemExit(f"peer_envelope_signer_subject_profile:retired_signing_fallback:{retired}")
for required in (
    "required_subject_profile(envelope)?",
    "explicit subject profile before descriptor subject normalization",
    "fn required_subject_profile(envelope: &Envelope) -> Result<String, Status>",
    "sign_peer_request_rejects_missing_subject_profile_before_normalization",
):
    if required not in text:
        raise SystemExit(f"peer_envelope_signer_subject_profile:missing:{required}")
PY
}

check_session_prelude_credentials_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local prelude="$cli_root/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  [[ -f "$prelude" ]] || return 0

  "$PYTHON_BIN" - "$prelude" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
start = text.find("async fn sync_paired_user_trust_prelude(")
if start < 0:
    raise SystemExit("session_prelude_user_trust_sync_missing")
end = text.find("\nfn resolved_public_keys", start)
body = text[start : end if end >= 0 else len(text)]

for retired in (
    "let Ok(creds)",
    "let Ok(user_ura)",
    "load_credentials().ok()",
    "load_credentials().ok()?",
    "user_ura().ok()",
):
    if retired in body:
        raise SystemExit(f"session_prelude_credentials_retired_fallback:{retired}")

required = (
    "load_credentials_optional()",
    "UserTrustBootstrapError::CredentialsUnavailable",
    "load paired credentials",
    "project paired user URA",
)
for token in required:
    if token not in body:
        raise SystemExit(f"session_prelude_credentials_missing_fail_closed_path:{token}")

not_required_count = body.count("return Ok(UserTrustBootstrapOutcome::NotRequired);")
if not_required_count != 2:
    raise SystemExit(f"session_prelude_credentials_not_required_count:{not_required_count}")

for test in (
    "paired_user_trust_bootstrap_ignores_missing_credentials_only",
    "paired_user_trust_bootstrap_rejects_malformed_credentials",
):
    if test not in text:
        raise SystemExit(f"missing_session_prelude_credentials_test:{test}")

advertise_start = text.find("async fn run_hosted_agent_advertise_prelude(")
if advertise_start < 0:
    raise SystemExit("session_prelude_hosted_agent_advertise_missing")
advertise_end = text.find("\nasync fn advertise_hosted_agent_entry", advertise_start)
advertise_body = text[advertise_start : advertise_end if advertise_end >= 0 else len(text)]
for retired in (
    ".username\n            .filter",
    ".username.filter",
    ".unwrap_or_default()",
):
    if retired in advertise_body:
        raise SystemExit(f"session_prelude_hosted_agent_owner_retired_fallback:{retired}")
if "resolve_hosted_agent_user_segment(hub_endpoint)?" not in advertise_body:
    raise SystemExit("session_prelude_hosted_agent_owner_projector_not_used")

helper_start = text.find("fn resolve_hosted_agent_user_segment(")
if helper_start < 0:
    raise SystemExit("session_prelude_hosted_agent_owner_projector_missing")
helper_end = text.find("\nasync fn advertise_hosted_agent_entry", helper_start)
helper_body = text[helper_start : helper_end if helper_end >= 0 else len(text)]
for required in (
    "load_credentials_optional()",
    "pages_user_from_env_or_credentials(",
    "no user-root Pages identity is bound",
    "SessionError::HostedAgentPreludeFailed",
    "project username for hosted-agent owner projection",
):
    if required not in helper_body:
        raise SystemExit(f"session_prelude_hosted_agent_owner_projector_missing:{required}")
for retired in (
    "EASYNET_PAGES_USER",
    ".username_slug()",
    "load_credentials()",
):
    if retired in helper_body:
        raise SystemExit(f"session_prelude_hosted_agent_owner_projector_not_shared:{retired}")
for test in (
    "hosted_agent_owner_segment_accepts_explicit_dev_override",
    "hosted_agent_owner_segment_reads_valid_paired_credentials",
    "hosted_agent_owner_segment_rejects_federation_native_credentials_without_username",
):
    if test not in text:
        raise SystemExit(f"missing_session_prelude_hosted_agent_owner_test:{test}")
PY
}

check_start_attach_user_signer_readiness_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local discovery="$cli_root/src/daemon/control/discovery.rs"
  local server="$cli_root/src/daemon/control/server.rs"
  local daemon_bin="$cli_root/src/bin/easynet-daemon.rs"
  local lifecycle_start="$cli_root/src/daemon/boot/lifecycle/start.rs"
  local lifecycle_discovery="$cli_root/src/daemon/boot/lifecycle/discovery.rs"
  local lifecycle_errors="$cli_root/src/daemon/boot/lifecycle/errors.rs"
  [[ -f "$discovery" ]] || fail "control discovery source is missing: $discovery"
  [[ -f "$server" ]] || fail "control server source is missing: $server"
  [[ -f "$daemon_bin" ]] || fail "daemon bin source is missing: $daemon_bin"
  [[ -f "$lifecycle_start" ]] || fail "lifecycle start source is missing: $lifecycle_start"
  [[ -f "$lifecycle_discovery" ]] || fail "lifecycle discovery source is missing: $lifecycle_discovery"
  [[ -f "$lifecycle_errors" ]] || fail "lifecycle errors source is missing: $lifecycle_errors"

  "$PYTHON_BIN" - "$discovery" "$server" "$daemon_bin" "$lifecycle_start" "$lifecycle_discovery" "$lifecycle_errors" <<'PY'
import re
import sys
from pathlib import Path

discovery, server, daemon_bin, start, lifecycle_discovery, errors = [
    Path(arg).read_text() for arg in sys.argv[1:]
]

if 'pub const PAIRED_USER_RUNTIME_SIGNER: &str = "paired_user_runtime_signer";' not in discovery:
    raise SystemExit("start_attach_user_signer_readiness:flag_missing")

if "pub capability_flags: Vec<String>" not in server:
    raise SystemExit("start_attach_user_signer_readiness:runtime_discovery_flags_missing")
if "fn discovery_capability_flags(" not in server:
    raise SystemExit("start_attach_user_signer_readiness:flag_merger_missing")
if "runtime.capability_flags" not in server:
    raise SystemExit("start_attach_user_signer_readiness:ready_flags_not_consumed")
for required in ("flags::BOOT_STATUS", "flags::CONTROL_DIAGNOSTICS"):
    if required not in server:
        raise SystemExit(f"start_attach_user_signer_readiness:base_flag_missing:{required}")

ready = re.search(r"fn ready_runtime_discovery\(\s*capability_flags: Vec<String>,\s*\) -> anyhow::Result<server::ControlRuntimeDiscovery> \{(?P<body>.*?)\n\}", daemon_bin, re.DOTALL)
if ready is None:
    raise SystemExit("start_attach_user_signer_readiness:ready_discovery_missing")
ready_body = ready.group("body")
for required in (
    "capability_flags",
    "ready_daemon_identity(&config)",
):
    if required not in ready_body:
        raise SystemExit(f"start_attach_user_signer_readiness:ready_discovery_missing:{required}")
for retired in (
    "DaemonMode::Device",
    "DaemonMode::Both",
    "PAIRED_USER_RUNTIME_SIGNER.to_string()",
    "capability_flags.push(",
):
    if retired in ready_body:
        raise SystemExit(f"start_attach_user_signer_readiness:mode_derived_ready_flag:{retired}")
if 'std::env::var("EASYNET_NODE_ID")' in ready_body:
    raise SystemExit("start_attach_user_signer_readiness:ready_identity_uses_env_node_id")
if "fn ready_daemon_identity(" not in daemon_bin or "config::load_credentials()" not in daemon_bin:
    raise SystemExit("start_attach_user_signer_readiness:ready_identity_credentials_helper_missing")
for required_test in (
    "ready_discovery_uses_paired_credentials_node_id_not_env",
    "ready_discovery_does_not_infer_signer_readiness_from_device_mode",
    "ready_discovery_rejects_credentials_realm_mismatch",
):
    if required_test not in daemon_bin:
        raise SystemExit(f"start_attach_user_signer_readiness:missing_ready_identity_test:{required_test}")

if "pub fn has_capability_flag(&self, flag: &str) -> bool" not in lifecycle_discovery:
    raise SystemExit("start_attach_user_signer_readiness:daemon_snapshot_flag_query_missing")

for required in (
    "fn validate_attach_capabilities(",
    "has_capability_flag(crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER)",
    "StartRefusedMissingRuntimeCapability",
    "start_preflight_refuses_device_attach_without_paired_user_signer_readiness",
):
    if required not in start:
        raise SystemExit(f"start_attach_user_signer_readiness:start_missing:{required}")

if "StartRefusedMissingRuntimeCapability" not in errors:
    raise SystemExit("start_attach_user_signer_readiness:error_variant_missing")

retired = (
    "RuntimeStartPreflightAction::AlreadyRunning => Ok(())",
    "RuntimeStartPreflightAction::AttachAndRebuildProjection => Ok(())",
)
for token in retired:
    if token in start and "validate_attach_capabilities" not in start:
        raise SystemExit(f"start_attach_user_signer_readiness:unchecked_attach:{token}")
PY
}

check_session_prelude_receipt_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local prelude="$cli_root/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  local heartbeat="$cli_root/src/daemon/invocation/bidi/session_initiator/heartbeat.rs"
  [[ -f "$prelude" ]] || return 0
  [[ -f "$heartbeat" ]] || return 0

  "$PYTHON_BIN" - "$prelude" "$heartbeat" <<'PY'
import sys
from pathlib import Path

prelude = Path(sys.argv[1]).read_text(encoding="utf-8")
heartbeat = Path(sys.argv[2]).read_text(encoding="utf-8")

if "fn apply_federation_join_receipt" not in prelude:
    raise SystemExit("session_prelude_receipt:join_projection_missing")
join_projection = prelude.split("fn apply_federation_join_receipt", 1)[1].split(
    "\nfn federation_join_public_key_hex", 1
)[0]
if "parse_receipt::<" not in join_projection or "JoinReceipt" not in join_projection:
    raise SystemExit("session_prelude_receipt:join_parse_receipt_missing")
if "receipt body is empty" not in join_projection:
    raise SystemExit("session_prelude_receipt:join_empty_body_gate_missing")
if "serde_json::from_slice" in join_projection or "if let Ok" in join_projection:
    raise SystemExit("session_prelude_receipt:join_tolerant_decode")

if "fn apply_federation_heartbeat_receipt" not in heartbeat:
    raise SystemExit("session_prelude_receipt:heartbeat_projection_missing")
heartbeat_projection = heartbeat.split("fn apply_federation_heartbeat_receipt", 1)[1].split(
    "\n#[cfg(test)]", 1
)[0]
if "parse_receipt::<" not in heartbeat_projection or "HeartbeatReceipt" not in heartbeat_projection:
    raise SystemExit("session_prelude_receipt:heartbeat_parse_receipt_missing")
if "receipt body is empty" not in heartbeat_projection:
    raise SystemExit("session_prelude_receipt:heartbeat_empty_body_gate_missing")
if "serde_json::from_slice" in heartbeat_projection or "if let Ok" in heartbeat_projection:
    raise SystemExit("session_prelude_receipt:heartbeat_tolerant_decode")
if "!diff.added.is_empty() || !diff.removed.is_empty()" in heartbeat:
    raise SystemExit("session_prelude_receipt:heartbeat_revision_only_diff_skipped")
if "fn heartbeat_refresh_owner_uras_for_caller" not in heartbeat:
    raise SystemExit("session_prelude_receipt:heartbeat_owner_refresh_projection_missing")
if "heartbeat_refresh_owner_uras()" in heartbeat and ".unwrap_or_default()" in heartbeat:
    raise SystemExit("session_prelude_receipt:heartbeat_owner_refresh_error_collapsed")
owner_refresh_projection = heartbeat.split("fn heartbeat_refresh_owner_uras_for_caller", 1)[1].split(
    "\nfn apply_federation_heartbeat_receipt", 1
)[0]
if "owner projection cursor unavailable" not in owner_refresh_projection:
    raise SystemExit("session_prelude_receipt:heartbeat_owner_refresh_error_context_missing")

for test in (
    "federation_join_receipt_rejects_empty_or_malformed_body",
    "federation_join_receipt_seeds_canonical_hub_catalog",
):
    if test not in prelude:
        raise SystemExit(f"session_prelude_receipt:missing_join_test:{test}")
for test in (
    "federation_heartbeat_receipt_rejects_empty_or_malformed_body",
    "federation_heartbeat_receipt_applies_revision_only_diff",
    "heartbeat_refresh_owner_uras_rejects_corrupt_cursor_store",
):
    if test not in heartbeat:
        raise SystemExit(f"session_prelude_receipt:missing_heartbeat_test:{test}")
PY
}

check_device_settings_loader_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local config="$cli_root/src/daemon/persistence/config.rs"
  [[ -f "$config" ]] || return 0
  local config_cmd="$cli_root/src/cli/commands/config_cmd.rs"

  "$PYTHON_BIN" - "$config" "$config_cmd" <<'PY'
import re
import sys
from pathlib import Path

config = Path(sys.argv[1]).read_text()
config_cmd = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""

settings = re.search(
    r"#\[derive\([^\n]*\)\]\n(?P<attrs>(?:#\[[^\n]+\]\n)*)pub struct DeviceSettings \{",
    config,
)
if settings is None:
    raise SystemExit("device_settings_struct_missing")
if "#[serde(deny_unknown_fields)]" not in settings.group("attrs"):
    raise SystemExit("device_settings_unknown_fields_not_denied")
loader = re.search(
    r"pub fn load_device_settings\(\) -> anyhow::Result<DeviceSettings> \{(?P<body>.*?)\n\}\n\npub fn save_device_settings",
    config,
    re.DOTALL,
)
if loader is None:
    raise SystemExit("device_settings_fallible_loader_missing")
body = loader.group("body")
for retired in (
    "fs::read_to_string(&path)\n        .ok()",
    "serde_json::from_str(&data).ok()",
    "unwrap_or_default()",
):
    if retired in body:
        raise SystemExit(f"device_settings_retired_default_fallback:{retired}")
for required in (
    "ErrorKind::NotFound",
    "Ok(DeviceSettings::default())",
    "parse device settings",
):
    if required not in body:
        raise SystemExit(f"device_settings_loader_missing:{required}")
if "let mut settings = load_device_settings()?" not in config:
    raise SystemExit("install_id_generation_does_not_propagate_settings_error")
if config_cmd and "config::load_device_settings()?" not in config_cmd:
    raise SystemExit("config_command_does_not_propagate_settings_error")
for test in (
    "load_device_settings_missing_file_returns_default",
    "load_device_settings_rejects_malformed_existing_file",
    "load_device_settings_rejects_unknown_fields",
    "load_or_create_install_id_rejects_malformed_settings_without_rewriting",
):
    if test not in config:
        raise SystemExit(f"missing_device_settings_loader_test:{test}")
PY
}

check_mission_traditional_target_conflict_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local orchestration="$cli_root/src/daemon/execution/mission/orchestration.rs"
  [[ -f "$orchestration" ]] || return 0
  local parser="$cli_root/src/eal/parser/mod.rs"
  local ir="$cli_root/src/eal/runtime/ir.rs"

  "$PYTHON_BIN" - "$orchestration" "$parser" "$ir" <<'PY'
import sys
from pathlib import Path

orchestration = Path(sys.argv[1]).read_text()
parser = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""
ir = Path(sys.argv[3]).read_text() if Path(sys.argv[3]).exists() else ""

for retired in (
    "ImplicitAgentFallback",
    "find_implicit_agent_fallback",
    "implicit agent fallback",
    "implicit-agent-fallback",
):
    if retired in orchestration:
        raise SystemExit(f"mission_retired_fallback_concept:{retired}")
for required in (
    "struct TraditionalAgentTargetConflict",
    "fn find_traditional_agent_target_conflict(",
    "AgentAggregateRepository::load_snapshot()?",
    "registered_agent_surface_names()",
):
    if required not in orchestration:
        raise SystemExit(f"mission_target_conflict_missing:{required}")
for retired_test in (
    "no_implicit_agent_fallback",
    "implicit-fallback check",
):
    if retired_test in orchestration:
        raise SystemExit(f"mission_retired_fallback_test_concept:{retired_test}")
for test in (
    "traditional_agent_target_conflict_traditional_form_with_agent_name_is_rejected",
    "traditional_agent_target_conflict_member_call_form_is_accepted",
    "traditional_agent_target_conflict_traditional_form_with_device_name_is_accepted",
):
    if test not in orchestration:
        raise SystemExit(f"missing_mission_target_conflict_test:{test}")
for doc_name, doc in (("parser", parser), ("ir", ir)):
    if "find_implicit_agent_fallback" in doc or "No implicit agent fallback" in doc:
        raise SystemExit(f"mission_retired_fallback_doc:{doc_name}")
    if doc and "find_traditional_agent_target_conflict" not in doc:
        raise SystemExit(f"mission_target_conflict_doc_missing:{doc_name}")
PY
}

check_edge_adapter_policy_contract() {
  "$PYTHON_BIN" "$EDGE_ADAPTER_POLICY" --manifest "$MANIFEST" >/dev/null
}

check_daemon_tuple_route_contract() {
  bash "$ROOT/tools/scripts/check-daemon-invocation-migration.sh" >/dev/null
}

check_remote_invocation_subject_provenance_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local remote="$cli_root/src/daemon/invocation/routing/remote_invoke.rs"
  local ffi_invocation="$cli_root/src/ffi/invocation/mod.rs"

  [[ -f "$remote" ]] || fail "remote invocation routing source is missing"
  [[ -f "$ffi_invocation" ]] || fail "FFI invocation source is missing"

  "$PYTHON_BIN" - "$remote" "$ffi_invocation" <<'PY'
import sys
from pathlib import Path

remote = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
ffi_invocation = Path(sys.argv[2]).read_text(encoding="utf-8", errors="replace")
production = remote.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]
ffi_production = ffi_invocation.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]

for retired in (
    "Public compatibility may still offer ergonomic subject omission",
    "TargetOwnedSystem",
    "RemoteInvocationSubject::Explicit",
    'Self::Explicit(value) => (value, "explicit subject")',
):
    if retired in production or retired in ffi_production:
        raise SystemExit(f"remote_invocation_subject_provenance:retired_subject_policy:{retired}")

for required in (
    "enum RemoteInvocationSubject",
    "CallerDeclared(String)",
    "DaemonTargetOwned(String)",
    "no public subject omission, callee substitution, or descriptor substitution",
    "RemoteInvocationSubject::CallerDeclared(subject_ura.into())",
    "Self::CallerDeclared(value) => (value, \"caller-declared subject\")",
    "Self::DaemonTargetOwned(value) => (value, \"daemon target-owned subject\")",
):
    if required not in remote:
        raise SystemExit(f"remote_invocation_subject_provenance:missing_remote_state:{required}")

for retired in (
    "RemoteDescriptorCatalogProbe",
    "prepare remote descriptor catalog probe",
    "RemoteInvocationSubject::DaemonTargetOwned(",
):
    if retired in ffi_production:
        raise SystemExit(f"remote_invocation_subject_provenance:retired_ffi_descriptor_probe:{retired}")

for required_test in (
    'assert_eq!(plan.subject.policy_name(), "CallerDeclared")',
    'assert_eq!(plan.subject.policy_name(), "DaemonTargetOwned")',
):
    if required_test not in remote:
        raise SystemExit(f"remote_invocation_subject_provenance:missing_test:{required_test}")
PY
}

check_daemon_runtime_route_inventory_contract() {
  bash "$ROOT/tools/scripts/check-architecture-convergence.sh" >/dev/null
}

check_daemon_local_device_identity_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local identity="$cli_root/src/daemon/identity/local_invocation.rs"
  [[ -f "$identity" ]] || fail "daemon local invocation identity source is missing: $identity"

  "$PYTHON_BIN" - "$identity" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

for retired_global in (
    "UNPAIRED_LOCAL_REALM",
    "UNPAIRED_LOCAL_DEVICE_ID",
):
    if retired_global in text:
        raise SystemExit(f"local_device_identity:retired_default_constant:{retired_global}")

if "pub(crate) fn local_device_ura() -> anyhow::Result<String>" not in text:
    raise SystemExit("local_device_identity:local_device_ura_not_fallible")

start = text.find("pub(crate) fn local_device_ura()")
end = text.find("pub(crate) fn local_daemon_ura()", start)
if start < 0 or end < 0:
    raise SystemExit("local_device_identity:local_device_ura_section_missing")
body = text[start:end]

for retired in (
    "unwrap_or_else",
    "UNPAIRED_LOCAL_REALM",
    "UNPAIRED_LOCAL_DEVICE_ID",
    'device_ura("default"',
    'device_ura( "default"',
):
    if retired in body:
        raise SystemExit(f"local_device_identity:default_local_fallback_retired:{retired}")

for required in (
    "persisted_local_device_ura()",
    "crate::daemon::persistence::config::load_credentials()",
    "local device identity unavailable",
):
    if required not in body:
        raise SystemExit(f"local_device_identity:projection_or_error_missing:{required}")

for required_test in (
    "local_device_ura_rejects_missing_identity_instead_of_synthesizing_default_local",
    "local_device_ura_projects_credentials_when_hosted_identity_is_absent",
):
    if required_test not in text:
        raise SystemExit(f"local_device_identity:test_missing:{required_test}")
PY
}

check_filesystem_resource_owner_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local files="$cli_root/src/daemon/resources/files/mod.rs"
  [[ -f "$files" ]] || fail "filesystem ResourceRef source is missing: $files"

  "$PYTHON_BIN" - "$files" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

if "fn resource_ref_value(" not in text:
    raise SystemExit("filesystem_resource_owner:resource_ref_value_missing")
start = text.find("fn resource_ref_value(")
end = text.find("fn map_local_path_to_virtual_resource", start)
if end < 0:
    raise SystemExit("filesystem_resource_owner:resource_ref_value_section_missing")
body = text[start:end]

if ") -> Result<Value>" not in body:
    raise SystemExit("filesystem_resource_owner:resource_ref_value_not_fallible")

for retired in (
    ".ok()",
    "unwrap_or_else",
    "UNPAIRED_LOCAL_REALM",
    "UNPAIRED_LOCAL_DEVICE_ID",
    'device_ura("default"',
    'device_ura( "default"',
):
    if retired in body:
        raise SystemExit(f"filesystem_resource_owner:default_local_fallback_retired:{retired}")

for required in (
    "crate::daemon::identity::local_invocation::local_device_ura()",
    "resource_ref: local device owner unavailable",
    "parsed_owner.kind != crate::core::ura::URAKind::Device",
):
    if required not in body:
        raise SystemExit(f"filesystem_resource_owner:canonical_owner_projection_missing:{required}")

for required_test in (
    "resource_ref_for_local_path_rejects_missing_local_device_identity",
    "resource_ref_for_local_path_binds_credentials_backed_device_owner",
    "provision_local_device_credentials",
):
    if required_test not in text:
        raise SystemExit(f"filesystem_resource_owner:test_missing:{required_test}")
PY
}

check_federation_probe_local_identity_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local probe="$cli_root/src/daemon/ability/builtins/integrations/federation_probe.rs"
  local ops="$cli_root/src/daemon/ability/builtins/device_control/ability_management/ops.rs"
  [[ -f "$probe" ]] || return 0

  "$PYTHON_BIN" - "$probe" "$ops" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
ops = Path(sys.argv[2]).read_text(encoding="utf-8") if len(sys.argv) > 2 and Path(sys.argv[2]).exists() else ""

if "pub(crate) enum LocalIdentity" not in text:
    raise SystemExit("federation_probe_local_identity:not_explicit_state")
for required in (
    "Paired {",
    "Unavailable {",
    "local device identity unavailable",
    "collect_device_view_rejects_missing_local_identity_without_default_node",
):
    if required not in text:
        raise SystemExit(f"federation_probe_local_identity:missing:{required}")

start = text.find("pub(crate) fn local_identity()")
end = text.find("pub(crate) fn collect_device_view", start)
if start < 0 or end < 0:
    raise SystemExit("federation_probe_local_identity:local_identity_section_missing")
body = text[start:end]
for retired in (
    "UNPAIRED_LOCAL_REALM",
    "UNPAIRED_LOCAL_DEVICE_ID",
    'node_id: "local"',
    'tenant_id: "default"',
    'paired: false',
):
    if retired in body:
        raise SystemExit(f"federation_probe_local_identity:retired_default_identity:{retired}")

collect = text[text.find("fn collect_device_view_with_probe("):]
collect = collect[: collect.find("/// Resolve one device", 0) if "/// Resolve one device" in collect else len(collect)]
if "nodes: Vec::new()" not in collect:
    raise SystemExit("federation_probe_local_identity:missing_unavailable_empty_nodes")

for retired in (
    "fn local_identity() -> (String, String, Option<String>, bool)",
    ".paired",
):
    if retired in ops:
        raise SystemExit(f"federation_probe_local_identity:ops_retired_tuple_helper:{retired}")
for required in (
    "struct LocalDeviceIdentity",
    "device operation local identity unavailable",
):
    if ops and required not in ops:
        raise SystemExit(f"federation_probe_local_identity:ops_missing:{required}")
PY
}

check_ready_capability_proof_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local invocation="$cli_root/src/daemon/boot/invocation/mod.rs"
  [[ -f "$daemon" ]] || fail "daemon entrypoint is missing: $daemon"
  [[ -f "$invocation" ]] || fail "invocation boot source is missing: $invocation"

  "$PYTHON_BIN" - "$daemon" "$invocation" <<'PY'
import sys
from pathlib import Path

daemon = Path(sys.argv[1]).read_text(encoding="utf-8")
invocation = Path(sys.argv[2]).read_text(encoding="utf-8")

start = daemon.find("fn ready_runtime_discovery(")
if start < 0:
    raise SystemExit("ready_capability_proof:ready_runtime_discovery_missing")
end = daemon.find("\nfn ready_daemon_identity", start)
if end < 0:
    raise SystemExit("ready_capability_proof:ready_runtime_discovery_section_missing")
body = daemon[start:end]

for retired in (
    "DaemonMode::Device",
    "DaemonMode::Both",
    "PAIRED_USER_RUNTIME_SIGNER.to_string()",
    "capability_flags.push(",
):
    if retired in body:
        raise SystemExit(f"ready_capability_proof:mode_derived_flag_retired:{retired}")

for required in (
    "fn ready_runtime_discovery(",
    "capability_flags: Vec<String>",
    "capability_flags,",
    "let invocation_capability_flags = session_shutdown.capability_flags().to_vec();",
    "ready_runtime_discovery(invocation_capability_flags)",
    "ready_discovery_does_not_infer_signer_readiness_from_device_mode",
):
    if required not in daemon:
        raise SystemExit(f"ready_capability_proof:daemon_contract_missing:{required}")

for required in (
    "pub struct InvocationTransportReady",
    "_session_shutdown: SessionShutdown",
    "capability_flags: Vec<String>",
    "pub fn capability_flags(&self) -> &[String]",
    "ready_capability_flags",
    "crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER.to_string()",
    "Ok(InvocationTransportReady::new(",
):
    if required not in invocation:
        raise SystemExit(f"ready_capability_proof:transport_contract_missing:{required}")

register_pos = invocation.find("register_paired_user_runtime_signer(")
flag_pos = invocation.find("PAIRED_USER_RUNTIME_SIGNER.to_string()", register_pos)
if register_pos < 0 or flag_pos < 0 or flag_pos < register_pos:
    raise SystemExit("ready_capability_proof:flag_not_after_signer_registration")
PY
}

check_daemon_local_runtime_identity_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local loop_instance="$cli_root/src/daemon/execution/loop_instance/mod.rs"
  local identity="$cli_root/src/daemon/execution/runtime_identity.rs"
  [[ -f "$daemon" ]] || fail "daemon entrypoint is missing: $daemon"
  [[ -f "$loop_instance" ]] || fail "loop invocation producer is missing: $loop_instance"
  [[ -f "$identity" ]] || fail "local runtime invocation identity object is missing: $identity"

  "$PYTHON_BIN" - "$daemon" "$loop_instance" "$identity" <<'PY'
import re
import sys
from pathlib import Path

daemon = Path(sys.argv[1]).read_text()
loop_instance = Path(sys.argv[2]).read_text()
identity = Path(sys.argv[3]).read_text()
daemon_prod = daemon.split("#[cfg(test)]", 1)[0]
loop_prod = loop_instance.split("#[cfg(test)]", 1)[0]

for source_name, text in (("daemon", daemon_prod), ("loop_instance", loop_prod)):
    for retired in (
        'device_ura("default"',
        'device_ura( "default"',
        'resource_dot_ura("default"',
        'resource_dot_ura( "default"',
        'std::env::var("EASYNET_NODE_ID")',
        'NodeId::new("self")',
    ):
        if retired in text:
            raise SystemExit(f"local_runtime_identity_retired_fork:{source_name}:{retired}")

for required in (
    "pub struct LocalRuntimeInvocationIdentity",
    "pub fn new(realm: impl Into<String>, local_node: NodeId) -> anyhow::Result<Self>",
    "pub fn local_device_ura(&self) -> String",
    "pub fn device_ura_for_node(&self, node_id: &str) -> String",
    "pub fn resource_subject_ura(&self, resource_name: &str, resource_path: &str) -> String",
    "projects_device_and_resource_uras_from_configured_realm",
):
    if required not in identity:
        raise SystemExit(f"local_runtime_identity_object_missing:{required}")

for required in (
    "identity: LocalRuntimeInvocationIdentity",
    "fn invocation_uras(",
    "self.identity.local_device_ura()",
    "kernel_loop_driver_projects_configured_realm_into_invocation_tuple",
):
    if required not in loop_instance:
        raise SystemExit(f"loop_runtime_identity_cutover_missing:{required}")

for required in (
    "fn local_runtime_invocation_identity(",
    "let identity = ready_daemon_identity(config)?;",
    "return Ok(None);",
    "LocalRuntimeInvocationIdentity::new(identity.realm, NodeId::new(node_id)).map(Some)",
    "spawn_schedule_tick(kernel_for_tick, schedule_for_tick, identity)",
    "boot_bus.emit_skipped(\"schedule-tick\")",
    "schedule_tick_invocation_uras(&identity, &entry.target_node, &fire.schedule_id)",
    "schedule_tick_invocation_uras_use_runtime_realm",
    "local_runtime_invocation_identity_uses_paired_credentials_not_env",
):
    if required not in daemon:
        raise SystemExit(f"daemon_runtime_identity_cutover_missing:{required}")
PY
}

check_kernel_runtime_session_read_model_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local kernel="$cli_root/src/daemon/boot/kernel/mod.rs"
  local identity="$cli_root/src/daemon/execution/runtime_identity.rs"
  [[ -f "$kernel" ]] || fail "kernel source is missing: $kernel"
  [[ -f "$identity" ]] || fail "local runtime identity source is missing: $identity"

  "$PYTHON_BIN" - "$kernel" "$identity" <<'PY'
import sys
from pathlib import Path

kernel = Path(sys.argv[1]).read_text()
identity = Path(sys.argv[2]).read_text()
kernel_prod = kernel.split("#[cfg(test)]", 1)[0]

for retired in (
    'node: NodeId::new("self")',
    'tenant: TenantId::default_v1()',
):
    if retired in kernel_prod:
        raise SystemExit(f"kernel_session_read_model_retired_projection:{retired}")

for required in (
    "pub struct LocalRuntimeSessionProjection",
    "pub fn from_callee_ura(callee_ura: &str) -> anyhow::Result<Self>",
    "parsed.kind != crate::core::ura::URAKind::Device",
    "TenantId::new(parsed.realm.clone())",
    "NodeId::new(device_id)",
    "projects_session_read_model_from_device_callee_ura",
    "rejects_non_device_session_read_model_callee",
):
    if required not in identity:
        raise SystemExit(f"kernel_session_projection_identity_missing:{required}")

for required in (
    "runtime_identity::LocalRuntimeSessionProjection",
    "tenant: tenant.clone()",
    "let session_projection = LocalRuntimeSessionProjection::from_callee_ura(&callee)?;",
    "node: session_projection.node().clone()",
    "tenant: session_projection.tenant().clone()",
    "invoke_rejects_non_device_session_projection_without_admitting_row",
):
    if required not in kernel:
        raise SystemExit(f"kernel_session_read_model_cutover_missing:{required}")
PY
}

check_daemon_runtime_session_binding_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local session="$cli_root/src/daemon/execution/session/mod.rs"
  local session_ability="$cli_root/src/daemon/ability/builtins/device_control/session.rs"
  [[ -f "$daemon" ]] || fail "daemon entrypoint is missing: $daemon"
  [[ -f "$session" ]] || fail "session service source is missing: $session"
  [[ -f "$session_ability" ]] || fail "session ability source is missing: $session_ability"

  "$PYTHON_BIN" - "$daemon" "$session" "$session_ability" <<'PY'
import sys
from pathlib import Path

daemon = Path(sys.argv[1]).read_text()
session = Path(sys.argv[2]).read_text()
session_ability = Path(sys.argv[3]).read_text()

def production(text: str) -> str:
    return text.split("#[cfg(test)]", 1)[0]

daemon_prod = production(daemon)
session_prod = production(session)
session_ability_prod = production(session_ability)

for source_name, text in (
    ("daemon", daemon_prod),
    ("session", session_prod),
    ("session_ability", session_ability_prod),
):
    for retired in ('NodeId::new("self")', "TenantId::default_v1()"):
        if retired in text:
            raise SystemExit(f"daemon_runtime_session_binding_retired_default:{source_name}:{retired}")

for required in (
    "let daemon_identity = ready_daemon_identity(&daemon_config)?;",
    "if let Some(node_id) = daemon_identity.node_id",
    "let runtime_node = NodeId::new(node_id);",
    "kernel",
    ".session_service()",
    ".bind_runtime(runtime_node.clone(), tenant.clone())",
):
    if required not in daemon:
        raise SystemExit(f"daemon_runtime_session_binding_boot_missing:{required}")

for required in (
    "struct SessionRuntimeBinding",
    "binding: RwLock<Option<SessionRuntimeBinding>>",
    "pub fn bind_runtime(&self, node: NodeId, tenant: TenantId) -> anyhow::Result<()>",
    "let binding = self.bound_runtime()?;",
    "session.node = binding.node;",
    "session.tenant = binding.tenant;",
    "fn bound_runtime(&self) -> anyhow::Result<SessionRuntimeBinding>",
    "admit_rejects_unbound_runtime_identity",
):
    if required not in session:
        raise SystemExit(f"session_runtime_binding_missing:{required}")
PY
}

check_daemon_runtime_discuss_binding_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local discuss="$cli_root/src/daemon/execution/mission/discuss/mod.rs"
  local discuss_ability="$cli_root/src/daemon/ability/builtins/automation/discuss.rs"
  [[ -f "$daemon" ]] || fail "daemon entrypoint is missing: $daemon"
  [[ -f "$discuss" ]] || fail "discuss service source is missing: $discuss"
  [[ -f "$discuss_ability" ]] || fail "discuss ability source is missing: $discuss_ability"

  "$PYTHON_BIN" - "$daemon" "$discuss" "$discuss_ability" <<'PY'
import sys
from pathlib import Path

daemon = Path(sys.argv[1]).read_text()
discuss = Path(sys.argv[2]).read_text()
discuss_ability = Path(sys.argv[3]).read_text()

def production(text: str) -> str:
    return text.split("#[cfg(test)]", 1)[0]

daemon_prod = production(daemon)
discuss_prod = production(discuss)
discuss_ability_prod = production(discuss_ability)

for source_name, text in (
    ("daemon", daemon_prod),
    ("discuss", discuss_prod),
    ("discuss_ability", discuss_ability_prod),
):
    for retired in ('NodeId::new("self")', "TenantId::default_v1()"):
        if retired in text:
            raise SystemExit(f"daemon_runtime_discuss_binding_retired_default:{source_name}:{retired}")

for required in (
    "let daemon_identity = ready_daemon_identity(&daemon_config)?;",
    "if let Some(node_id) = daemon_identity.node_id",
    "let runtime_node = NodeId::new(node_id);",
    "kernel",
    ".discuss_service()",
    ".bind_runtime(runtime_node, tenant.clone())",
):
    if required not in daemon:
        raise SystemExit(f"daemon_runtime_discuss_binding_boot_missing:{required}")

for required in (
    "struct DiscussRuntimeBinding",
    "binding: RwLock<Option<DiscussRuntimeBinding>>",
    "pub fn bind_runtime(&self, node: NodeId, tenant: TenantId) -> anyhow::Result<()>",
    "let binding = self.bound_runtime()?;",
    "origin_node: binding.node,",
    "tenant: binding.tenant,",
    "fn bound_runtime(&self) -> anyhow::Result<DiscussRuntimeBinding>",
    "create_rejects_unbound_runtime_identity",
):
    if required not in discuss:
        raise SystemExit(f"discuss_runtime_binding_missing:{required}")
PY
}

check_daemon_runtime_tenant_store_binding_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local daemon="$cli_root/src/bin/easynet-daemon.rs"
  local schedule="$cli_root/src/daemon/execution/schedule/mod.rs"
  local loop_instance="$cli_root/src/daemon/execution/loop_instance/mod.rs"
  local schedule_ability="$cli_root/src/daemon/ability/builtins/automation/schedule.rs"
  [[ -f "$daemon" ]] || fail "daemon entrypoint is missing: $daemon"
  [[ -f "$schedule" ]] || fail "schedule service source is missing: $schedule"
  [[ -f "$loop_instance" ]] || fail "loop service source is missing: $loop_instance"
  [[ -f "$schedule_ability" ]] || fail "schedule ability source is missing: $schedule_ability"

  "$PYTHON_BIN" - "$daemon" "$schedule" "$loop_instance" "$schedule_ability" <<'PY'
import sys
from pathlib import Path

daemon = Path(sys.argv[1]).read_text()
schedule = Path(sys.argv[2]).read_text()
loop_instance = Path(sys.argv[3]).read_text()
schedule_ability = Path(sys.argv[4]).read_text()

def production(text: str) -> str:
    return text.split("#[cfg(test)]", 1)[0]

daemon_prod = production(daemon)
schedule_prod = production(schedule)
loop_prod = production(loop_instance)
schedule_ability_prod = production(schedule_ability)

for source_name, text in (
    ("daemon", daemon_prod),
    ("schedule", schedule_prod),
    ("loop_instance", loop_prod),
    ("schedule_ability", schedule_ability_prod),
):
    if "TenantId::default_v1()" in text:
        raise SystemExit(f"daemon_runtime_tenant_store_retired_default:{source_name}")

for required in (
    "TenantId::new(daemon_config.realm().to_string())",
    "kernel.schedule_service().bind(&tenant)",
    "kernel.loop_service().bind(&tenant)",
):
    if required not in daemon:
        raise SystemExit(f"daemon_runtime_tenant_store_boot_binding_missing:{required}")

for required in (
    "pub struct ScheduleCreateSpec",
    "tenant: RwLock<Option<TenantId>>",
    "pub fn add_spec(&self, spec: ScheduleCreateSpec)",
    "fn add_with_bound_tenant(",
    "entry.tenant = tenant;",
    "fn bound_tenant(&self) -> anyhow::Result<TenantId>",
    "add_rejects_unbound_runtime_tenant",
):
    if required not in schedule:
        raise SystemExit(f"schedule_runtime_tenant_binding_missing:{required}")

for required in (
    "tenant: RwLock<Option<TenantId>>",
    "let tenant = self.bound_tenant()?;",
    "fn bound_tenant(&self) -> anyhow::Result<TenantId>",
    "create_rejects_unbound_runtime_tenant",
):
    if required not in loop_instance:
        raise SystemExit(f"loop_runtime_tenant_binding_missing:{required}")

for required in (
    "ScheduleCreateSpec::new(",
    ".with_catch_up_window_secs(catch_up_window_secs)",
    ".with_enabled(enabled)",
    ".with_prompt(prompt)",
    "svc.add_spec(spec)?",
):
    if required not in schedule_ability:
        raise SystemExit(f"schedule_ability_runtime_tenant_spec_missing:{required}")
PY
}

check_schedule_store_current_schema_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local schedule="$cli_root/src/daemon/execution/schedule/mod.rs"
  local store="$cli_root/src/daemon/execution/schedule/store.rs"
  [[ -f "$schedule" ]] || fail "schedule service source is missing: ${schedule#$cli_root/}"
  [[ -f "$store" ]] || fail "schedule store source is missing: ${store#$cli_root/}"

  "$PYTHON_BIN" - "$schedule" "$store" <<'PY'
import sys
from pathlib import Path

schedule = Path(sys.argv[1]).read_text()
store = Path(sys.argv[2]).read_text()
store_prod = store.split("#[cfg(test)]", 1)[0]

for retired in (
    "default_schema_version",
    "#[serde(default = \"default_schema_version\")]",
    "readers tolerate a missing field",
    "absent as `1`",
):
    if retired in store_prod:
        raise SystemExit(f"schedule_store_legacy_schema_version_default:{retired}")

for retired in (
    "Legacy entry without the prompt field should parse",
    "pre-prompt remain readable",
):
    if retired in schedule:
        raise SystemExit(f"schedule_entry_legacy_prompt_read_test:{retired}")

for required in (
    "fn parse_on_disk_schedule(",
    "schedule record missing explicit schema_version",
    "schedule record missing explicit prompt field",
    "schema_version {schema_version} is not supported",
    "fn serialize_on_disk_schedule(",
    ".entry(\"prompt\".to_string())",
    ".or_insert(serde_json::Value::Null)",
):
    if required not in store:
        raise SystemExit(f"schedule_store_current_schema_missing:{required}")

for required_test in (
    "load_all_skips_records_missing_current_schema_facts",
    "parse_on_disk_schedule_rejects_unsupported_schema_version",
):
    if required_test not in store:
        raise SystemExit(f"schedule_store_current_schema_missing_test:{required_test}")
PY
}

check_mission_runtime_meta_identity_schema_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local orchestration="$cli_root/src/daemon/execution/mission/orchestration.rs"
  local run_store="$cli_root/src/daemon/execution/mission/run_store.rs"
  local persisted_identity="$cli_root/src/daemon/execution/mission/persisted_identity.rs"
  [[ -f "$orchestration" ]] || fail "mission orchestration source is missing: ${orchestration#$cli_root/}"
  [[ -f "$run_store" ]] || fail "mission run store source is missing: ${run_store#$cli_root/}"
  [[ -f "$persisted_identity" ]] || fail "mission persisted identity source is missing: ${persisted_identity#$cli_root/}"

  "$PYTHON_BIN" - "$orchestration" "$run_store" "$persisted_identity" <<'PY'
import re
import sys
from pathlib import Path

orchestration = Path(sys.argv[1]).read_text()
run_store = Path(sys.argv[2]).read_text()
persisted_identity = Path(sys.argv[3]).read_text()
trace_match = re.search(r"(?s)((?:#\[[^\]]*\]\s*)*)pub trace_id: String,", orchestration)
if not trace_match:
    raise SystemExit("mission_meta_trace_id_field_missing")
if "#[serde(default)]" in trace_match.group(0):
    raise SystemExit("mission_meta_trace_id_legacy_default")
if "deserialize_non_empty_string" not in trace_match.group(0):
    raise SystemExit("mission_meta_trace_id_not_non_empty_validated")

invocation_match = re.search(r"(?s)((?:#\[[^\]]*\]\s*)*)pub invocation_id: String,", run_store)
if not invocation_match:
    raise SystemExit("run_meta_invocation_id_field_missing")
if "#[serde(default" in invocation_match.group(0):
    raise SystemExit("run_meta_invocation_id_legacy_default")
if "skip_serializing_if" in invocation_match.group(0):
    raise SystemExit("run_meta_invocation_id_skip_empty_serialization")
if "deserialize_non_empty_string" not in invocation_match.group(0):
    raise SystemExit("run_meta_invocation_id_not_non_empty_validated")

for required in (
    "pub(crate) fn deserialize_non_empty_string",
    "runtime identity fact must be a non-empty string",
    "required_identity_rejects_empty_string",
):
    if required not in persisted_identity:
        raise SystemExit(f"mission_persisted_identity_validator_missing:{required}")

for retired in (
    "pre_trace_id_meta_still_deserializes",
    "legacy meta parses",
    "absent field defaults",
    "backward-compatible with meta.json",
    "written before this field existed",
):
    if retired in orchestration or retired in run_store:
        raise SystemExit(f"runtime_meta_identity_legacy_contract:{retired}")

for required in (
    "mission_run_meta_requires_trace_id_identity_fact",
    "mission_run_meta_rejects_empty_trace_id_identity_fact",
    "missing field `trace_id`",
):
    if required not in orchestration:
        raise SystemExit(f"mission_meta_identity_schema_missing:{required}")

for required in (
    "run_meta_requires_invocation_id_identity_fact",
    "run_meta_rejects_empty_invocation_id_identity_fact",
    "missing field `invocation_id`",
):
    if required not in run_store:
        raise SystemExit(f"run_meta_identity_schema_missing:{required}")
PY
}

check_mission_terminal_receipt_projection_contract() {
  local cli_root="${1:-${CLI_ROOT:-$ROOT}}"
  local gateway="$cli_root/src/daemon/execution/mission/invocation_gateway.rs"
  [[ -f "$gateway" ]] || fail "mission invocation gateway source is missing: ${gateway#$cli_root/}"

  "$PYTHON_BIN" - "$gateway" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
if "pub(crate) fn projection(&self) -> Value" not in text:
    raise SystemExit("mission_terminal_receipt_projection:projection_missing")
projection = text.split("pub(crate) fn projection(&self) -> Value", 1)[1].split("#[cfg(test)]", 1)[0]
if '"receipt": {"anchor": self.terminal_receipt.projection()}' in projection:
    raise SystemExit("mission_terminal_receipt_projection:retired_receipt_anchor_wrapper")
if '"terminal_receipt": self.terminal_receipt.projection()' not in projection:
    raise SystemExit("mission_terminal_receipt_projection:terminal_receipt_field_missing")
for required in (
    '"dependency_receipts"',
    "child_is_receipt_anchored_and_inherits_subject_trace_and_parent_deadline",
    'invocation_record.get("receipt").is_none()',
    'invocation_record["terminal_receipt"]["receipt_ura"]',
    'invocation_record["terminal_receipt"]["receipt_hash"]',
):
    if required not in text:
        raise SystemExit(f"mission_terminal_receipt_projection:missing:{required}")
PY
}

check_retired_federation_directory_v1_stream_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  "$PYTHON_BIN" - "$cli_root/src" "$cli_root/tests" "$cli_root/ability-descriptors/system/federation" <<'PY'
import re
import sys
from pathlib import Path

roots = [Path(arg) for arg in sys.argv[1:]]
retired_symbol_patterns = {
    r"\bABILITY_FEDERATION_SUBSCRIBE_DIRECTORY\b(?!_V2)": "retired_v1_ability_constant",
    r"\bfederation\.subscribe_directory\b(?!_v2)": "retired_v1_ability_name",
    r"\bSubscribeDirectoryInitial\b": "retired_v1_snapshot_dto",
    r"\bPresenceEventDelta\b": "retired_v1_delta_dto",
    r"\bbuild_subscribe_directory_initial\b": "retired_v1_snapshot_builder",
}

for root in roots:
    if not root.exists():
        continue
    for path in list(root.rglob("*.rs")) + list(root.rglob("*.toml")):
        if "target" in path.relative_to(root).parts:
            continue
        if path.name == "federation.subscribe_directory_v2.ability.toml":
            continue
        if path.name == "federation.subscribe_directory.ability.toml":
            raise SystemExit(
                f"federation_directory_v1_stream_retired:retired_v1_descriptor:{path}:1"
            )
        text = path.read_text(encoding="utf-8")
        for pattern, label in retired_symbol_patterns.items():
            match = re.search(pattern, text)
            if match:
                line = text.count("\n", 0, match.start()) + 1
                raise SystemExit(
                    f"federation_directory_v1_stream_retired:{label}:{path}:{line}"
                )
PY
}

check_route_resolver_descriptor_ref_selector_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local route_resolver="$cli_root/src/daemon/invocation/routing/route_resolver.rs"
  [[ -f "$route_resolver" ]] || fail "route resolver source is missing: $route_resolver"

  "$PYTHON_BIN" - "$route_resolver" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
production = text.split("\n#[cfg(test)]\nmod tests", 1)[0]

def function_slice(name: str, next_name: str | None = None) -> tuple[str, str]:
    marker = f"fn {name}"
    start = production.find(marker)
    if start < 0:
        raise SystemExit(f"route_resolver:{name}:missing")
    brace = production.find("{", start)
    if brace < 0:
        raise SystemExit(f"route_resolver:{name}:body_missing")
    signature = production[start:brace]
    if next_name is not None:
        next_marker = f"\nfn {next_name}"
        end = production.find(next_marker, brace)
        if end < 0:
            end = len(production)
    else:
        end = len(production)
    return signature, production[brace:end]

route_signature, route_body = function_slice(
    "route_selector_from_query",
    "route_selector_from_descriptor_ref",
)
descriptor_signature, descriptor_body = function_slice(
    "ability_selector_from_descriptor_ref",
    "selected_execution_for_owner",
)

if "Result<Option<RouteSelector>, ResolveRouteFailure>" not in route_signature:
    raise SystemExit("route_resolver:descriptor_ref_selector:route_selector_not_fallible")
for required_selector_state in (
    "owner_kind: RouteOwnerKind",
    "enum RouteOwnerKind",
    "fn route_selector_from_ability_selector(",
    "RouteOwnerKind::from_ability_selector",
):
    if required_selector_state not in production:
        raise SystemExit(f"route_resolver:descriptor_ref_selector:owner_kind_state_missing:{required_selector_state}")
if "ability_selector_from_descriptor_ref(query_name)?" not in route_body:
    raise SystemExit("route_resolver:descriptor_ref_selector:query_parse_not_propagated")
if "route_selector_from_descriptor_ref(owner_ura, ability_name).map(Some)" not in route_body:
    raise SystemExit("route_resolver:descriptor_ref_selector:owner_parse_not_propagated")
if "Result<crate::core::ura::AbilitySelector, ResolveRouteFailure>" not in descriptor_signature:
    raise SystemExit("route_resolver:descriptor_ref_selector:descriptor_selector_not_fallible")

compact_descriptor = re.sub(r"\s+", "", descriptor_body)
legacy_patterns = {
    "canonical_ability_descriptor_ref(descriptor_ref).ok()": "canonicalization_none",
    "ability_ura_from_descriptor_ref(&descriptor_ref).ok()": "ability_extraction_none",
    ".ok()?": "option_question_fallback",
    "AbilitySelector::parse(&ability_ura).ok()": "selector_parse_none",
}
for pattern, label in legacy_patterns.items():
    if re.sub(r"\s+", "", pattern) in compact_descriptor:
        raise SystemExit(f"route_resolver:descriptor_ref_selector:{label}")

compact_production = re.sub(r"\s+", "", production)
for pattern, label in {
    "parse_ura(&selector.owner_ura).ok()": "selector_owner_parse_none",
    "parse_ura(&selector.owner_ura).map(|parsed| parsed.kind == crate::core::ura::URAKind::Agent).unwrap_or(false)": "selector_owner_kind_default_false",
}.items():
    if re.sub(r"\s+", "", pattern) in compact_production:
        raise SystemExit(f"route_resolver:descriptor_ref_selector:{label}")

for required_test in (
    "malformed_descriptor_ref_does_not_fall_through_as_public_name",
    "descriptor_ref_owner_mismatch_fails_before_route_lookup",
    "route_selector_carries_owner_kind_from_ability_selector",
):
    if required_test not in text:
        raise SystemExit(f"route_resolver:descriptor_ref_selector:missing_test:{required_test}")
PY
}

check_namespace_resolver_authority_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local route_resolver="$cli_root/src/daemon/invocation/routing/route_resolver.rs"
  local federation_wrappers="$cli_root/src/daemon/invocation/dispatch/federation_wrappers.rs"
  [[ -f "$route_resolver" ]] || fail "route resolver source is missing: $route_resolver"
  [[ -f "$federation_wrappers" ]] || fail "federation wrappers source is missing: $federation_wrappers"

  "$PYTHON_BIN" - "$route_resolver" "$federation_wrappers" <<'PY'
import sys
from pathlib import Path

route_path, wrappers_path = map(Path, sys.argv[1:])
route_text = route_path.read_text(encoding="utf-8")
wrappers_text = wrappers_path.read_text(encoding="utf-8")
route_production = route_text.split("\n#[cfg(test)]\nmod tests", 1)[0]
wrappers_production = wrappers_text.split("\n#[cfg(test)]\nmod tests", 1)[0]

def function_body(text: str, name: str) -> str:
    marker = f"fn {name}"
    start = text.find(marker)
    if start < 0:
        raise SystemExit(f"namespace_resolver_authority_projection:{name}:missing")
    brace = text.find("{", start)
    if brace < 0:
        raise SystemExit(f"namespace_resolver_authority_projection:{name}:body_missing")
    depth = 0
    for index in range(brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[brace:index + 1]
    raise SystemExit(f"namespace_resolver_authority_projection:{name}:unterminated")

authority_body = function_body(route_production, "authority_for_query")
realm_body = function_body(route_production, "authority_realm_for_query")
wrapper_body = function_body(wrappers_production, "namespace_resolve_input_failure")
combined_production = "\n".join((route_production, wrappers_production))

required_route_fragments = {
    "pub(crate) fn authority_for_query(": "authority_helper_not_shared",
    "fn authority_realm_for_query(": "realm_projection_helper_missing",
    'strip_prefix("route-ref::")': "route_ref_realm_projection_missing",
    "ability_ura_from_descriptor_ref(candidate)": "descriptor_ref_realm_projection_missing",
    '"query_name_unavailable"': "unavailable_authority_state_missing",
    '"daemon-local-unavailable"': "unavailable_authority_algorithm_missing",
}
for fragment, label in required_route_fragments.items():
    if fragment not in route_production:
        raise SystemExit(f"namespace_resolver_authority_projection:{label}")

legacy_authority_patterns = {
    'unwrap_or_else(|| "localhost".to_string())': "localhost_string_fallback",
    'unwrap_or_else(|| "localhost")': "localhost_str_fallback",
    'unwrap_or("localhost")': "localhost_literal_fallback",
}
for pattern, label in legacy_authority_patterns.items():
    if pattern in authority_body or pattern in realm_body:
        raise SystemExit(f"namespace_resolver_authority_projection:route_resolver:{label}")
    if pattern in wrapper_body:
        raise SystemExit(f"namespace_resolver_authority_projection:federation_wrapper:{label}")

if "route_resolver::authority_for_query(query_name)" not in wrapper_body:
    raise SystemExit("namespace_resolver_authority_projection:wrapper_not_using_shared_authority_helper")
for forbidden in ("parse_ura(query_name)", '"localhost"'):
    if forbidden in wrapper_body:
        raise SystemExit(f"namespace_resolver_authority_projection:wrapper_legacy_authority:{forbidden}")

for required_test in (
    "authority_projection_uses_route_ref_embedded_ability_realm",
    "authority_projection_uses_descriptor_ref_embedded_ability_realm",
    "authority_projection_does_not_default_invalid_query_to_localhost",
    "namespace_resolve_input_failure_does_not_fabricate_localhost_authority",
):
    if required_test not in route_text and required_test not in wrappers_text:
        raise SystemExit(f"namespace_resolver_authority_projection:missing_test:{required_test}")

if '"query_name is not a canonical URA, route-ref, or descriptor ref"' not in combined_production:
    raise SystemExit("namespace_resolver_authority_projection:unavailable_reason_not_explicit")
PY
}

check_daemon_invocation_service_descriptor_ref_route_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local service="$cli_root/src/daemon/invocation/dispatch/daemon_invocation_service.rs"
  local tests="$cli_root/src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"
  local descriptor_ref="$cli_root/src/daemon/axon_bridge/descriptor_ref.rs"
  [[ -f "$service" ]] || fail "daemon invocation service source is missing: $service"
  [[ -f "$tests" ]] || fail "daemon invocation service tests are missing: $tests"
  [[ -f "$descriptor_ref" ]] || fail "descriptor_ref bridge source is missing: $descriptor_ref"

  "$PYTHON_BIN" - "$service" "$tests" "$descriptor_ref" <<'PY'
import re
import sys
from pathlib import Path

service_path, tests_path, descriptor_ref_path = map(Path, sys.argv[1:])
service = service_path.read_text(encoding="utf-8")
tests = tests_path.read_text(encoding="utf-8")
descriptor_ref = descriptor_ref_path.read_text(encoding="utf-8")
production = service.split("\n#[cfg(test)]", 1)[0]

def function_slice(text: str, name: str, next_name: str | None = None) -> tuple[str, str]:
    marker = f"fn {name}"
    start = text.find(marker)
    if start < 0:
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:{name}:missing")
    brace = text.find("{", start)
    if brace < 0:
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:{name}:body_missing")
    signature = text[start:brace]
    if next_name is not None:
        end = text.find(f"\nfn {next_name}", brace)
        if end < 0:
            end = len(text)
    else:
        end = len(text)
    return signature, text[brace:end]

dispatch_signature, dispatch_body = function_slice(
    production,
    "dispatch_function_name_for_route_table",
    "descriptor_ref_public_name_for_callee",
)
projection_signature, projection_body = function_slice(
    production,
    "descriptor_ref_public_name_for_callee",
    "is_descriptor_ref_route_token",
)

if "Result<String, Status>" not in dispatch_signature:
    raise SystemExit("daemon_invocation_service_descriptor_projection:dispatch_not_fallible")
if "is_descriptor_ref_route_token(function_name)" not in dispatch_body:
    raise SystemExit("daemon_invocation_service_descriptor_projection:descriptor_token_gate_missing")
if "Result<String, Status>" not in projection_signature:
    raise SystemExit("daemon_invocation_service_descriptor_projection:projection_not_fallible")
for required in (
    "ability_selector_from_descriptor_ref(",
    "descriptor_ref selector projection failed",
    "does not match envelope callee",
):
    if required not in projection_body:
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:missing:{required}")
compact_projection = re.sub(r"\s+", "", projection_body)
for pattern, label in {
    "canonical_ability_descriptor_ref(function_name).ok()?": "canonicalization_none",
    "ability_ura_from_descriptor_ref(&descriptor_ref).ok()?": "ability_extraction_none",
    "AbilitySelector::parse(&ability_ura).ok()?": "selector_parse_none",
    ".ok()?": "option_question_fallback",
}.items():
    if re.sub(r"\s+", "", pattern) in compact_projection:
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:{label}")

compact_service = re.sub(r"\s+", "", service)
if "descriptor_ref_route_projection" not in service:
    raise SystemExit("daemon_invocation_service_descriptor_projection:audit_stage_missing")
if "return Err(status);" not in service:
    raise SystemExit("daemon_invocation_service_descriptor_projection:callsite_return_missing")
for required, haystack in (
    ("dispatch_function_name_for_route_table(function, inner.envelope.as_ref())", service),
    ("dispatch_function_name_for_route_table(ability_name,envelope_open.envelope.as_ref(),)", compact_service),
    ("DaemonBidiRoute::from_function(&route_function)", service),
    ("dispatcher.dispatch(&route_function, envelope_open, up)", service),
):
    if re.sub(r"\s+", "", required) not in re.sub(r"\s+", "", haystack):
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:callsite_missing:{required}")

if "pub(crate) fn ability_selector_from_descriptor_ref(" not in descriptor_ref:
    raise SystemExit("daemon_invocation_service_descriptor_projection:shared_helper_missing")
for required_test in (
    "route_table_projects_hub_bidi_descriptor_ref_to_session_open",
    "route_table_rejects_malformed_descriptor_ref_before_name_fallback",
    "route_table_rejects_descriptor_ref_owner_mismatch_before_name_fallback",
):
    if required_test not in tests:
        raise SystemExit(f"daemon_invocation_service_descriptor_projection:missing_test:{required_test}")
PY
}

check_ffi_descriptor_runtime_owner_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ffi_invocation="$cli_root/src/ffi/invocation/mod.rs"
  [[ -f "$ffi_invocation" ]] || return 0

  "$PYTHON_BIN" - "$ffi_invocation" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
production = text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]

resolve = re.search(
    r"fn runtime_resolve_descriptor_ref_json\([^)]*\)\s*->\s*Result<serde_json::Value,\s*DescriptorResolutionError>\s*\{(?P<body>.*?)\n\}\n\n#\[cfg\(feature = \"axon-pb\"\)\]\nfn runtime_system_descriptor_catalog_entries",
    text,
    re.S,
)
if resolve is None:
    raise SystemExit("ffi_descriptor_runtime_owner:resolve_function_missing")
resolve_body = resolve.group("body")
if "runtime_owner_ura_from_session(session).ok()" in resolve_body:
    raise SystemExit("ffi_descriptor_runtime_owner:runtime_owner_error_collapsed")
if "resolve descriptor_ref runtime owner" not in resolve_body:
    raise SystemExit("ffi_descriptor_runtime_owner:runtime_owner_error_context_missing")
if "RemoteSystemInvocationIssuer::root_plan(" in resolve_body:
    raise SystemExit("ffi_descriptor_runtime_owner:remote_probe_inline_plan")
if "invoke_remote_target)" in resolve_body or ".and_then(remote_invoke::invoke_remote_target)" in resolve_body:
    raise SystemExit("ffi_descriptor_runtime_owner:remote_probe_implicit_invoke")
for retired in (
    'descriptor_ref_request_required_string(object, "caller_ura")',
    "RemoteDescriptorCatalogProbe",
    "DescriptorCatalogProbeSubject",
    "RemoteInvocationCallerSigner",
    "load_remote_invocation_caller_signer(",
    "invoke_remote_target_with_caller_signer_typed(",
    "runtime_meta_descriptor_catalog_entries",
    "descriptor_catalog_entry_from_value",
    "runtime_meta_list_abilities",
):
    if retired in production:
        raise SystemExit(f"ffi_descriptor_runtime_owner:retired_remote_probe_path:{retired}")
if "descriptor_ref not found in runtime realm catalog" not in resolve_body:
    raise SystemExit("ffi_descriptor_runtime_owner:realm_catalog_miss_error_missing")
if "target_owned_descriptor_catalog_subject_ura" in production:
    raise SystemExit("ffi_descriptor_runtime_owner:retired_target_owned_subject_helper")

if "fn descriptor_resolution_error_projection(" in production:
    raise SystemExit("ffi_descriptor_runtime_owner:retired_message_projection_classifier")
for retired in (
    "from_remote_probe_failure",
    "lowered.contains",
    'contains("owner is not online")',
    'contains("NEGATIVE_REASON_NXDOMAIN")',
    'contains("ROUTE_NEGATIVE")',
    'contains("requires a caller signer")',
    "CallerSignerUnavailable",
    "OwnerOffline",
    "RuntimeOffline",
):
    if retired in production:
        raise SystemExit(f"ffi_descriptor_runtime_owner:retired_remote_probe_classifier:{retired}")
for required in (
    "enum DescriptorResolutionError",
    "RuntimeOwnerUnavailable(String)",
    "DescriptorNotFound(String)",
    "fn abi_projection(&self) -> (i32, ErrorProjection)",
    'code: "CALLER_IDENTITY_UNAVAILABLE"',
    'code: "DESCRIPTOR_NOT_FOUND"',
    "error.abi_projection()",
):
    if required not in text:
        raise SystemExit(f"ffi_descriptor_runtime_owner:typed_projection_missing:{required}")
entry = text[text.find("pub unsafe extern \"C\" fn runtime_resolve_descriptor_ref("):]
entry = entry[: entry.find("/// Allocate a mutable Invocation builder handle.") if "/// Allocate a mutable Invocation builder handle." in entry else len(entry)]
if "descriptor_resolution_error_projection(&message)" in entry:
    raise SystemExit("ffi_descriptor_runtime_owner:ffi_entry_uses_message_classifier")
if "format!(\"runtime_resolve_descriptor_ref: {error:#}\")" in entry:
    raise SystemExit("ffi_descriptor_runtime_owner:ffi_entry_formats_error_before_projection")

for required_test in (
    "runtime_descriptor_resolver_requires_runtime_owner_for_realm_catalog",
    "runtime_descriptor_resolver_does_not_remote_probe_realm_catalog_miss",
    "descriptor_resolution_errors_project_canonical_runtime_codes",
):
    if required_test not in text:
        raise SystemExit(f"ffi_descriptor_runtime_owner:missing_test:{required_test}")
PY
}

check_ffi_descriptor_probe_not_found_vocabulary_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ffi_invocation="$cli_root/src/ffi/invocation/mod.rs"
  [[ -f "$ffi_invocation" ]] || return 0

  "$PYTHON_BIN" - "$ffi_invocation" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
production = text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]

for retired in (
    "fn from_remote_probe_rejection(",
    "RemoteInvocationFailure::",
    "Self::OwnerOffline(",
    '"ROUTE_NEGATIVE"',
    '"DESCRIPTOR_OWNER_OFFLINE"',
    '"NOT_FOUND"',
):
    if retired in production:
        raise SystemExit(f"ffi_descriptor_probe_not_found_vocabulary:retired_remote_probe_classifier:{retired}")
if "descriptor_ref not found in runtime realm catalog" not in production:
    raise SystemExit("ffi_descriptor_probe_not_found_vocabulary:realm_catalog_miss_missing")
for required_test in (
    "descriptor_resolution_errors_project_canonical_runtime_codes",
    "runtime_descriptor_resolver_does_not_remote_probe_realm_catalog_miss",
):
    if required_test not in text:
        raise SystemExit(f"ffi_descriptor_probe_not_found_vocabulary:missing_test:{required_test}")
PY
}

check_cli_discover_candidate_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local discover="$cli_root/src/cli/commands/discover.rs"
  local e2e="$cli_root/tests/seven_axes_w1_discover_e2e.rs"
  [[ -f "$discover" ]] || fail "CLI discover source is missing: $discover"
  [[ -f "$e2e" ]] || fail "CLI discover e2e source is missing: $e2e"

  "$PYTHON_BIN" - "$discover" "$e2e" <<'PY'
import re
import sys
from pathlib import Path

discover_path, e2e_path = map(Path, sys.argv[1:])
discover = discover_path.read_text(encoding="utf-8")
e2e = e2e_path.read_text(encoding="utf-8")
production = discover.split("\n#[cfg(test)]", 1)[0]

if "skipped_unparseable" in discover or "skipped_unparseable" in e2e:
    raise SystemExit("cli_discover_candidate_projection:retired_skipped_counter")
if "pub skipped_unparseable" in production:
    raise SystemExit("cli_discover_candidate_projection:report_legacy_counter_field")
if "pub diagnostics: Vec<DiscoverDiagnostic>" not in production:
    raise SystemExit("cli_discover_candidate_projection:typed_diagnostics_missing")
if "struct DiscoverCandidateRow" not in production:
    raise SystemExit("cli_discover_candidate_projection:typed_row_parser_missing")
if "fn parse(row: &Value) -> anyhow::Result<Self>" not in production:
    raise SystemExit("cli_discover_candidate_projection:row_parser_parse_missing")
parser_start = production.find("impl DiscoverCandidateRow")
candidate_start = production.find("impl Candidate")
if parser_start < 0 or candidate_start < 0:
    raise SystemExit("cli_discover_candidate_projection:parser_slice_missing")
parser_body = production[parser_start:candidate_start]
candidate_body = production[candidate_start:production.find("/// Typed projection", candidate_start)]
if 'let scope = required_row_string(row, "scope_matched")?;' not in production:
    raise SystemExit("cli_discover_candidate_projection:scope_not_required")
for forbidden, label in {
    '.unwrap_or("device")': "scope_literal_default",
    '.unwrap_or_else(|| "device"': "scope_lazy_default",
    '.and_then(Value::as_bool)': "callable_type_downgrade",
}.items():
    if forbidden in parser_body or forbidden in candidate_body:
        raise SystemExit(f"cli_discover_candidate_projection:{label}")
for required in (
    "fn optional_row_bool(",
    "discover candidate row field {field} must be a boolean",
    "fn required_row_string(",
    "discover candidate row missing non-empty {field}",
):
    if required not in production:
        raise SystemExit(f"cli_discover_candidate_projection:missing:{required}")
for required_test in (
    "ladder_row_missing_scope_fails_closed_instead_of_defaulting_to_device",
    "ladder_row_malformed_callable_fails_closed_instead_of_downgrading",
):
    if required_test not in discover:
        raise SystemExit(f"cli_discover_candidate_projection:missing_test:{required_test}")
if "candidate projection defects must fail closed or surface as typed diagnostics" not in e2e:
    raise SystemExit("cli_discover_candidate_projection:e2e_typed_diagnostic_assertion_missing")
PY
}

check_ffi_invocation_json_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ffi_invocation="$cli_root/src/ffi/invocation/mod.rs"
  [[ -f "$ffi_invocation" ]] || fail "FFI invocation source is missing: $ffi_invocation"

  "$PYTHON_BIN" - "$ffi_invocation" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
production = text.split("\n#[cfg(all(test, feature = \"axon-pb\"))]\nmod tests", 1)[0]

retired_patterns = {
    "serde_json::from_slice::<serde_json::Value>(&result.output).ok()": "unary_output_json_parse_downgrade",
    "serde_json::from_slice::<serde_json::Value>(&chunk.payload).ok()": "stream_payload_json_parse_downgrade",
}
for pattern, label in retired_patterns.items():
    if pattern in production:
        raise SystemExit(f"ffi_invocation_json_projection:{label}")

if "fn runtime_json_projection(" not in production:
    raise SystemExit("ffi_invocation_json_projection:shared_projection_helper_missing")
if "payload is not valid JSON" not in production:
    raise SystemExit("ffi_invocation_json_projection:declared_json_error_missing")
if not re.search(
    r"fn invocation_outcome_json_with_tuple\([^)]*\)\s*->\s*Result<serde_json::Value,\s*String>",
    production,
    re.S,
):
    raise SystemExit("ffi_invocation_json_projection:unary_projection_not_fallible")
if "runtime_json_projection(&chunk.payload, &chunk.content_type, \"payload_json\")?" not in production:
    raise SystemExit("ffi_invocation_json_projection:stream_projection_not_shared")
if "runtime_json_projection(&result.output, &result.output_content_type, \"output_json\")?" not in production:
    raise SystemExit("ffi_invocation_json_projection:unary_projection_not_shared")
for required in (
    "fn validate_public_invocation_tuple(",
    "validate_public_invocation_tuple(&caller_ura, &callee_ura, &subject_ura, &metadata)?",
    "project_invocation_authority_metadata_shape(metadata)",
    "session_authority_admits_subject(&payload, subject_ura)",
    "AuthoritySubjectMismatch",
    "AllZeroPrincipal",
):
    if required not in production:
        raise SystemExit(f"ffi_invocation_json_projection:public_tuple_gate_missing:{required}")
for required_test in (
    "unary_result_json_rejects_declared_json_output_that_is_not_json",
    "stream_chunk_json_rejects_declared_json_payload_that_is_not_json",
    "parse_invocation_json_rejects_all_zero_subject_before_daemon_io",
    "parse_invocation_json_rejects_session_authority_subject_mismatch_before_daemon_io",
):
    if required_test not in text:
        raise SystemExit(f"ffi_invocation_json_projection:missing_test:{required_test}")
PY
}

check_ffi_last_error_typed_tls_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ffi_errors="$cli_root/src/ffi/errors/mod.rs"
  [[ -f "$ffi_errors" ]] || fail "FFI error source is missing: $ffi_errors"

  "$PYTHON_BIN" - "$ffi_errors" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")

record = re.search(r"struct LastErrorRecord \{(?P<body>.*?)\n\}", text, re.S)
if record is None:
    raise SystemExit("ffi_last_error_typed_tls:record_missing")
record_body = record.group("body")
if re.search(r"\bcode:\s*Option\s*<\s*i32\s*>", record_body):
    raise SystemExit("ffi_last_error_typed_tls:optional_abi_code")
if not re.search(r"\bcode:\s*i32\b", record_body):
    raise SystemExit("ffi_last_error_typed_tls:mandatory_abi_code_missing")

retired = {
    "pub(crate) fn set_last_error(": "raw_text_setter",
    "fn set_last_error(": "raw_text_setter",
    "set_last_error_record(None": "untyped_record_write",
    "typed_error_json(Some(": "optional_projection_api",
    "typed_error_json_with_projection(Some(": "optional_projection_api",
    "code.unwrap_or(ERR_GENERIC)": "generic_projection_fallback",
    "last_error_json_projects_legacy_message_as_generic": "legacy_projection_test",
    "last_error_message().unwrap_or_default()": "explicit_code_reads_tls_message",
}
for pattern, label in retired.items():
    if pattern in text:
        raise SystemExit(f"ffi_last_error_typed_tls:{label}")

for required in (
    "set_last_error_code(ERR_INVALID_HANDLE, \"bad handle\")",
    "set_last_error_code(ERR_GENERIC, \"a\\0b\\0c\")",
    "error_json_null_message_does_not_read_tls_last_error",
    "#[cfg(test)]\nfn last_error_message()",
    "fn typed_error_json(code: i32, message: &str)",
    "fn typed_error_json_with_projection(",
):
    if required not in text:
        raise SystemExit(f"ffi_last_error_typed_tls:missing_typed_path:{required}")
PY
}

check_canonical_ability_catalog_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local descriptor="$cli_root/src/daemon/ability/descriptors/surface.rs"
  local store="$cli_root/src/daemon/federation/read_model/hub_published_abilities.rs"
  local meta="$cli_root/src/daemon/ability/builtins/governance/meta.rs"
  local ffi_invocation="$cli_root/src/ffi/invocation/mod.rs"
  local cli_catalog="$cli_root/src/cli/daemon_client/ability_catalog.rs"
  local cli_ability="$cli_root/src/cli/commands/groups/ability.rs"
  [[ -f "$descriptor" ]] || fail "AbilityDescriptor source is missing: $descriptor"
  [[ -f "$store" ]] || fail "hub-published ability store is missing: $store"
  [[ -f "$meta" ]] || fail "meta.list_abilities source is missing: $meta"
  [[ -f "$ffi_invocation" ]] || fail "FFI invocation source is missing: $ffi_invocation"
  [[ -f "$cli_catalog" ]] || fail "CLI ability catalogue client is missing: $cli_catalog"
  [[ -f "$cli_ability" ]] || fail "CLI ability command source is missing: $cli_ability"

  "$PYTHON_BIN" - "$descriptor" "$store" "$meta" "$ffi_invocation" "$cli_catalog" "$cli_ability" <<'PY'
import re
import sys
from pathlib import Path

descriptor = Path(sys.argv[1]).read_text(encoding="utf-8")
store = Path(sys.argv[2]).read_text(encoding="utf-8")
meta = Path(sys.argv[3]).read_text(encoding="utf-8")
ffi = Path(sys.argv[4]).read_text(encoding="utf-8")
cli_catalog = Path(sys.argv[5]).read_text(encoding="utf-8")
cli_ability = Path(sys.argv[6]).read_text(encoding="utf-8")

descriptor_production = descriptor.split("\n#[cfg(test)]\nmod tests", 1)[0]
production_store = store.split("\n#[cfg(test)]\nmod tests", 1)[0]
cli_catalog_production = cli_catalog.split("\n#[cfg(test)]\nmod tests", 1)[0]
cli_ability_production = cli_ability.split("\n#[cfg(test)]\nmod tests", 1)[0]
if "pub fn descriptor_ref(&self) -> Result<String, DescriptorError>" not in descriptor_production:
    raise SystemExit("ability_descriptor:descriptor_ref_not_fallible")
if "InvalidDescriptorIdentity" not in descriptor_production:
    raise SystemExit("ability_descriptor:descriptor_identity_error_missing")
if ".descriptor_ref().ok()" in descriptor_production or ".descriptor_ref().is_none()" in descriptor_production:
    raise SystemExit("ability_descriptor:descriptor_ref_optional_collapse")
if "descriptor_ref_derivation_fails_closed_for_corrupt_identity" not in descriptor:
    raise SystemExit("ability_descriptor:descriptor_ref_fail_closed_test_missing")
if "entries: BTreeMap<String, AbilityDescriptor>" not in production_store:
    raise SystemExit("hub_published_store:entries_not_canonical_descriptor")
if "entries: BTreeMap<String, HubAbilityEntry>" in production_store:
    raise SystemExit("hub_published_store:opaque_entry_cache")
if "fn validate_hub_ability_entry" not in production_store:
    raise SystemExit("hub_published_store:validation_boundary_missing")
if "serde_json::from_value(entry.descriptor)" not in production_store:
    raise SystemExit("hub_published_store:descriptor_parse_missing")
if "descriptor.descriptor_ref().map_err" not in production_store:
    raise SystemExit("hub_published_store:descriptor_ref_error_propagation_missing")
if "descriptor.descriptor_ref().is_none()" in production_store:
    raise SystemExit("hub_published_store:descriptor_ref_optional_collapse")
if "pub fn seed_from_snapshot" not in production_store or "-> Result<(), String>" not in production_store:
    raise SystemExit("hub_published_store:seed_not_fallible")
if "pub fn apply_diff" not in production_store or "-> Result<(), String>" not in production_store:
    raise SystemExit("hub_published_store:diff_not_fallible")
for test_name in (
    "seed_rejects_noncanonical_descriptor_rows",
    "apply_diff_is_atomic_when_added_row_is_noncanonical",
):
    if test_name not in store:
        raise SystemExit(f"hub_published_store:missing_test:{test_name}")

realm_split = meta.split("if scope.include_realm", 1)
if len(realm_split) != 2:
    raise SystemExit("meta_list_abilities:realm_merge_missing")
realm_body = realm_split[1].split("scope.apply", 1)[0]
if "entry.descriptor" in realm_body:
    raise SystemExit("meta_list_abilities:realm_opaque_descriptor_passthrough")
if "serde_json::to_value(descriptor)" not in realm_body:
    raise SystemExit("meta_list_abilities:realm_canonical_descriptor_projection_missing")
if 'Value::String("hub:broadcast".to_string())' not in realm_body:
    raise SystemExit("meta_list_abilities:realm_source_projection_missing")

dedupe = re.search(
    r"fn dedupe_descriptor_catalog_entries\([^)]*\)\s*->\s*std::result::Result<Vec<serde_json::Value>, String>\s*\{(?P<body>.*?)\n\}\n\n#\[cfg\(feature = \"axon-pb\"\)\]\nfn descriptor_catalog_dedupe_required_string",
    ffi,
    re.S,
)
if dedupe is None:
    raise SystemExit("ffi_descriptor_catalog:dedupe_not_fallible")
dedupe_body = dedupe.group("body")
if re.search(r"\bcontinue\s*;", dedupe_body):
    raise SystemExit("ffi_descriptor_catalog:dedupe_silent_drop")
if "descriptor_catalog_dedupe_required_string" not in dedupe_body:
    raise SystemExit("ffi_descriptor_catalog:dedupe_required_fields_missing")
if "descriptor_catalog_dedupe_rejects_schema_incomplete_rows" not in ffi:
    raise SystemExit("ffi_descriptor_catalog:missing_schema_incomplete_dedupe_test")

if "fn schema_bound_catalogue_entry" not in cli_catalog_production:
    raise SystemExit("cli_ability_catalog:schema_bound_entry_missing")
for field in ("ability_ura", "owner_ura", "name", "version"):
    if f'required_catalogue_string(object, index, "{field}")' not in cli_catalog_production:
        raise SystemExit(f"cli_ability_catalog:required_field_missing:{field}")
for token, code in (
    ("AbilitySelector::parse(ability_ura)", "ability_selector_missing"),
    ("owner_ura != selector.owner_ura()", "owner_binding_missing"),
    ("name != selector.public_name()", "public_name_binding_missing"),
    ("ability_ura_from_descriptor_ref(descriptor_ref)", "descriptor_ref_ability_binding_missing"),
    ('descriptor_ref.starts_with(&format!("{ability_ura}@{version}#"))', "descriptor_ref_version_binding_missing"),
):
    if token not in cli_catalog_production:
        raise SystemExit(f"cli_ability_catalog:{code}")
if "abilities_from_value_rejects_name_derived_owner_repair" not in cli_catalog:
    raise SystemExit("cli_ability_catalog:owner_repair_regression_test_missing")

for forbidden, code in (
    ('get("ability_version")', "retired_ability_version_field"),
    ('get("input_schema")', "retired_input_schema_field"),
    ("name.split_once", "name_derived_owner_fallback"),
    ("unwrap_or(&args.ability_ura)", "ability_ura_as_name_fallback"),
    ("best-effort: `meta.list_abilities`", "best_effort_catalogue_show"),
):
    if forbidden in cli_ability_production:
        raise SystemExit(f"cli_ability_show:{code}")
for token, code in (
    ('get("version")', "canonical_version_missing"),
    ('get("schema_summary").and_then(|s| s.get("input"))', "canonical_schema_summary_missing"),
    ('expect("schema-bound catalogue row carries owner_ura")', "schema_bound_owner_projection_missing"),
):
    if token not in cli_ability_production:
        raise SystemExit(f"cli_ability_show:{code}")
PY
}

check_daemon_runtime_assembly_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local runtime_binding="$cli_root/src/daemon/invocation/dispatch/deps.rs"
  local invocation_service="$cli_root/src/daemon/invocation/dispatch/daemon_invocation_service.rs"
  local ability_catalog="$cli_root/src/daemon/ability/catalog/build.rs"
  local ability_catalog_tests="$cli_root/src/daemon/ability/catalog/assembly_tests.rs"

  if rg -n 'CanonicalOnly|pub fn with_local_runtime\s*\(' \
    "$runtime_binding" "$invocation_service"; then
    fail "daemon Invocation transport retains a bare LocalRuntime construction path"
  fi

  if [[ ! -f "$ability_catalog" || ! -f "$ability_catalog_tests" ]]; then
    fail "daemon runtime assembly contract sources are missing"
  fi

  "$PYTHON_BIN" - "$ability_catalog" "$ability_catalog_tests" <<'PY'
import re
import sys
from pathlib import Path

catalog = Path(sys.argv[1]).read_text(encoding="utf-8")
tests = Path(sys.argv[2]).read_text(encoding="utf-8")

if "fn replays_hosted_agent_runtime(self) -> bool" not in catalog:
    raise SystemExit("daemon_runtime_assembly:hosted_agent_replay_mode_missing")

if not re.search(
    r"let\s+replay_hosted_agent_runtime\s*=\s*"
    r"hosts_device_authority\s*&&\s*assembly_mode\.replays_hosted_agent_runtime\(\)\s*;",
    catalog,
    re.S,
):
    raise SystemExit("daemon_runtime_assembly:hosted_agent_replay_guard_missing")

if not re.search(r"if\s+replay_hosted_agent_runtime\s*\{\s*if\s+let\s+Some\(hot_registrar\)", catalog, re.S):
    raise SystemExit("daemon_runtime_assembly:hosted_agent_replay_not_bound_to_runtime_mode")

if re.search(r"if\s+hosts_device_authority\s*\{\s*if\s+let\s+Some\(hot_registrar\)", catalog, re.S):
    raise SystemExit("daemon_runtime_assembly:hosted_agent_replay_bound_to_device_authority_only")

if "fn deterministic_registry_snapshot_does_not_replay_hosted_agent_runtime" not in tests:
    raise SystemExit("daemon_runtime_assembly:deterministic_snapshot_replay_regression_test_missing")
PY
}

check_catalog_exact_runtime_key_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local dispatch="$cli_root/src/daemon/ability/dispatch.rs"
  local control_plane="$cli_root/src/daemon/ability/control_plane.rs"
  [[ -f "$dispatch" ]] || fail "ability dispatch source is missing"
  [[ -f "$control_plane" ]] || fail "ability control-plane source is missing"

  "$PYTHON_BIN" - "$dispatch" "$control_plane" <<'PY'
import re
import sys
from pathlib import Path

dispatch = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
control_plane = Path(sys.argv[2]).read_text(encoding="utf-8", errors="replace")

if "fn control_plane_authority_root" in dispatch:
    raise SystemExit("catalog_runtime_key:ability_level_authority_root_helper_present")
if "fn handlers_for_ability" in dispatch:
    raise SystemExit("catalog_runtime_key:ability_level_handler_merge_present")
if "fn fill_missing_from" in dispatch:
    raise SystemExit("catalog_runtime_key:missing_slot_handler_fallback_present")
if "fn list_dynamic_abilities" in dispatch:
    raise SystemExit("catalog_runtime_key:dynamic_list_read_model_present")
if "union dynamic with static" in dispatch:
    raise SystemExit("catalog_runtime_key:dynamic_static_union_wording_present")
if "fall-through paths" in dispatch:
    raise SystemExit("catalog_runtime_key:fallthrough_handler_projection_wording_present")
if "authority_roots_for_ability" in control_plane:
    raise SystemExit("catalog_runtime_key:ability_level_authority_root_query_present")

for token, code in (
    ("fn unique_handler_slot", "unique_handler_slot_projection_missing"),
    ("fn unique_mode_registered", "unique_mode_projection_missing"),
    ("fn runtime_handlers_for_key", "authority_keyed_runtime_handler_read_missing"),
):
    if token not in dispatch:
        raise SystemExit(f"catalog_runtime_key:{code}")

for token, code in (
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc)",
        "resolve_rpc_unique_projection_missing",
    ),
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream)",
        "resolve_stream_unique_projection_missing",
    ),
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream_with_env)",
        "resolve_stream_with_env_unique_projection_missing",
    ),
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi)",
        "resolve_bidi_unique_projection_missing",
    ),
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi_with_env)",
        "resolve_bidi_with_env_unique_projection_missing",
    ),
    (
        "self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc_with_env)",
        "resolve_rpc_with_env_unique_projection_missing",
    ),
):
    if token not in dispatch:
        raise SystemExit(f"catalog_runtime_key:{code}")

if "dynamic execution row remains present after adding a second mode" not in dispatch:
    raise SystemExit("catalog_runtime_key:dynamic_mode_preservation_diagnostic_test_missing")

sync_match = re.search(
    r"fn\s+sync_runtime_ability\s*\([^)]*\)\s*->\s*anyhow::Result<\(\)>\s*\{(?P<body>.*?)\n    \}",
    dispatch,
    re.S,
)
if not sync_match:
    raise SystemExit("catalog_runtime_key:sync_runtime_ability_missing")
sync_body = sync_match.group("body")
for token, code in (
    ("handler_control_plane_key", "sync_exact_key_lookup_missing"),
    ("runtime_handlers_for_key", "sync_authority_keyed_handler_read_missing"),
    ("sync_runtime_ability_from_handlers", "sync_shared_runtime_writer_missing"),
):
    if token not in sync_body:
        raise SystemExit(f"catalog_runtime_key:{code}")

match = re.search(
    r"fn\s+verify_execution_key_control_plane_modes\s*\([^)]*\)\s*->\s*anyhow::Result<\(\)>\s*\{(?P<body>.*?)\n    \}",
    dispatch,
    re.S,
)
if not match:
    raise SystemExit("catalog_runtime_key:exact_verifier_missing")
body = match.group("body")
for token, code in (
    ("control_plane_record_for_authority_mode", "exact_authority_mode_lookup_missing"),
    ("key.authority_root()", "execution_authority_binding_missing"),
    ("key.ability()", "execution_ability_binding_missing"),
    ("slot.call_mode()", "handler_mode_binding_missing"),
    ("no exact control-plane record", "fail_closed_error_missing"),
):
    if token not in body:
        raise SystemExit(f"catalog_runtime_key:{code}")

for method in ("static_control_plane_key", "dynamic_control_plane_key"):
    method_match = re.search(
        rf"fn\s+{method}\s*\([^)]*\)\s*->\s*anyhow::Result<Option<ControlPlaneAbilityKey>>\s*\{{(?P<body>.*?)\n    \}}",
        dispatch,
        re.S,
    )
    if not method_match:
        raise SystemExit(f"catalog_runtime_key:{method}_missing")
    method_body = method_match.group("body")
    for token, code in (
        ("origin_key_by_ability", "execution_key_lookup_missing"),
        ("handlers_for_key", "authority_keyed_handler_read_missing"),
        ("verify_execution_key_control_plane_modes", "exact_verifier_not_used"),
    ):
        if token not in method_body:
            raise SystemExit(f"catalog_runtime_key:{method}:{code}")

for token, code in (
    (
        "static_runtime_key_validates_exact_authority_mode_record",
        "static_positive_test_missing",
    ),
    (
        "static_runtime_key_rejects_unrelated_authority_record_as_rescue_path",
        "static_negative_test_missing",
    ),
    (
        "dynamic_runtime_key_validates_exact_authority_mode_record",
        "dynamic_positive_test_missing",
    ),
    (
        "ability_name_handler_projection_rejects_multi_authority_same_slot",
        "same_slot_ambiguity_test_missing",
    ),
    (
        "ability_name_handler_projection_does_not_synthesize_cross_authority_runtime_set",
        "cross_authority_runtime_set_test_missing",
    ),
):
    if token not in dispatch:
        raise SystemExit(f"catalog_runtime_key:{code}")
PY
}

check_plugin_sidecar_helper_matrix_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local template="$cli_root/src/cli/commands/groups/plugin_template.rs"
  if [[ ! -f "$template" ]]; then
    fail "plugin sidecar helper matrix source is missing: $template"
  fi

  "$PYTHON_BIN" - "$template" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
cli_root = path.parents[4]

required_states = {
    "Unsupported",
    "Seam",
    "ProviderBacked",
    "CutoverReady",
}
state_enum = re.search(
    r"pub enum ProviderSidecarHelperState\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not state_enum:
    raise SystemExit("plugin_sidecar_helper_state_enum_missing")
state_variants = set(re.findall(r"\b([A-Z][A-Za-z0-9_]*)\s*,", state_enum.group("body")))
if not required_states.issubset(state_variants):
    raise SystemExit(
        "plugin_sidecar_helper_state_enum_incomplete:"
        + ",".join(sorted(required_states - state_variants))
    )

required_call_modes = {
    "ExecInvoke",
    "ExecStream",
    "ExecBidi",
}
call_mode_enum = re.search(
    r"pub enum ProviderSidecarCallMode\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not call_mode_enum:
    raise SystemExit("plugin_sidecar_call_mode_enum_missing")
call_mode_variants = set(
    re.findall(r"\b([A-Z][A-Za-z0-9_]*)\s*,", call_mode_enum.group("body"))
)
if not required_call_modes.issubset(call_mode_variants):
    raise SystemExit(
        "plugin_sidecar_call_mode_enum_incomplete:"
        + ",".join(sorted(required_call_modes - call_mode_variants))
    )

language_enum = re.search(
    r"pub enum PluginTemplateLanguage\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not language_enum:
    raise SystemExit("plugin_template_language_enum_missing")
template_variants = set(
    re.findall(r"^\s+([A-Z][A-Za-z0-9_]*)\s*,", language_enum.group("body"), re.M)
)
if template_variants != {"Python", "Go", "Rust", "Java", "Node"}:
    raise SystemExit(
        "plugin_template_language_surface_not_helper_backed:"
        + ",".join(sorted(template_variants))
    )

matrix = re.search(
    r"PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX:\s*&\[ProviderSidecarHelperCapability\]\s*=\s*&\[(?P<body>.*?)\n\];",
    text,
    re.S,
)
if not matrix:
    raise SystemExit("plugin_sidecar_helper_matrix_missing")

rows = {}
for match in re.finditer(
    r"ProviderSidecarHelperCapability\s*\{(?P<body>.*?)\n\s*\},",
    matrix.group("body"),
    re.S,
):
    body = match.group("body")
    language = re.search(r'language:\s*"([^"]+)"', body)
    call_mode = re.search(r"call_mode:\s*ProviderSidecarCallMode::([A-Za-z0-9_]+)", body)
    state = re.search(r"state:\s*ProviderSidecarHelperState::([A-Za-z0-9_]+)", body)
    template_available = re.search(r"template_available:\s*(true|false)", body)
    helper = re.search(r'helper_package:\s*(Some\("([^"]+)"\)|None)', body)
    if not (language and call_mode and state and template_available and helper):
        raise SystemExit("plugin_sidecar_helper_matrix_row_malformed")
    key = (language.group(1), call_mode.group(1))
    if key in rows:
        raise SystemExit("plugin_sidecar_helper_matrix_duplicate:" + "/".join(key))
    rows[key] = {
        "state": state.group(1),
        "template_available": template_available.group(1) == "true",
        "helper_package": helper.group(2),
    }

required_languages = {"python", "go", "rust", "node", "java", "c/c++"}
matrix_languages = {language for language, _call_mode in rows}
if not required_languages.issubset(matrix_languages):
    raise SystemExit(
        "plugin_sidecar_helper_matrix_incomplete:"
        + ",".join(sorted(required_languages - matrix_languages))
    )
for language in required_languages:
    for call_mode in required_call_modes:
        if (language, call_mode) not in rows:
            raise SystemExit(f"plugin_sidecar_helper_matrix_missing_cell:{language}:{call_mode}")

expected_helpers = {
    "python": "easynet_sdk.providers.easynet.plugin_exec",
    "go": "easynet.run/cli/sdk/go/provider/easynet/pluginexec",
    "rust": "easynet-provider-pluginexec",
    "java": "run.runtime.sdk.provider.easynet.pluginexec",
    "node": "@easynet/daemon-sdk/provider/easynet/pluginexec",
}
expected_helper_files = {
    "python": [
        "sdk/python/easynet_sdk/providers/easynet/plugin_exec.py",
        "sdk/python/tests/test_plugin_exec.py",
    ],
    "go": [
        "sdk/go/provider/easynet/pluginexec/pluginexec.go",
        "sdk/go/provider/easynet/pluginexec/pluginexec_test.go",
    ],
    "rust": [
        "sdk/rust/provider/easynet/pluginexec/Cargo.toml",
        "sdk/rust/provider/easynet/pluginexec/src/lib.rs",
        "sdk/rust/provider/easynet/pluginexec/tests/pluginexec.rs",
    ],
    "java": [
        "sdk/java/src/main/java/run/runtime/sdk/provider/easynet/pluginexec/SidecarRuntime.java",
        "sdk/java/src/main/java/run/runtime/sdk/provider/easynet/pluginexec/SidecarInvocation.java",
        "sdk/java/src/test/java/run/runtime/sdk/provider/easynet/pluginexec/SidecarRuntimeTest.java",
    ],
    "node": [
        "sdk/node/provider/easynet/pluginexec.js",
        "sdk/node/provider/easynet/pluginexec.d.ts",
        "sdk/node/test/pluginexec.test.mjs",
    ],
}
for language, helper in expected_helpers.items():
    row = rows[(language, "ExecInvoke")]
    if row["state"] not in {"ProviderBacked", "CutoverReady"}:
        raise SystemExit(f"plugin_template_helper_not_provider_backed:{language}")
    if not row["template_available"]:
        raise SystemExit(f"plugin_template_helper_not_exposed:{language}")
    if row["helper_package"] != helper:
        raise SystemExit(f"plugin_template_helper_package_mismatch:{language}")
    for rel_path in expected_helper_files[language]:
        if not (cli_root / rel_path).is_file():
            raise SystemExit(f"plugin_template_helper_source_missing:{language}:{rel_path}")

for language in sorted(required_languages - set(expected_helpers)):
    row = rows[(language, "ExecInvoke")]
    if row["state"] not in {"Unsupported", "Seam"}:
        raise SystemExit(f"plugin_unbacked_language_state_open:{language}:{row['state']}")
    if row["template_available"]:
        raise SystemExit(f"plugin_unbacked_language_template_open:{language}")
    if row["helper_package"] is not None:
        raise SystemExit(f"plugin_unbacked_language_helper_claim:{language}")

for language in sorted(required_languages):
    for call_mode in ("ExecStream", "ExecBidi"):
        row = rows[(language, call_mode)]
        if row["state"] not in {"Unsupported", "Seam"}:
            raise SystemExit(
                f"plugin_streaming_helper_state_open_without_contract:{language}:{call_mode}:{row['state']}"
            )
        if row["template_available"]:
            raise SystemExit(f"plugin_streaming_template_open_without_helper:{language}:{call_mode}")
        if row["helper_package"] is not None:
            raise SystemExit(f"plugin_streaming_helper_claim_without_contract:{language}:{call_mode}")

variant_labels = {
    "Python": "python",
    "Go": "go",
    "Rust": "rust",
    "Java": "java",
    "Node": "node",
}
if {variant_labels[variant] for variant in template_variants} != {
    language for (language, call_mode), row in rows.items()
    if call_mode == "ExecInvoke" and row["template_available"]
}:
    raise SystemExit("plugin_template_enum_and_matrix_drift")

for const_name in ("PYTHON_EXEC_PLUGIN", "GO_EXEC_PLUGIN", "RUST_EXEC_PLUGIN", "JAVA_EXEC_PLUGIN", "NODE_EXEC_PLUGIN"):
    template = re.search(
        rf'const {const_name}: &str = r#"(.*?)"#;',
        text,
        re.S,
    )
    if not template:
        raise SystemExit(f"plugin_template_constant_missing:{const_name}")
    body = template.group(1)
    forbidden = [
        "json.loads",
        "JSON.parse",
        "json.NewDecoder",
        "NewDecoder(",
        "encoding/json",
        "serde_json::from_str",
        "serde_json::Deserializer",
        "JsonFrameCodec",
        "ObjectMapper",
        "Gson",
    ]
    leaked = [pattern for pattern in forbidden if pattern in body]
    if leaked:
        raise SystemExit(
            f"plugin_template_naked_sidecar_frame:{const_name}:{','.join(leaked)}"
        )

if "serve_exec_plugin(handle)" not in text:
    raise SystemExit("plugin_python_template_missing_provider_helper")
if "pluginexec.MustServe" not in text:
    raise SystemExit("plugin_go_template_missing_provider_helper")
if "serve_exec_plugin" not in text or "easynet_provider_pluginexec" not in text:
    raise SystemExit("plugin_rust_template_missing_provider_helper")
if "SidecarRuntime.serve" not in text or "run.runtime.sdk.provider.easynet.pluginexec" not in text:
    raise SystemExit("plugin_java_template_missing_provider_helper")
if "serveExecPlugin" not in text:
    raise SystemExit("plugin_node_template_missing_provider_helper")
PY
}

check_retired_browser_mock_surface_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local descriptor_dir="$cli_root/ability-descriptors/system/device_control"
  local active_paths=(
    "$cli_root/src/daemon/ability/builtins/device_control/mod.rs"
    "$cli_root/src/daemon/ability/catalog/build.rs"
    "$cli_root/src/daemon/ability/catalog/catalog_metadata.rs"
    "$cli_root/src/daemon/ability/catalog/descriptor_paths.rs"
    "$cli_root/src/daemon/ability/conformance.rs"
    "$cli_root/src/daemon/ability/names/device_control.rs"
    "$cli_root/src/daemon/ability/wire/mod.rs"
    "$cli_root/src/daemon/invocation/dispatch/local_session_dispatcher.rs"
    "$cli_root/sdk/go/live_smoke_cabi_test.go"
    "$cli_root/tools/scripts/python-sdk-live-smoke.sh"
    "$cli_root/tools/scripts/ffi-smoke.sh"
    "$cli_root/tests/script_checks.rs"
  )

  if [[ -e "$cli_root/src/daemon/ability/builtins/device_control/browser.rs" ]]; then
    fail "retired browser placeholder ability implementation is still present"
  fi
  if [[ -e "$cli_root/tools/scripts/check-browser-session-service-boundary.sh" ]]; then
    fail "retired browser placeholder boundary gate is still present"
  fi
  if [[ -e "$cli_root/tests/scripts/test_check_browser_session_service_boundary.sh" ]]; then
    fail "retired browser placeholder boundary self-test is still present"
  fi
  if [[ -d "$descriptor_dir" ]] && find "$descriptor_dir" -name 'browser.*.ability.toml' -print -quit | grep -q .; then
    fail "retired browser placeholder descriptors are still present"
  fi
  if [[ -d "$descriptor_dir" ]] && rg -n \
    'browser\.(open_session|send_input|capture_viewport|close_session|attach_session)|V0 MOCK|PLACEHOLDER|DeviceBrowser|BrowserSessionService' \
    "$descriptor_dir"; then
    fail "retired browser placeholder vocabulary leaked into active descriptor inventory"
  fi
  for path in "${active_paths[@]}"; do
    [[ -f "$path" ]] || continue
    if rg -n \
      'browser_session_ability|DeviceBrowser|BrowserSessionService|PLACEHOLDER_WEBP|V0 MOCK|is_placeholder|check-browser-session-service-boundary|browser_session_service_boundary|device_control::browser|BROWSER_(OPEN_SESSION|SEND_INPUT|CAPTURE_VIEWPORT|CLOSE_SESSION|ATTACH_SESSION)|browser\.(open_session|send_input|capture_viewport|close_session|attach_session)' \
      "$path"; then
      fail "retired browser placeholder ability leaked into active runtime surface: ${path#$cli_root/}"
    fi
  done
}

check_ability_deploy_product_neutrality_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local ability_deploy="$cli_root/src/daemon/ability/builtins/device_control/ability_management/ops.rs"
  [[ -f "$ability_deploy" ]] || fail "ability.deploy runtime source is missing: ${ability_deploy#$cli_root/}"

  if rg -n '\bEasyRemote\b|\bEasyNet Backend\b|\bEasyNet-specific\b|\bEasyRemote-specific\b' "$ability_deploy"; then
    fail "ability.deploy runtime path preserves product-specific vocabulary"
  fi
}

check_ability_manifest_exec_absence_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local manifest="$cli_root/src/daemon/ability/manifest.rs"
  local authoring="$cli_root/src/daemon/ability/builtins/agents/authoring.rs"
  local chat="$cli_root/src/daemon/ability/builtins/agents/chat.rs"
  local teach="$cli_root/src/daemon/ability/builtins/governance/teach.rs"

  for path in "$manifest" "$authoring" "$chat" "$teach"; do
    [[ -f "$path" ]] || fail "ability manifest exec-absence contract source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$manifest" "$authoring" "$chat" "$teach" <<'PY'
import sys
from pathlib import Path

manifest, authoring, chat, teach = [
    Path(arg).read_text(encoding="utf-8", errors="replace") for arg in sys.argv[1:]
]

for retired in (
    "owning agent's chat handler\" (legacy default)",
    "legacy default",
    "runtime fallback",
):
    if retired in manifest:
        raise SystemExit(f"ability_manifest_exec_absence:retired_manifest_model:{retired}")

required = (
    (manifest, "discovery-only metadata and has no executable runtime binding", "manifest_doc_missing"),
    (authoring, "no executable binding and cannot enter the live capability catalog", "authoring_reject_missing"),
    (chat, "manifest without [exec] must not be routed through an LLM-mediated handler", "chat_reject_missing"),
    (teach, "manifest without [exec] must remain discovery-only, not a runtime binding", "teach_runtime_binding_reject_missing"),
)
for text, token, code in required:
    if token not in text:
        raise SystemExit(f"ability_manifest_exec_absence:{code}")
PY
}

check_sdk_directory_projection_fail_closed_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local go_directory="$cli_root/sdk/go/directory.go"
  local py_directory="$cli_root/sdk/python/easynet_sdk/directory.py"
  local go_test="$cli_root/sdk/go/directory_test.go"
  local py_test="$cli_root/sdk/python/tests/test_directory.py"

  for path in "$go_directory" "$py_directory" "$go_test" "$py_test"; do
    [[ -f "$path" ]] || fail "SDK Directory projection contract source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$go_directory" "$py_directory" "$go_test" "$py_test" <<'PY'
import re
import sys
from pathlib import Path

go_directory = Path(sys.argv[1]).read_text(encoding="utf-8")
py_directory = Path(sys.argv[2]).read_text(encoding="utf-8")
go_test = Path(sys.argv[3]).read_text(encoding="utf-8")
py_test = Path(sys.argv[4]).read_text(encoding="utf-8")

project = re.search(
    r"func ProjectDirectoryResolution\(.*?\n\}",
    go_directory,
    re.S,
)
if not project:
    raise SystemExit("go_directory_projection_missing")
project_body = project.group(0)
for token, code in (
    ('invalidDirectory("Directory answer must be an object"', "answer_type_gate_missing"),
    ('invalidDirectory("Directory records must be a list"', "records_type_gate_missing"),
    ("optionalDirectoryMap(output, \"negative\")", "negative_gate_missing"),
    ("optionalDirectoryMap(output, \"next_hop\")", "next_hop_gate_missing"),
    ("optionalDirectoryMap(output, \"selected_route\")", "selected_route_gate_missing"),
    ("optionalDirectoryMapSlice(output, \"route_candidates\")", "route_candidates_gate_missing"),
    ("optionalDirectoryMap(output, \"authority\")", "authority_gate_missing"),
    ("optionalDirectoryMap(output, \"cache_policy\")", "cache_policy_gate_missing"),
):
    if token not in project_body:
        raise SystemExit("go_directory_projection:" + code)
for retired in (
    'answerKind == "" && len(negative) > 0',
    'answerKind = "RESOLVE_ANSWER_KIND_NEGATIVE"',
    'directoryString(raw, "kind", "type")',
    'directoryString(raw, "ura", "canonical_name")',
    'func directoryString(value map[string]any, keys ...string)',
):
    if retired in go_directory:
        raise SystemExit("go_directory_projection_answer_kind_fallback")
if 'func directoryText(value map[string]any, key string)' not in go_directory:
    raise SystemExit("go_directory_single_field_text_projector_missing")

map_slice = re.search(
    r"func optionalDirectoryMapSlice\(.*?\n\}",
    go_directory,
    re.S,
)
if not map_slice:
    raise SystemExit("go_directory_map_slice_gate_missing")
map_slice_body = map_slice.group(0)
if "continue" in map_slice_body:
    raise SystemExit("go_directory_route_candidate_item_skip")
for token, code in (
    ("must be a list", "list_type_error_missing"),
    ("item must be an object", "item_type_error_missing"),
):
    if token not in map_slice_body:
        raise SystemExit("go_directory_map_slice:" + code)

optional_map = re.search(
    r"func optionalDirectoryMap\(.*?\n\}",
    go_directory,
    re.S,
)
if not optional_map or "must be an object" not in optional_map.group(0):
    raise SystemExit("go_directory_optional_map_gate_missing")

py_optional = re.search(
    r"def _optional_mapping\(.*?\n\n",
    py_directory,
    re.S,
)
if not py_optional:
    raise SystemExit("python_directory_optional_mapping_missing")
py_optional_body = py_optional.group(0)
if "json.loads" in py_optional_body or "_required_mapping" in py_optional_body:
    raise SystemExit("python_directory_optional_mapping_decodes_nested_json")
if "must be an object" not in py_optional_body:
    raise SystemExit("python_directory_optional_mapping_type_gate_missing")
project_resolution = re.search(
    r"def _project_resolution\(.*?\n\n",
    py_directory,
    re.S,
)
if not project_resolution:
    raise SystemExit("python_directory_projection_missing")
if "Directory answer must be an object" not in project_resolution.group(0):
    raise SystemExit("python_directory_answer_type_gate_missing")
for retired in (
    'if not answer_kind and _optional_mapping(output.get("negative"), "negative")',
    'answer_kind = "RESOLVE_ANSWER_KIND_NEGATIVE"',
    '_mapping_text(value, "kind", "type")',
    '_mapping_text(value, "ura", "canonical_name")',
    'def _mapping_text(value: Mapping[str, object], *keys: str)',
):
    if retired in py_directory:
        raise SystemExit("python_directory_projection_answer_kind_fallback")
if 'def _mapping_text(value: Mapping[str, object], key: str) -> str:' not in py_directory:
    raise SystemExit("python_directory_single_field_text_projector_missing")

py_sequence = re.search(
    r"def _optional_mapping_sequence\(.*?\n\n",
    py_directory,
    re.S,
)
if not py_sequence:
    raise SystemExit("python_directory_optional_sequence_missing")
py_sequence_body = py_sequence.group(0)
for token, code in (
    ("must be a list", "list_type_error_missing"),
    ("item must be an object", "item_type_error_missing"),
):
    if token not in py_sequence_body:
        raise SystemExit("python_directory_optional_sequence:" + code)

for text, name in ((go_test, "go"), (py_test, "python")):
    if "RejectsMalformedPresentFacts" not in text and "rejects_malformed_present_facts" not in text:
        raise SystemExit(f"{name}_directory_malformed_present_facts_test_missing")
    if "RejectsNegativeWithoutAnswerKind" not in text and "rejects_negative_without_answer_kind" not in text:
        raise SystemExit(f"{name}_directory_negative_without_answer_kind_test_missing")
    if "DoesNotPromoteLegacyAliases" not in text and "does_not_promote_legacy_aliases" not in text:
        raise SystemExit(f"{name}_directory_legacy_alias_projection_test_missing")
    for field in (
        "answer",
        "records",
        "next_hop",
        "selected_route",
        "route_candidates",
        "negative",
        "authority",
        "cache_policy",
    ):
        if field not in text:
            raise SystemExit(f"{name}_directory_malformed_field_test_missing:{field}")
PY
}

check_sdk_runtime_recovery_report_fail_closed_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local go_runtime="$cli_root/sdk/go/runtime.go"
  local go_test="$cli_root/sdk/go/runtime_test.go"
  local py_runtime="$cli_root/sdk/python/easynet_sdk/runtime.py"
  local py_test="$cli_root/sdk/python/tests/test_runtime.py"

  for path in "$go_runtime" "$go_test" "$py_runtime" "$py_test"; do
    [[ -f "$path" ]] || fail "SDK Runtime recovery contract source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$go_runtime" "$go_test" "$py_runtime" "$py_test" <<'PY'
import re
import sys
from pathlib import Path

go_runtime = Path(sys.argv[1]).read_text(encoding="utf-8")
go_test = Path(sys.argv[2]).read_text(encoding="utf-8")
py_runtime = Path(sys.argv[3]).read_text(encoding="utf-8")
py_test = Path(sys.argv[4]).read_text(encoding="utf-8")

if "type runtimeRecoveryEventDTO struct" not in go_runtime:
    raise SystemExit("go_runtime_recovery_event_private_wire_dto_missing")
if "Terminal     *bool  `json:\"terminal\"`" not in go_runtime:
    raise SystemExit("go_runtime_recovery_event_terminal_pointer_missing")
if "recovery event terminal is required" not in go_runtime:
    raise SystemExit("go_runtime_recovery_event_terminal_required_gate_missing")
for field in (
    "RecoveredInvocations     *int",
    "ReapedOrphans            *int",
    "ReplayedTerminalReceipts *int",
):
    if field not in go_runtime:
        raise SystemExit("go_runtime_recovery_counter_pointer_missing")
for token in (
    "requiredRuntimeRecoveryCounter(",
    'field+" is required"',
    'field+" must be non-negative"',
):
    if token not in go_runtime:
        raise SystemExit("go_runtime_recovery_counter_required_gate_missing")
decoder = re.search(
    r"func NewRuntimeRecoveryReportFromJSON\(.*?\n\}",
    go_runtime,
    re.S,
)
if not decoder:
    raise SystemExit("go_runtime_recovery_decoder_missing")
if re.search(r"Events\s+\[\]RuntimeRecoveryEvent\s+`json:\"events\"`", decoder.group(0)):
    raise SystemExit("go_runtime_recovery_event_public_dto_decode_fallback")
for retired in (
    "RecoveredInvocations     int",
    "ReapedOrphans            int",
    "ReplayedTerminalReceipts int",
):
    if retired in decoder.group(0):
        raise SystemExit("go_runtime_recovery_counter_zero_default_fallback")
if "_required_bool(value, \"terminal\")" not in py_runtime:
    raise SystemExit("python_runtime_recovery_event_terminal_required_gate_missing")
if "_required_non_negative_int(" not in py_runtime:
    raise SystemExit("python_runtime_required_counter_helper_missing")
for retired in (
    '_optional_non_negative_int(\n                decoded.get("recovered_invocations")',
    '_optional_non_negative_int(\n                decoded.get("reaped_orphans")',
    '_optional_non_negative_int(\n                decoded.get("replayed_terminal_receipts")',
):
    if retired in py_runtime:
        raise SystemExit("python_runtime_recovery_counter_zero_default_fallback")
for text, name in ((go_test, "go"), (py_test, "python")):
    if (
        "missing recovery event terminal" not in text
        and '"events":[{"sequence":1,"kind":"orphan_reaped"}]' not in text
    ):
        raise SystemExit(f"{name}_runtime_recovery_event_terminal_test_missing")
    if "missing recovery counter" not in text and "missing_counter_caught" not in text:
        raise SystemExit(f"{name}_runtime_recovery_counter_missing_test_missing")
    if "negative recovery counter" not in text and "negative_counter_caught" not in text:
        raise SystemExit(f"{name}_runtime_recovery_counter_negative_test_missing")
PY
}

check_federation_directory_device_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local directory="$cli_root/src/daemon/federation/directory.rs"
  local wrappers="$cli_root/src/daemon/invocation/dispatch/federation_wrappers.rs"
  local stream_dispatcher="$cli_root/src/daemon/invocation/streams/stream_dispatcher.rs"

  [[ -f "$directory" ]] || fail "federation directory source is missing"
  [[ -f "$wrappers" ]] || fail "federation wrappers source is missing"
  [[ -f "$stream_dispatcher" ]] || fail "stream dispatcher source is missing"

  "$PYTHON_BIN" - "$directory" "$wrappers" "$stream_dispatcher" <<'PY'
import re
import sys
from pathlib import Path

directory = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
wrappers = Path(sys.argv[2]).read_text(encoding="utf-8", errors="replace")
stream_dispatcher = Path(sys.argv[3]).read_text(encoding="utf-8", errors="replace")

for token, code in (
    ("fn agent_ura_to_node_id", "raw_ura_node_id_fallback_helper_present"),
    ("unwrap_or_else(|| agent_ura.to_string())", "raw_ura_node_id_fallback_present"),
    ("node_id = agent_ura.clone()", "raw_ura_node_id_wording_present"),
    ("presence_ura_to_directory_entry_falls_back_when_ura_non_canonical", "fallback_test_present"),
    ("presence_ura_to_directory_entry_treats_legacy_agent_shape_as_non_canonical", "legacy_agent_projection_test_present"),
):
    if token in directory:
        raise SystemExit(f"federation_directory_device_projection:{code}")

for token, code in (
    ("fn canonical_device_node_id", "canonical_device_validator_missing"),
    ("parsed.kind != crate::core::ura::URAKind::Device", "device_kind_gate_missing"),
    ("crate::core::ura::device_ura(&parsed.realm, node_id)", "canonical_device_ura_rebuild_missing"),
    ("pub fn presence_uras_to_directory_snapshot", "snapshot_adapter_missing"),
    (") -> Result<DirectoryEvent, String>", "snapshot_adapter_must_be_fallible"),
    ("pub fn presence_event_to_directory_event_at", "event_adapter_missing"),
    ("apply_snapshot_rejects_invalid_agent_ura_without_mutating_view", "atomic_snapshot_rejection_test_missing"),
    ("presence_event_rejects_non_device_ura", "event_rejection_test_missing"),
):
    if token not in directory:
        raise SystemExit(f"federation_directory_device_projection:{code}")

apply_frame = re.search(
    r"pub fn apply_frame\s*\([^)]*\)\s*->\s*Result<\(\), String>\s*\{(?P<body>.*?)\n    \}",
    directory,
    re.S,
)
if not apply_frame:
    raise SystemExit("federation_directory_device_projection:apply_frame_fallible_missing")
body = apply_frame.group("body")
for token, code in (
    ("let mut next_entries = BTreeMap::new();", "snapshot_staging_map_missing"),
    ("self.entries = next_entries;", "snapshot_atomic_commit_missing"),
    ("directory_agent_summary_to_entry(raw, &self.peer_realm)?", "snapshot_entry_validation_missing"),
    ("canonical_device_node_id(agent_ura, \"directory revoke event\")?", "revoke_validation_missing"),
):
    if token not in body:
        raise SystemExit(f"federation_directory_device_projection:{code}")

if "build_subscribe_directory_v2_snapshot_rejects_non_device_presence_row" not in wrappers:
    raise SystemExit("federation_directory_device_projection:snapshot_builder_rejection_test_missing")
if "build_subscribe_directory_v2_snapshot(&presence)" not in stream_dispatcher:
    raise SystemExit("federation_directory_device_projection:stream_dispatcher_snapshot_builder_missing")
for token, code in (
    ("invalid_presence_event", "stream_invalid_event_observability_missing"),
    ("invalid_presence_snapshot", "stream_invalid_snapshot_observability_missing"),
):
    if token not in stream_dispatcher:
        raise SystemExit(f"federation_directory_device_projection:{code}")
PY
}

check_cli_device_directory_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local devices="$cli_root/src/cli/commands/devices.rs"
  [[ -f "$devices" ]] || return 0

  "$PYTHON_BIN" - "$devices" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
production = text.split("\n#[cfg(test)]", 1)[0]

for retired in (
    'n.get("os")',
    'n.get("arch")',
    '"last_heartbeat_unix_ms"',
):
    if retired in production:
        raise SystemExit(f"cli_device_directory_projection:retired_alias:{retired}")

for required in (
    "renderer_ignores_legacy_top_level_platform_aliases",
    "renderer_ignores_legacy_last_heartbeat_alias",
):
    if required not in text:
        raise SystemExit(f"cli_device_directory_projection:missing_test:{required}")
PY
}

check_runtime_wire_target_state_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local descriptor_binding="$cli_root/src/daemon/invocation/dispatch/descriptor_binding.rs"
  [[ -f "$descriptor_binding" ]] || fail "runtime wire-target descriptor binding source is missing"

  "$PYTHON_BIN" - "$descriptor_binding" <<'PY'
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
for token, code in (
    ("enum WireAbilityTarget", "wire_target_state_enum_missing"),
    ("DescriptorRef {", "wire_target_descriptor_state_missing"),
    ("OwnerLocal {", "wire_target_owner_local_state_missing"),
    (
        "fn is_descriptor_bound_wire_target(",
        "wire_target_descriptor_classification_missing",
    ),
    (
        "WireAbilityTarget::parse(surface, callee_ura, wire_target)?",
        "wire_target_typed_parse_missing",
    ),
    (
        "wire_target.ability_ura() != self.runtime_ability_ura",
        "wire_target_runtime_match_missing",
    ),
    (
        "status_from_dispatch_key_mismatch",
        "wire_target_mismatch_semantics_missing",
    ),
    (
        "wire_target_match_accepts_owner_local_selector_explicitly",
        "wire_target_owner_local_test_missing",
    ),
    (
        "wire_target_match_accepts_descriptor_bound_selector_explicitly",
        "wire_target_descriptor_ref_test_missing",
    ),
    (
        "wire_target_match_rejects_malformed_descriptor_like_target_without_owner_local_reinterpretation",
        "wire_target_malformed_descriptor_test_missing",
    ),
):
    if token not in text:
        raise SystemExit(code)
if "historic forms" in text:
    raise SystemExit("wire_target_historic_fallback_wording_present")
PY
}

check_invocation_wire_callee_target_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local invocation_wire="$cli_root/src/daemon/invocation/dispatch/invocation_wire.rs"
  local unary_dispatcher="$cli_root/src/daemon/invocation/dispatch/unary_dispatcher.rs"
  local stream_dispatcher="$cli_root/src/daemon/invocation/streams/stream_dispatcher.rs"
  local bidi_dispatcher="$cli_root/src/daemon/invocation/bidi/bidi_dispatcher.rs"
  local session_dispatcher="$cli_root/src/daemon/invocation/dispatch/local_session_dispatcher.rs"

  for path in "$invocation_wire" "$unary_dispatcher" "$stream_dispatcher" "$bidi_dispatcher" "$session_dispatcher"; do
    [[ -f "$path" ]] || fail "invocation callee target contract source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$invocation_wire" "$unary_dispatcher" "$stream_dispatcher" "$bidi_dispatcher" "$session_dispatcher" <<'PY'
import re
import sys
from pathlib import Path

invocation_wire = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
callers = [Path(path).read_text(encoding="utf-8", errors="replace") for path in sys.argv[2:]]

for token, code in (
    ("fn target_ura_from_envelope", "retired_target_helper_present"),
    ("caller as fallback", "caller_fallback_wording_present"),
    ("callee or caller URA", "caller_fallback_error_wording_present"),
    (".or(envelope.caller.as_ref())", "caller_fallback_expression_present"),
    ("or(envelope.caller", "caller_fallback_expression_present"),
):
    if token in invocation_wire:
        raise SystemExit(f"invocation_wire_callee_target:{code}")

if "pub(crate) fn callee_ura_from_envelope" not in invocation_wire:
    raise SystemExit("invocation_wire_callee_target:callee_helper_missing")
helper = re.search(
    r"pub\(crate\) fn callee_ura_from_envelope\s*\([^)]*\)\s*->\s*Result<String, tonic::Status>\s*\{(?P<body>.*?)\n\}",
    invocation_wire,
    re.S,
)
if not helper:
    raise SystemExit("invocation_wire_callee_target:callee_helper_signature_missing")
body = helper.group("body")
for present, code in (
    ("callee" in body and ".as_ref()" in body, "callee_read_missing"),
    ("must carry callee URA" in body, "callee_required_error_missing"),
    ("crate::core::ura::parse_ura(callee_ura)" in body, "callee_ura_validation_missing"),
):
    if not present:
        raise SystemExit(f"invocation_wire_callee_target:{code}")
if "envelope.caller" in body:
    raise SystemExit("invocation_wire_callee_target:callee_helper_reads_caller")

for token, code in (
    ("callee_ura_from_envelope_extracts_explicit_callee", "callee_positive_test_missing"),
    ("callee_ura_from_envelope_rejects_caller_only_tuple", "caller_only_rejection_test_missing"),
):
    if token not in invocation_wire:
        raise SystemExit(f"invocation_wire_callee_target:{code}")

for index, text in enumerate(callers, start=1):
    if "target_ura_from_envelope" in text:
        raise SystemExit(f"invocation_wire_callee_target:caller_{index}_uses_retired_helper")
    if "callee_ura_from_envelope" not in text:
        raise SystemExit(f"invocation_wire_callee_target:caller_{index}_not_migrated")
PY
}

check_local_session_descriptor_ref_test_authority_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local session_dispatcher="$cli_root/src/daemon/invocation/dispatch/local_session_dispatcher.rs"

  [[ -f "$session_dispatcher" ]] || fail "local session dispatcher source is missing: ${session_dispatcher#$cli_root/}"

  "$PYTHON_BIN" - "$session_dispatcher" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")

for token, code in (
    ("fn descriptor_ref_for_version", "retired_descriptor_ref_synthesis_helper"),
    ("descriptor_ref_for_version(", "retired_descriptor_ref_synthesis_call"),
    ("unwrap_or_else(|_| descriptor_ref_for_version", "catalog_failure_descriptor_ref_repair"),
):
    if token in text:
        raise SystemExit(f"local_session_descriptor_ref_test_authority:{code}")

helper = re.search(
    r"fn descriptor_ref_for_call_mode\s*\([^)]*\)\s*->\s*String\s*\{(?P<body>.*?)\n    \}",
    text,
    re.S,
)
if not helper:
    raise SystemExit("local_session_descriptor_ref_test_authority:helper_missing")
body = helper.group("body")
required = {
    "canonical_ability_descriptor_ref(ability)": "explicit_descriptor_ref_parse_missing",
    "catalog_descriptor_ref_for_wire(": "catalog_descriptor_ref_authority_missing",
    "expect(\"test ability must resolve through canonical catalog descriptor authority\")": "fail_closed_catalog_error_missing",
}
for fragment, code in required.items():
    if fragment not in body:
        raise SystemExit(f"local_session_descriptor_ref_test_authority:{code}")
PY
}

check_local_daemon_loopback_explicit_subject_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local grpc="$cli_root/src/support/platform/local_daemon_grpc.rs"

  [[ -f "$grpc" ]] || fail "local daemon loopback source is missing: ${grpc#$cli_root/}"

  "$PYTHON_BIN" - "$grpc" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")

for token, code in (
    ("LocalDaemonSelf", "self_subject_policy_present"),
    ("local_daemon_self", "self_subject_constructor_present"),
    ("local_daemon_default_callee_ura", "daemon_identity_named_as_callee_present"),
    ("fn local_root(", "subjectless_local_root_constructor_present"),
    ("LocalDaemonLoopbackTuplePlan::local_root(", "subjectless_local_root_call_present"),
):
    if token in text:
        raise SystemExit(f"local_daemon_loopback_explicit_subject:{code}")

subject_resolver = re.search(
    r"impl LocalDaemonLoopbackSubjectPolicy\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not subject_resolver:
    raise SystemExit("local_daemon_loopback_explicit_subject:subject_policy_impl_missing")
body = subject_resolver.group("body")
if "fn resolve(&self) -> anyhow::Result<String>" not in body:
    raise SystemExit("local_daemon_loopback_explicit_subject:subject_resolve_must_not_take_callee")
if "callee_ura" in body:
    raise SystemExit("local_daemon_loopback_explicit_subject:subject_resolve_reads_callee")

helper = re.search(
    r"pub\(crate\) fn invoke_local_daemon_ability\s*\([^)]*\)\s*->\s*anyhow::Result<serde_json::Value>\s*\{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not helper:
    raise SystemExit("local_daemon_loopback_explicit_subject:generic_helper_missing")
helper_body = helper.group("body")
for token, code in (
    ("let subject_ura = local_daemon_identity_ura()?", "generic_helper_subject_resolution_missing"),
    ("LocalDaemonLoopbackTuplePlan::local_root_for_subject", "generic_helper_explicit_subject_plan_missing"),
    ("&subject_ura", "generic_helper_subject_not_bound_to_plan"),
):
    if token not in helper_body:
        raise SystemExit(f"local_daemon_loopback_explicit_subject:{code}")

for token, code in (
    ("loopback_invoke_request_does_not_pre_resolve_descriptor_ref", "descriptor_projection_test_missing"),
    ("loopback_tuple_plan_requires_explicit_targeted_subject", "explicit_subject_test_missing"),
):
    if token not in text:
        raise SystemExit(f"local_daemon_loopback_explicit_subject:{code}")
PY
}

check_local_ability_target_subject_policy_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local target="$cli_root/src/daemon/invocation/routing/target.rs"
  local local_invoke="$cli_root/src/support/platform/local_invoke.rs"
  local mcp_bridge="$cli_root/src/daemon/ability/builtins/integrations/mcp/bridge.rs"
  local a2a_bridge="$cli_root/src/daemon/ability/builtins/integrations/a2a/bridge.rs"
  local mcp_profile="$cli_root/src/daemon/ability/catalog/profiles/mcp.rs"

  for path in "$target" "$local_invoke" "$mcp_bridge" "$a2a_bridge" "$mcp_profile"; do
    [[ -f "$path" ]] || fail "local ability target subject policy source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$target" "$local_invoke" "$mcp_bridge" "$a2a_bridge" "$mcp_profile" <<'PY'
import sys
from pathlib import Path

target, local_invoke, mcp_bridge, a2a_bridge, mcp_profile = [
    Path(arg).read_text(encoding="utf-8", errors="replace") for arg in sys.argv[1:]
]

production = "\n".join(
    text.split("\nmod tests {", 1)[0].split("\n#[cfg(test)]", 1)[0]
    for text in (target, local_invoke, mcp_bridge, a2a_bridge, mcp_profile)
)
if "default_subject_ura" in production:
    raise SystemExit("local_ability_target_subject_policy:default_subject_accessor_leaked")

for token, code in (
    ("fn daemon_system_subject_ura_for_descriptor(", "descriptor_policy_missing"),
    ("pub(crate) fn daemon_system_subject_ura(&self) -> anyhow::Result<String>", "target_policy_method_missing"),
    ("pub fn local_root_for_target(", "target_issuer_missing"),
    ("pub struct LocalTargetRootInvocation", "issued_target_invocation_missing"),
    ("pub fn local_target_root(", "issued_target_root_missing"),
):
    if token not in target:
        raise SystemExit(f"local_ability_target_subject_policy:{code}")

for token, code in (
    ("invoke_issued_target_root_timeout", "daemon_system_issued_invoke_helper_missing"),
    ("root_context_for_target", "system_context_target_helper_missing"),
    ("pub fn classify_invoke_failure(err: &anyhow::Error) -> LocalInvokeFailureClass", "failure_classifier_missing"),
    ("pub enum LocalInvokeFailureClass", "failure_class_missing"),
):
    if token not in local_invoke:
        raise SystemExit(f"local_ability_target_subject_policy:{code}")

for retired, code in (
    ("invoke_target_root_derived_subject_timeout", "retired_derived_subject_invoke_helper"),
    ("classify_invoke_error", "retired_error_classifier_name"),
    ("LocalInvokeErrorKind", "retired_error_kind_name"),
    ("fallback executor", "fallback_executor_semantics"),
    ("fallback decisions", "fallback_decision_semantics"),
    ("Falling back to an in-process", "in_process_fallback_semantics"),
):
    if retired in local_invoke:
        raise SystemExit(f"local_ability_target_subject_policy:{code}")

for text, code in (
    (mcp_bridge, "mcp_bridge_not_migrated"),
    (a2a_bridge, "a2a_bridge_not_migrated"),
    (mcp_profile, "mcp_profile_not_migrated"),
):
    if "local_root_for_target" not in text and "root_context_for_target" not in text:
        raise SystemExit(f"local_ability_target_subject_policy:{code}")

for token, code in (
    ("local_system_context_for_agent_target_uses_agent_owner_subject", "agent_subject_regression_missing"),
    ("local_system_context_for_hub_target_uses_ability_subject", "hub_subject_regression_missing"),
):
    if token not in local_invoke:
        raise SystemExit(f"local_ability_target_subject_policy:{code}")
PY
}

check_sdk_principal_projection_fail_closed_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local go_principal="$cli_root/sdk/go/principal.go"
  local py_principal="$cli_root/sdk/python/easynet_sdk/principal.py"
  local go_test="$cli_root/sdk/go/principal_test.go"
  local py_test="$cli_root/sdk/python/tests/test_principal.py"

  for path in "$go_principal" "$py_principal" "$go_test" "$py_test"; do
    [[ -f "$path" ]] || fail "SDK Principal projection contract source is missing: ${path#$cli_root/}"
  done

  "$PYTHON_BIN" - "$go_principal" "$py_principal" "$go_test" "$py_test" <<'PY'
import sys
from pathlib import Path

go_principal = Path(sys.argv[1]).read_text(encoding="utf-8")
py_principal = Path(sys.argv[2]).read_text(encoding="utf-8")
go_test = Path(sys.argv[3]).read_text(encoding="utf-8")
py_test = Path(sys.argv[4]).read_text(encoding="utf-8")

for token in (
    "func principalStringFromMap",
    "func principalStringSliceFromMap",
    "func principalPublicKeyFromMap",
    "func int64FromPrincipalMap",
    "func uint64FromPrincipalMap",
    "func boolFromPrincipalMap",
    "return map[string]any{}",
):
    if token in go_principal:
        raise SystemExit(f"go_principal_projection_legacy_fallback_present:{token}")

for token in (
    "requiredPrincipalMap(output, \"principal\")",
    "requiredPrincipalMapValue(value any, path string) (map[string]any, error)",
    "func requiredPrincipalStringSlice",
    "func requiredPrincipalPublicKey",
    "func requiredPrincipalBool",
    "func optionalPrincipalProjectionString",
):
    if token not in go_principal:
        raise SystemExit(f"go_principal_projection_fail_closed_missing:{token}")

for token in (
    "def _sequence(",
    "def _text(",
    "def _int(",
):
    if token in py_principal:
        raise SystemExit(f"python_principal_projection_legacy_fallback_present:{token}")

for token in (
    "def _optional_mapping",
    "def _optional_sequence",
    "def _required_text_sequence",
    "def _required_public_key",
    "def _required_principal_state",
    "def _required_public_key_binding_state",
    "def _required_principal_proof_kind",
):
    if token not in py_principal:
        raise SystemExit(f"python_principal_projection_fail_closed_missing:{token}")

for text, language, test_name in (
    (
        go_test,
        "go",
        "TestRuntimePrincipalProviderRejectsMalformedPrincipalProjection",
    ),
    (
        py_test,
        "python",
        "test_runtime_principal_provider_rejects_malformed_projection",
    ),
):
    if test_name not in text:
        raise SystemExit(f"{language}_principal_malformed_projection_test_missing")
    for field in (
        "principal root must be object",
        "bindings must be array",
        "binding key id is required",
        "binding public key must decode",
        "grant actions must be string array",
    ):
        if field not in text:
            raise SystemExit(f"{language}_principal_malformed_projection_case_missing:{field}")
PY
}

check_runtime_owner_signer_custody_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local self_identity="$cli_root/src/daemon/identity/self_identity.rs"
  [[ -f "$self_identity" ]] || return 0

  "$PYTHON_BIN" - "$self_identity" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
for token in (
    "fn validate_runtime_owner_signing_ura(owner_ura: &str)",
    "runtime-owner signing identity does not manage User URAs; use managed user signing custody",
    "runtime_owner_signing_identity_rejects_user_before_keyring_lookup",
):
    if token not in text:
        raise SystemExit(f"runtime_owner_signer_custody_missing:{token}")

impl = re.search(
    r"impl\s+RuntimeSigningIdentity\s*\{(?P<body>.*?)\n\}\n\n#\[async_trait::async_trait\]",
    text,
    re.DOTALL,
)
if impl is None:
    raise SystemExit("runtime_owner_signer_impl_not_inspectable")
body = impl.group("body")
guard = body.find("validate_runtime_owner_signing_ura(owner_ura)?")
lookup = body.find("provider.public_key(owner_ura)?")
if guard < 0 or lookup < 0 or guard > lookup:
    raise SystemExit("runtime_owner_signer_lookup_before_custody_classification")
PY
}

check_remote_invocation_signer_first_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local remote_invoke="$cli_root/src/daemon/invocation/routing/remote_invoke.rs"
  [[ -f "$remote_invoke" ]] || fail "remote invocation source is missing: ${remote_invoke#$cli_root/}"

  "$PYTHON_BIN" - "$remote_invoke" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
production = text.split("\n#[cfg(test)]", 1)[0]

def fn_body(name: str, next_name: str) -> str:
    start = production.find(f"fn {name}")
    if start < 0:
        raise SystemExit(f"remote_invocation_signer_first:missing:{name}")
    end = production.find(f"\nfn {next_name}", start)
    if end < 0:
        end = len(production)
    return production[start:end]

for required in (
    "enum RemoteInvocationCarrier",
    "fn load_remote_invocation_caller_signer_for_carrier(",
    "RemoteInvocationCarrier::Unary",
    "RemoteInvocationCarrier::Stream",
    "RemoteInvocationCarrier::Bidi",
):
    if required not in production:
        raise SystemExit(f"remote_invocation_signer_first:missing:{required}")

unary = fn_body("invoke_remote_target", "load_remote_invocation_caller_signer")
stream = fn_body("invoke_remote_target_stream", "invoke_remote_target_bidi_json_frames")
bidi = fn_body("invoke_remote_target_bidi_json_frames", "checked_remote_invocation_ura")

checks = (
    ("unary", unary, r"load_remote_invocation_caller_signer\s*\(\s*request\.caller_ura\.as_str\s*\(\s*\)\s*\)\s*\?", r"ensure_remote_invocation_daemon_accepting\s*\(\s*&socket_path\s*\)\s*\?"),
    ("stream", stream, r"load_remote_invocation_caller_signer_for_carrier\s*\(\s*&caller_ura\s*,\s*RemoteInvocationCarrier::Stream\s*,?\s*\)\s*\?", r"probe_accepting\s*\(\s*&socket_path\s*\)"),
    ("bidi", bidi, r"load_remote_invocation_caller_signer_for_carrier\s*\(\s*&caller_ura\s*,\s*RemoteInvocationCarrier::Bidi\s*,?\s*\)\s*\?", r"probe_accepting\s*\(\s*&socket_path\s*\)"),
)
for name, body, signer_pattern, daemon_pattern in checks:
    signer_match = re.search(signer_pattern, body)
    daemon_match = re.search(daemon_pattern, body)
    if signer_match is None:
        raise SystemExit(f"remote_invocation_signer_first:{name}:signer_precondition_missing")
    if daemon_match is None:
        raise SystemExit(f"remote_invocation_signer_first:{name}:daemon_probe_missing")
    if signer_match.start() > daemon_match.start():
        raise SystemExit(f"remote_invocation_signer_first:{name}:daemon_probe_before_signer")

for carrier_body, name in ((stream, "stream"), (bidi, "bidi")):
    if "load_runtime_caller_signer(caller_ura.clone())" in carrier_body:
        raise SystemExit(f"remote_invocation_signer_first:{name}:duplicated_signer_loader")

for required_test in (
    "remote_unary_loads_caller_signer_before_daemon_socket_probe",
    "remote_stream_loads_caller_signer_before_daemon_socket_probe",
    "remote_bidi_loads_caller_signer_before_daemon_socket_probe",
):
    if required_test not in text:
        raise SystemExit(f"remote_invocation_signer_first:missing_test:{required_test}")
PY
}

check_daemon_runtime_identity_vocabulary_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local identity="$cli_root/src/daemon/identity/local_invocation.rs"
  local authority="$cli_root/src/daemon/ability/authority/mod.rs"

  "$PYTHON_BIN" - "$identity" "$authority" <<'PY'
import sys
from pathlib import Path

identity_path, authority_path = map(Path, sys.argv[1:])

for path in (identity_path, authority_path):
    if not path.exists():
        continue
    text = path.read_text()
    for forbidden in (
        "Product URA",
        "Product-level authority fact",
        "product-level authority fact",
    ):
        if forbidden in text:
            raise SystemExit(
                f"daemon_runtime_identity_vocabulary_product_shape:{path}:{forbidden}"
            )

identity = identity_path.read_text() if identity_path.exists() else ""
if identity and "Runtime-published URA owned by the local daemon process" not in identity:
    raise SystemExit("daemon_runtime_identity_vocabulary_missing_runtime_published_identity_doc")

authority = authority_path.read_text() if authority_path.exists() else ""
if authority and "Runtime-local authority fact for a local hosted-agent call" not in authority:
    raise SystemExit("daemon_runtime_identity_vocabulary_missing_runtime_local_authority_doc")
PY
}

check_key_custody_boundary_contract() {
  bash "$ROOT/tools/scripts/check-daemon-key-service-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-product-key-custody-boundary.sh" >/dev/null
}

check_daemon_mission_eal_boundary_contract() {
  bash "$ROOT/tools/scripts/check-dispatch-mission-context-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-runtime-abilities-manifest-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-orchestration-service-boundary.sh" >/dev/null
}

check_product_identity_boundary_contract() {
  bash "$ROOT/tools/scripts/check-plugin-control-subject-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-current-realm-hub-context-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-call-create-participant-identity-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-runtime-state-read-subject-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-status-pairing-state-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-start-credential-readiness-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-reset-credential-state-boundary.sh" >/dev/null
  bash "$ROOT/tools/scripts/check-start-ready-signer-proof-boundary.sh" >/dev/null
}

check_axon_product_protocol_boundary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for product protocol boundary contract: $AXON_ROOT"
  fi

  local path
  for path in \
    core/proto/axon/v1/voice.proto \
    core/proto/axon/v1/remote_desktop.proto \
    core/runtime-rs/client-sdk/proto/axon/v1/voice.proto \
    core/runtime-rs/client-sdk/proto/axon/v1/remote_desktop.proto \
    sdk/rust/proto/axon/v1/voice.proto \
    sdk/rust/proto/axon/v1/remote_desktop.proto \
    sdk/rust/src/audio.rs \
    sdk/rust/src/mcp.rs \
    sdk/rust/src/voice.rs \
    sdk/rust/src/remote_desktop.rs \
    sdk/rust/src/federation_directory.rs \
    sdk/go/easynet/audio.go \
    sdk/go/easynet/audio_stub.go \
    sdk/go/easynet/tool_adapter.go \
    sdk/go/easynet/mcp/server.go \
    sdk/python/axon_sdk/audio.py \
    sdk/python/axon_sdk/tool_adapter.py \
    sdk/python/axon_sdk/mcp/server.py \
    sdk/python/axon_sdk/presets/remote_control/descriptor.py \
    sdk/node/src/audio.ts \
    sdk/node/src/tool_adapter.ts \
    sdk/node/src/mcp/server.ts \
    sdk/node/src/presets/remote_control/descriptor.ts \
    sdk/node/src/presets/ability_dispatch.ts \
    sdk/node/src/presets/remote_control_case.ts \
    sdk/react/src/tool_adapter.ts \
    sdk/react/src/tool_adapter.js \
    sdk/react/src/tool_adapter.d.ts \
    sdk/react/dist/types/tool_adapter.d.ts \
    sdk/java/src/main/java/run/easynet/axon/Audio.java \
    sdk/java/src/main/java/run/easynet/axon/AbilityToolAdapter.java \
    sdk/java/src/main/java/run/easynet/axon/AxonMcpException.java \
    sdk/java/src/main/java/run/easynet/axon/DeployMcpListDirRequest.java \
    sdk/java/src/main/java/run/easynet/axon/UpdateMcpListDirRequest.java \
    sdk/java/src/main/java/run/easynet/axon/VoiceBridge.java \
    sdk/java/src/main/java/run/easynet/axon/VoiceService.java \
    sdk/java/src/main/java/run/easynet/axon/mcp/StdioMcpServer.java \
    sdk/java/src/main/java/run/easynet/axon/presets/remote_control/RemoteControlDescriptor.java \
    sdk/java/src/main/java/run/easynet/axon/cases/ability_dispatch/AbilityDispatchCase.java \
    sdk/swift/Sources/EasyNetAxon/Audio.swift \
    sdk/swift/Sources/EasyNetAxon/StdioMcpServer.swift \
    sdk/swift/Sources/EasyNetAxon/ToolAdapter.swift
  do
    [[ ! -e "$AXON_ROOT/$path" ]] \
      || fail "product-owned file remains in canonical Axon surface: $path"
  done

  if [[ -d "$AXON_ROOT/sdk/go" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/go 2>/dev/null | grep -Eq '/(audio|voice|tool_adapter|mcp)([^/]*|/.*)$'); then
    fail "Go SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/python" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/python 2>/dev/null | grep -Eq '/(audio|tool_adapter|mcp|presets/(remote_control|ability_dispatch|federation))([^/]*|/.*)$'); then
    fail "Python SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/node" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/node 2>/dev/null | grep -Eq '/(audio|tool_adapter|mcp|presets/(remote_control|ability_dispatch)|remote_control_case)([^/]*|/.*)$'); then
    fail "Node SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/react" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/react 2>/dev/null | grep -Eq '/tool_adapter(\.[^/]+)?$'); then
    fail "React SDK tracks a product-owned canonical package"
  fi
  local react_product_paths=()
  for path in \
    "$AXON_ROOT/sdk/react/src" \
    "$AXON_ROOT/sdk/react/README.md" \
    "$AXON_ROOT/sdk/react/SKILL.md"
  do
    [[ -e "$path" ]] && react_product_paths+=("$path")
  done
  if ((${#react_product_paths[@]} > 0)) \
    && rg -n '\b(tool_adapter|useAbilityTools|AbilityTool(Renderer|Invocation|Result|Options)?|AbilityTools)\b' "${react_product_paths[@]}"; then
    fail "React SDK exposes product-owned tool-adapter surface"
  fi

  if [[ -d "$AXON_ROOT/sdk/java" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/java 2>/dev/null | grep -Eq '/(Audio|AbilityToolAdapter|AxonMcpException|DeployMcpListDirRequest|UpdateMcpListDirRequest|VoiceBridge|VoiceService)\.java$|/(mcp|presets/remote_control|cases/ability_dispatch)/'); then
    fail "Java SDK tracks a product-owned canonical package"
  fi

  if [[ -d "$AXON_ROOT/sdk/swift" ]] \
    && (cd "$AXON_ROOT" && git ls-files sdk/swift 2>/dev/null | grep -Eq '/(Audio|StdioMcpServer|ToolAdapter)\.swift$'); then
    fail "Swift SDK tracks a product-owned canonical package"
  fi

  local rust_lib="$AXON_ROOT/sdk/rust/src/lib.rs"
  if [[ -f "$rust_lib" ]] \
    && grep -Eq 'pub (mod|use) (audio|mcp|voice|remote_desktop|presets|tool_adapter|federation_directory)\b|DeviceJoinCredentialEnvelope|DirectoryAgentSummary|ListUserDevices(Request|Response)' "$rust_lib"; then
    fail "Rust SDK exports a product-owned module"
  fi

  local proto_root="$AXON_ROOT/core/proto/axon/v1"
  if [[ -d "$proto_root" ]] \
    && grep -R -nE '^[[:space:]]*(message|service|enum)[[:space:]]+(Mcp|MCP|Voice|RemoteDesktop|EasyNet)' "$proto_root"; then
    fail "canonical Axon proto declares a product protocol type"
  fi

  local proto_mirrors=(
    "$AXON_ROOT/core/proto/axon/v1"
    "$AXON_ROOT/core/runtime-rs/client-sdk/proto/axon/v1"
    "$AXON_ROOT/sdk/rust/proto/axon/v1"
  )
  existing_proto_mirrors=()
  for path in "${proto_mirrors[@]}"; do
    [[ -d "$path" ]] && existing_proto_mirrors+=("$path")
  done
  if ((${#existing_proto_mirrors[@]} > 0)) \
    && grep -R -nE '\b(McpToolSpec|McpToolTarget|EasyNetContext|EasyNetHook|ObjectiveWeights)\b' "${existing_proto_mirrors[@]}"; then
    fail "canonical Axon proto mirrors contain a product protocol type"
  fi

  local dendrite_paths=(
    "$AXON_ROOT/core/runtime-rs/dendrite-bridge/src"
    "$AXON_ROOT/core/runtime-rs/dendrite-bridge/include"
    "$AXON_ROOT/packaging/sdk-pack/build_sdk_packs.sh"
  )
  existing_dendrite_paths=()
  for path in "${dendrite_paths[@]}"; do
    [[ -e "$path" ]] && existing_dendrite_paths+=("$path")
  done
  if ((${#existing_dendrite_paths[@]} > 0)) \
    && grep -R -n 'axon_dendrite_voice_' "${existing_dendrite_paths[@]}"; then
    fail "Dendrite exports a voice product client"
  fi

  for path in \
    core/runtime-rs/build.rs \
    core/runtime-rs/client-sdk/build.rs \
    sdk/rust/build.rs
  do
    if [[ -f "$AXON_ROOT/$path" ]]; then
      grep -q 'CANONICAL_AXON_PROTO_FILES' "$AXON_ROOT/$path" \
        || fail "$path does not use the canonical proto allowlist"
    fi
  done

  local rfc004="$AXON_ROOT/document/rfcs/004-mcp-binding.md"
  if [[ -f "$rfc004" ]]; then
    grep -q 'Withdrawn from Axon canonical protocol' "$rfc004" \
      || fail "RFC 004 still claims Axon MCP ownership"
  fi
  local sdk_parity="$AXON_ROOT/sdk/SDK_PARITY.md"
  if [[ -f "$sdk_parity" ]]; then
    grep -q '^## Product Boundary$' "$sdk_parity" \
      || fail "SDK parity does not declare the product ownership boundary"
  fi
}

check_axon_plain_proof_public_boundary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for plain proof boundary contract: $AXON_ROOT"
  fi
  local cli_root="${CLI_ROOT:-$ROOT}"

  local active_text_paths=()
  for path in \
    "$AXON_ROOT/document/rfcs/001-envelope-axiom-alignment.md" \
    "$AXON_ROOT/document/rfcs/001-pr2-acceptance-checklist.md" \
    "$AXON_ROOT/sdk/conformance/cases/axiom/axiom-admission-pipeline.json" \
    "$AXON_ROOT/sdk/conformance/cases/axiom/axiom-worked-example-authenticated.json" \
    "$AXON_ROOT/sdk/go/axon/dendrite_bridge_signed_invoke_cgo.go" \
    "$AXON_ROOT/sdk/go/axon/invocation/axiom.go" \
    "$AXON_ROOT/sdk/java/src/test/java/run/axon/sdk/invocation/AxiomWorkedExampleTest.java" \
    "$AXON_ROOT/sdk/python/axon_sdk/invocation/axiom.py"
  do
    [[ -f "$path" ]] && active_text_paths+=("$path")
  done
  if ((${#active_text_paths[@]} > 0)) \
    && rg -n '\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature)\b|\bcanonicalInvocationBytes\b|plain canonical invocation|client-sdk::admission::canonical_invocation_bytes' "${active_text_paths[@]}"; then
    fail "Axon active proof documents preserve retired plain proof/admission vocabulary"
  fi

  local rust_invocation="$AXON_ROOT/sdk/rust/src/invocation"
  if [[ -d "$rust_invocation" ]] \
    && rg -n 'pub fn (canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission)\b|pub use (admission|axiom)::\{[^}]*\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission)\b' "$rust_invocation"; then
    fail "Axon Rust exposes plain proof/admission helpers"
  fi
  if [[ -d "$rust_invocation" ]] \
    && rg -n '\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission|legacy_plain_invocation_bytes|sign_legacy_plain_invocation|verify_legacy_plain_invocation_signature|verify_legacy_plain_signature|verify_phase_legacy_plain|run_legacy_plain_admission)\b|legacy_plain_invocation_bytes_empty' "$rust_invocation"; then
    fail "Axon Rust invocation source preserves retired plain proof/admission helper names"
  fi

  local python_invocation="$AXON_ROOT/sdk/python/axon_sdk/invocation"
  if [[ -d "$python_invocation" ]] \
    && rg -n '^def (canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|run_admission)\b|from \.axiom import \([^)]*\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature)\b|from \.admission import \([^)]*\b(verify_signature|run_admission)\b|"(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|run_admission)"' "$python_invocation"; then
    fail "Axon Python exposes plain proof/admission helpers"
  fi
  local python_sdk="$AXON_ROOT/sdk/python"
  if [[ -d "$python_sdk" ]] \
    && rg -n '\b(_canonical_invocation_bytes|_sign_invocation|_verify_invocation_signature|_verify_signature|_run_admission|_legacy_plain_invocation_bytes|_sign_legacy_plain_invocation|_verify_legacy_plain_invocation_signature|_verify_legacy_plain_signature|_run_legacy_plain_admission|canonical_invocation_bytes_empty|legacy_plain_invocation_bytes_empty)\b' "$python_sdk"; then
    fail "Axon Python source preserves retired plain proof/admission helper names"
  fi

  local go_invocation="$AXON_ROOT/sdk/go/axon/invocation"
  local go_plain_paths=()
  [[ -d "$go_invocation" ]] && go_plain_paths+=("$go_invocation")
  [[ -f "$AXON_ROOT/sdk/API_MAPPING.md" ]] && go_plain_paths+=("$AXON_ROOT/sdk/API_MAPPING.md")
  [[ -d "$cli_root/sdk/go" ]] && go_plain_paths+=("$cli_root/sdk/go")
  if ((${#go_plain_paths[@]} > 0)) \
    && rg -n '^func (CanonicalInvocationBytes|SignInvocation|VerifyInvocationSignature|VerifySignature|RunAdmission)\b|\b(CanonicalInvocationBytes|SignInvocation|VerifyInvocationSignature|VerifySignature|RunAdmission)\b' "${go_plain_paths[@]}"; then
    fail "Axon Go exposes plain proof/admission helpers"
  fi
  if [[ -d "$go_invocation" ]] \
    && rg -n '\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' "$go_invocation" \
      --glob '!**/*_test.go'; then
    fail "Axon Go production invocation source preserves retired plain proof/admission helper names"
  fi
  if [[ -d "$go_invocation" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|legacy_plain_invocation_bytes_empty' "$go_invocation" \
      --glob '!**/*_test.go'; then
    fail "Axon Go production invocation source preserves legacy plain proof/admission helper names"
  fi
  if [[ -d "$go_invocation" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|legacy_plain_invocation_bytes_empty' "$go_invocation"; then
    fail "Axon Go invocation package preserves legacy plain proof/admission helper names"
  fi

  local node_invocation="$AXON_ROOT/sdk/node/src/invocation"
  local node_plain_paths=()
  [[ -d "$node_invocation" ]] && node_plain_paths+=("$node_invocation")
  [[ -f "$AXON_ROOT/sdk/node/src/index.ts" ]] && node_plain_paths+=("$AXON_ROOT/sdk/node/src/index.ts")
  [[ -f "$AXON_ROOT/sdk/node/src/index.js" ]] && node_plain_paths+=("$AXON_ROOT/sdk/node/src/index.js")
  [[ -f "$AXON_ROOT/sdk/node/src/index.d.ts" ]] && node_plain_paths+=("$AXON_ROOT/sdk/node/src/index.d.ts")
  if ((${#node_plain_paths[@]} > 0)) \
    && rg -n '\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' "${node_plain_paths[@]}" \
      --glob '!**/tests/**' \
      --glob '!**/*.test.*'; then
    fail "Axon Node exposes plain proof/admission helpers"
  fi
  if [[ -d "$node_invocation" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|canonical_invocation_bytes_empty' "$node_invocation"; then
    fail "Axon Node production invocation source preserves legacy plain proof/admission exports"
  fi
  local node_sdk="$AXON_ROOT/sdk/node"
  if [[ -d "$node_sdk" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|legacy_plain_invocation|canonical_invocation_bytes unexpectedly empty' "$node_sdk" \
      --glob '!**/node_modules/**'; then
    fail "Axon Node SDK preserves legacy plain proof/admission helper names"
  fi

  local java_invocation="$AXON_ROOT/sdk/java/src/main/java/run/axon/sdk/invocation"
  if [[ -d "$java_invocation" ]] \
    && rg -n 'public static [^{;=]+ (canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' "$java_invocation"; then
    fail "Axon Java exposes plain proof/admission helpers"
  fi
  if [[ -d "$java_invocation" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission|verifyPhaseLegacyPlain)\b|canonical_invocation_bytes_empty' "$java_invocation"; then
    fail "Axon Java production invocation source preserves legacy plain proof/admission helpers"
  fi

  local swift_invocation="$AXON_ROOT/sdk/swift/Sources/AxonSDK/Invocation"
  local swift_plain_paths=()
  [[ -d "$swift_invocation" ]] && swift_plain_paths+=("$swift_invocation")
  [[ -f "$AXON_ROOT/sdk/swift/README.md" ]] && swift_plain_paths+=("$AXON_ROOT/sdk/swift/README.md")
  [[ -d "$AXON_ROOT/sdk/swift/Examples" ]] && swift_plain_paths+=("$AXON_ROOT/sdk/swift/Examples")
  if ((${#swift_plain_paths[@]} > 0)) \
    && rg -n 'public func (canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b|\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' "${swift_plain_paths[@]}"; then
    fail "Axon Swift exposes plain proof/admission helpers"
  fi
  if [[ -d "$swift_invocation" ]] \
    && rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission|verifyPhaseLegacyPlain)\b|legacy_plain_invocation_bytes_empty|canonical_invocation_bytes_empty' "$swift_invocation"; then
    fail "Axon Swift production invocation source preserves legacy plain proof/admission helpers"
  fi
}

check_axon_rust_local_fast_signer_boundary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for Rust local-fast signer boundary contract: $AXON_ROOT"
  fi

  local rust_manifest="$AXON_ROOT/sdk/rust/Cargo.toml"
  if [[ -f "$rust_manifest" ]] && rg -n '\blocal-fast-probes\b' "$rust_manifest"; then
    fail "Axon Rust SDK still exposes local-fast signer probe feature"
  fi

  local rust_invocation="$AXON_ROOT/sdk/rust/src/invocation"
  if [[ -d "$rust_invocation" ]] && rg -n 'feature = "local-fast-probes"' "$rust_invocation"; then
    fail "Axon Rust SDK still gates signer fallback helpers behind a public feature"
  fi

  local rust_external_consumers=()
  [[ -d "$AXON_ROOT/sdk/rust/examples" ]] && rust_external_consumers+=("$AXON_ROOT/sdk/rust/examples")
  [[ -d "$AXON_ROOT/sdk/rust/tests" ]] && rust_external_consumers+=("$AXON_ROOT/sdk/rust/tests")
  if ((${#rust_external_consumers[@]} > 0)) \
    && rg -n '\b(LocalReceiptSigningAuthorityProvider|Ed25519ReceiptSigningAuthority|StaticReceiptSigningAuthorityProvider|Ed25519InvocationSigningAuthority|StaticInvocationSigningAuthorityProvider|new_local_fast|new_local_fast_with_limits)\b' "${rust_external_consumers[@]}" \
      --glob '!signed_receipt_api_gate.rs'; then
    fail "Axon Rust examples/tests still consume process-local signer fallback helpers"
  fi
}

check_axon_process_local_signer_fallback_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for process-local signer fallback contract: $AXON_ROOT"
  fi

  local fallback_paths=()
  for path in \
    "$AXON_ROOT/core/runtime-rs/client-sdk/src" \
    "$AXON_ROOT/core/runtime-rs/src" \
    "$AXON_ROOT/sdk/rust/src" \
    "$AXON_ROOT/sdk/go/axon" \
    "$AXON_ROOT/sdk/python/axon_sdk" \
    "$AXON_ROOT/sdk/node/src" \
    "$AXON_ROOT/sdk/java/src/main/java/run/axon" \
    "$AXON_ROOT/sdk/swift/Sources/AxonSDK"
  do
    [[ -e "$path" ]] && fallback_paths+=("$path")
  done

  if ((${#fallback_paths[@]} > 0)) \
    && rg -n '\b(default_auth_for_subject|generate_subject_auth|generate_private_agent_auth|generate_private_hub_auth|GeneratedSubjectAuth|ProcessLocalSigner|PrivateKeyAuthenticator|DefaultAuthForSubject|GenerateSubjectAuth|defaultAuthForSubject)\b' "${fallback_paths[@]}" \
      --glob '!**/tests/**' \
      --glob '!**/*_test.go' \
      --glob '!**/*.test.*'; then
    fail "Axon source still exposes process-local signer fallback helpers"
  fi
}

check_cli_rust_local_fast_signer_boundary_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local cli_paths=()
  for path in \
    "$cli_root/Cargo.toml" \
    "$cli_root/src" \
    "$cli_root/tests" \
    "$cli_root/plugins"
  do
    [[ -e "$path" ]] && cli_paths+=("$path")
  done

  if ((${#cli_paths[@]} > 0)) \
    && rg -n '\b(local-fast-probes|LocalReceiptSigningAuthorityProvider|Ed25519ReceiptSigningAuthority|StaticReceiptSigningAuthorityProvider|Ed25519InvocationSigningAuthority|StaticInvocationSigningAuthorityProvider|new_local_fast|new_local_fast_with_limits)\b' "${cli_paths[@]}"; then
    fail "EasyNet-Cli still requests or consumes Rust local-fast signer fallback helpers"
  fi
}

check_cli_signed_submission_boundary_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local client="$cli_root/src/daemon/invocation/dispatch/client.rs"
  local request="$cli_root/src/daemon/invocation/dispatch/request.rs"
  local ffi="$cli_root/src/ffi/invocation/mod.rs"

  [[ -f "$client" ]] || fail "CLI signed submission client is missing: $client"
  [[ -f "$request" ]] || fail "CLI signed submission request model is missing: $request"
  [[ -f "$ffi" ]] || fail "CLI signed submission FFI adapter is missing: $ffi"

  "$PYTHON_BIN" - "$client" "$request" "$ffi" <<'PY'
import re
import sys
from pathlib import Path

client = Path(sys.argv[1]).read_text()
request = Path(sys.argv[2]).read_text()
ffi = Path(sys.argv[3]).read_text()

for method in ("invoke", "invoke_stream", "invoke_bidi"):
    signature = re.search(
        rf"pub\s+async\s+fn\s+{method}\s*\((?P<args>.*?)\)\s*->",
        client,
        re.DOTALL,
    )
    if signature is None:
        raise SystemExit(f"missing_daemon_client_method:{method}")
    args = signature.group("args")
    if not re.search(r"\bsigned\s*:\s*SignedInvocation\b", args):
        raise SystemExit(f"unsigned_daemon_client_submission:{method}")
    if re.search(r"\bDaemonInvocation\b", args):
        raise SystemExit(f"raw_daemon_invocation_submission:{method}")

if "fn signed_envelope(&self) -> Result<axon_sdk::pb::axon::v1::Envelope>" not in request:
    raise SystemExit("missing_signed_wire_envelope_gate")
signed_envelope = request.split("fn signed_envelope", 1)[1].split("fn content_envelope", 1)[0]
if "SignedInvocation state" not in signed_envelope:
    raise SystemExit("unsigned_wire_submission_not_rejected")
if "unwrap_or_default()" in request.split("fn into_bidi_open_frame", 1)[1].split("/// Builder", 1)[0]:
    raise SystemExit("bidi_signature_mac_fallback")

bind = re.search(
    r"async\s+fn\s+bind\s*\(.*?\)\s*->\s*"
    r"crate::daemon::Result<crate::daemon::SignedInvocation>",
    ffi,
    re.DOTALL,
)
if bind is None:
    raise SystemExit("session_authority_does_not_return_signed_state")

for pattern, label in (
    (r"\bclient\.invoke\s*\(\s*invocation\s*\)", "ffi_unary_raw_submission"),
    (r"\bclient\.invoke_stream\s*\(\s*invocation\s*\)", "ffi_stream_raw_submission"),
    (r"\bclient\.invoke_bidi\s*\(\s*invocation\s*,", "ffi_bidi_raw_submission"),
):
    if re.search(pattern, ffi):
        raise SystemExit(label)
PY
}

find_active_rfc_documents() {
  local root="$1"
  [[ -d "$root" ]] || return 0

  while IFS= read -r -d '' path; do
    if ! sed -n '1,20p' "$path" | grep -Fqi 'Historical status'; then
      printf '%s\0' "$path"
    fi
  done < <(
    find "$root" \
      -type f \( -name '*.md' -o -name '*.tex' -o -name '*.txt' \) -print0
  )
}

check_ura_vocabulary_contract() {
  # SDK naming owns public package surfaces. The shared active-token
  # classifier below owns normative prose and distinguishes transport-library
  # `Uri` types from the canonical runtime's URA vocabulary.
  bash "$ROOT/tools/scripts/check-sdk-ura-naming.sh" >/dev/null

  local docs=("$ROOT/docs/spec/canonical-runtime-convergence-v2.md")
  if [[ -d "$ROOT/docs/rfc" ]]; then
    while IFS= read -r -d '' path; do
      docs+=("$path")
    done < <(find_active_rfc_documents "$ROOT/docs/rfc")
  fi

  check_active_ura_transport_classification_contract "${docs[@]}"
}

check_axon_protocol_pack_ura_vector_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for protocol-pack URA vector contract: $AXON_ROOT"
  fi

  local vectors="$AXON_ROOT/packaging/protocol-pack/conformance-vectors"
  if [[ ! -d "$vectors" ]]; then
    return
  fi
  if [[ -e "$vectors/easynet-uri-v1.json" ]]; then
    fail "protocol-pack preserves URI-named URA conformance vector"
  fi
  if [[ ! -e "$vectors/easynet-ura-v1.json" ]]; then
    fail "protocol-pack URA conformance vector is missing"
  fi
  if rg -n '"(input_uri|canonical_uri)"|"[^"]*URI canonicalization[^"]*"' "$vectors"; then
    fail "protocol-pack conformance vectors preserve URI terminology for URA data"
  fi
}

check_axon_normative_ura_document_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for normative URA document contract: $AXON_ROOT"
  fi

  local docs=()
  if [[ -d "$AXON_ROOT/document" ]]; then
    while IFS= read -r -d '' path; do
      docs+=("$path")
    done < <(
      find "$AXON_ROOT/document" \
        \( -path '*/target/*' -o -path '*/node_modules/*' \) -prune \
        -o -type f \( -name '*.md' -o -name '*.tex' -o -name '*.txt' \) -print0
    )
  fi
  if [[ -d "$AXON_ROOT/docs/rfc" ]]; then
    while IFS= read -r -d '' path; do
      docs+=("$path")
    done < <(find_active_rfc_documents "$AXON_ROOT/docs/rfc")
  fi
  for path in \
    "$AXON_ROOT/sdk/SDK_INTERFACE_SPEC.md" \
    "$AXON_ROOT/sdk/FEDERATION_INVOKE_SCHEMAS.md" \
    "$AXON_ROOT/sdk/conformance/cases/axiom/README.md" \
    "$AXON_ROOT/sdk/conformance/cases/axiom/axiom-identity-composite-required.json"
  do
    [[ -f "$path" ]] && docs+=("$path")
  done
  if ((${#docs[@]} == 0)); then
    return
  fi
  check_active_ura_transport_classification_contract "${docs[@]}"
}

check_axon_proto_ura_vocabulary_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for proto URA vocabulary contract: $AXON_ROOT"
  fi

  local proto_roots=()
  for path in \
    "$AXON_ROOT/core/proto/axon/v1" \
    "$AXON_ROOT/core/runtime-rs/client-sdk/proto/axon/v1" \
    "$AXON_ROOT/sdk/rust/proto/axon/v1"
  do
    [[ -d "$path" ]] && proto_roots+=("$path")
  done
  if ((${#proto_roots[@]} == 0)); then
    return
  fi
  if rg -n '\bURI\b|\bURIs\b|<uri>|\b(canonical|device|agent|resource|subject|caller|callee|payload|receipt)[^[:cntrl:]]*\bURI\b|\bURI[^[:cntrl:]]*\b(canonical|device|agent|resource|subject|caller|callee|payload|receipt)\b|_[Uu][Rr][Ii]\b|\b[A-Za-z0-9]+URI\b' "${proto_roots[@]}" --glob '*.proto'; then
    fail "Axon active proto schemas preserve URI terminology for URA identity data"
  fi
}

check_axon_sdk_product_neutral_ura_error_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for SDK product-neutral URA error contract: $AXON_ROOT"
  fi

  local sdk_paths=()
  for path in \
    "$AXON_ROOT/sdk/go/axon" \
    "$AXON_ROOT/sdk/java/src/main/java" \
    "$AXON_ROOT/sdk/node/src" \
    "$AXON_ROOT/sdk/python/axon_sdk" \
    "$AXON_ROOT/sdk/rust/src" \
    "$AXON_ROOT/sdk/swift/Sources" \
    "$AXON_ROOT/sdk/react/src"
  do
    [[ -d "$path" ]] && sdk_paths+=("$path")
  done
  if ((${#sdk_paths[@]} == 0)); then
    return
  fi
  if rg -n '\bEasyNet URA\b|\bEasyNet URAs\b|\bEasyNet URA syntax\b|\bmust be an EasyNet\b|\bmust use EasyNet\b|\bSYSTEM_URI\b' "${sdk_paths[@]}" \
    --glob '!**/node_modules/**' \
    --glob '!**/__pycache__/**' \
    --glob '!**/*.d.ts' \
    --glob '!**/*.test.*' \
    --glob '!**/*_test.go'; then
    fail "Axon SDK active source preserves product-specific URA error vocabulary"
  fi
}

check_axon_active_ura_source_test_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for active URA source/test contract: $AXON_ROOT"
  fi

  local paths=()
  for path in \
    "$AXON_ROOT/core" \
    "$AXON_ROOT/sdk" \
    "$AXON_ROOT/scripts" \
    "$AXON_ROOT/packaging" \
    "$AXON_ROOT/core/runtime-rs/dendrite-bridge/docs/AUTHENTICATED_INVOCATION.md" \
    "$AXON_ROOT/sdk/go/axon/signed_invoke_request_test.go" \
    "$AXON_ROOT/sdk/go/axon/ability_lifecycle_server_test.go"
  do
    [[ -e "$path" ]] && paths+=("$path")
  done
  if ((${#paths[@]} == 0)); then
    return
  fi
  check_active_ura_transport_classification_contract "${paths[@]}"
}

check_active_ura_transport_classification_contract() {
  "$PYTHON_BIN" - "$@" <<'PY'
import re
import sys
from pathlib import Path

roots = [Path(arg) for arg in sys.argv[1:]]
if not roots:
    raise SystemExit("active_ura_transport_classification:missing_roots")

retired = re.compile(
    r"(^|[^A-Za-z0-9])(URI|Uri|uri)([A-Z0-9]|[^A-Za-z0-9]|$)"
    r"|[a-z0-9](URI|Uri)([A-Z0-9]|[^A-Za-z0-9]|$)"
)
transport = re.compile(
    r"\b(?:hyper::Uri|http::Uri|tonic::transport::Uri|url::Url)\b"
    r"|\b(?:hyper|http)::uri::[A-Za-z0-9_]+\b"
    r"|\bbase-uri\b"
    r"|use\s+(?:hyper|tonic::transport)::\{[^}]*\bUri\b[^}]*\}"
    r"|\bconnect_with_connector\b"
    r"|\btower::service_fn\(move \|_:\s*Uri\|"
    r"|\breq\.uri\b"
    r"|\breq\.uri\(\)"
    r"|\.uri\("
)
transport_target = re.compile(r"\btarget_uri\b|\brequest_uri\b")
semantic = re.compile(
    r"\b(?:ability|agent|callee|caller|device|invocation|owner|principal|receipt|resource|subject)"
    r"[A-Za-z0-9_]*(?:uri|url|address)\b"
    r"|\b(?:uri|url|address)[A-Za-z0-9_]*(?:ability|agent|callee|caller|device|invocation|owner|principal|receipt|resource|subject)\b",
    re.IGNORECASE,
)
ura = re.compile(r"ura", re.IGNORECASE)
skip_parts = {
    ".git",
    ".gradle",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    ".venv-test",
    ".build",
    "target",
    "build",
    "dist",
    "site-packages",
    "node_modules",
    "__pycache__",
}

def iter_files(root: Path):
    if root.is_file():
        yield root
        return
    if not root.exists():
        return
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        parts = set(path.parts)
        if parts & skip_parts:
            continue
        if path.name.endswith(".egg-info") or any(part.endswith(".egg-info") for part in path.parts):
            continue
        if "/tests/scripts/" in path.as_posix():
            continue
        if path.suffix in {".png", ".jpg", ".jpeg", ".gif", ".webp", ".wasm", ".lock"}:
            continue
        yield path

violations = []
for root in roots:
    for path in iter_files(root):
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for line_number, line in enumerate(text.splitlines(), start=1):
            if not retired.search(line):
                continue
            if transport.search(line):
                continue
            if transport_target.search(line) and "easynet:///" not in line:
                continue
            match = semantic.search(line)
            if match and ura.search(match.group(0)):
                continue
            violations.append(f"{path}:{line_number}:{line.strip()}")

if violations:
    print(
        "active source still uses URI terminology outside transport-library APIs:",
        file=sys.stderr,
    )
    print("\n".join(violations), file=sys.stderr)
    raise SystemExit(1)
PY
}

run_ura_vocabulary_self_test() {
  local fixture_root="$1"
  mkdir -p "$fixture_root/active-rfc-text"

  printf '%s\n' \
    'use tonic::transport::{Channel, Endpoint, Uri};' \
    'let _ = endpoint.connect_with_connector(tower::service_fn(move |_: Uri| async {}));' \
    'let path = req.uri().path().to_string();' \
    'let request = hyper::Request::builder().uri("/v1/models");' \
    'let target_uri: hyper::Uri = "http://127.0.0.1/mcp".parse().unwrap();' \
    "let policy = \"default-src 'self'; base-uri 'none'\";" \
    > "$fixture_root/transport-uri.rs"
  printf '%s\n' \
    'const caller_uri: &str = "easynet:///r/example/agent/alice";' \
    'fn rejects_empty_callee_URI() {}' \
    > "$fixture_root/semantic-uri.rs"

  check_active_ura_transport_classification_contract "$fixture_root/transport-uri.rs"
  if check_active_ura_transport_classification_contract "$fixture_root/semantic-uri.rs" >/dev/null 2>&1; then
    fail "self-test expected semantic URI terminology to fail"
  fi

  printf 'Rule 1 - hosted URI persistence\n' \
    > "$fixture_root/active-rfc-text/active-baseline.txt"
  printf 'Historical status: archived terminology fixture\nhosted URI persistence\n' \
    > "$fixture_root/active-rfc-text/historical-baseline.txt"
  local active_text_docs=()
  while IFS= read -r -d '' path; do
    active_text_docs+=("$path")
  done < <(find_active_rfc_documents "$fixture_root/active-rfc-text")
  if check_active_ura_transport_classification_contract "${active_text_docs[@]}" >/dev/null 2>&1; then
    fail "self-test expected active RFC .txt semantic URI terminology to fail"
  fi

  printf 'HTTP transport uses http::Uri and base-uri policy directives.\n' \
    > "$fixture_root/active-rfc-text/active-baseline.txt"
  active_text_docs=()
  while IFS= read -r -d '' path; do
    active_text_docs+=("$path")
  done < <(find_active_rfc_documents "$fixture_root/active-rfc-text")
  check_active_ura_transport_classification_contract "${active_text_docs[@]}"
}

check_schema_source_derivation_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for schema-source derivation contract: $AXON_ROOT"
  fi

  local checker="$AXON_ROOT/scripts/checks/check_proto_derivation.sh"
  local cli_root="${CLI_ROOT:-$ROOT}"
  if [[ ! -f "$checker" ]]; then
    fail "Axon proto source derivation gate is missing: ${checker#$AXON_ROOT/}"
  fi

  if ! EASYNET_CLI_ROOT="$cli_root" \
    AXON_PROTO_DERIVATION_ROOT="$AXON_ROOT" \
    bash "$checker" --check >/dev/null; then
    fail "Axon proto mirrors diverged from canonical core/proto source"
  fi
}

check_axon_benchmark_baseline_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for benchmark baseline contract: $AXON_ROOT"
  fi

  local checker="$AXON_ROOT/scripts/checks/check_benchmark_baselines.py"
  local baseline="$AXON_ROOT/sdk/rust/benches/baseline-v2.json"
  [[ -f "$checker" ]] || fail "Axon benchmark baseline checker is missing: ${checker#$AXON_ROOT/}"
  [[ -f "$baseline" ]] || fail "Axon benchmark baseline is missing: ${baseline#$AXON_ROOT/}"

  if ! PYTHONDONTWRITEBYTECODE=1 python3 "$checker" \
    --root "$AXON_ROOT" \
    --baseline "$baseline" >/dev/null; then
    fail "Axon canonical LocalRuntime V2 benchmark baseline is invalid"
  fi
}

check_receipt_proof_fact_contract() {
  if [[ ! -d "$AXON_ROOT" ]]; then
    fail "EasyNet-Axon root not found for receipt proof-fact contract: $AXON_ROOT"
  fi

  local java_axiom="$AXON_ROOT/sdk/java/src/main/java/run/axon/sdk/invocation/Axiom.java"
  local java_bundle="$AXON_ROOT/sdk/java/src/main/java/run/axon/sdk/invocation/Bundle.java"
  local java_local_runtime="$AXON_ROOT/sdk/java/src/main/java/run/axon/sdk/invocation/LocalRuntime.java"
  local java_receipt_paths=()
  local python_axiom="$AXON_ROOT/sdk/python/axon_sdk/invocation/axiom.py"
  local python_receipt_paths=()
  local node_invocation="$AXON_ROOT/sdk/node/src/invocation"
  local node_local_runtime="$AXON_ROOT/sdk/node/src/invocation/local-runtime.ts"
  local node_receipt_paths=()
  local swift_invocation="$AXON_ROOT/sdk/swift/Sources/AxonSDK/Invocation"
  local swift_receipt_paths=()
  local go_invocation="$AXON_ROOT/sdk/go/axon/invocation"
  local go_local_runtime="$AXON_ROOT/sdk/go/axon/invocation/local_runtime.go"
  local rust_invocation="$AXON_ROOT/sdk/rust/src/invocation"
  local rust_axiom="$AXON_ROOT/sdk/rust/src/invocation/axiom.rs"
  local runtime_client_admission="$AXON_ROOT/core/runtime-rs/client-sdk/src/domain/admission.rs"
  [[ -d "$AXON_ROOT/sdk/java/src/main/java/run/axon" ]] && java_receipt_paths+=("$AXON_ROOT/sdk/java/src/main/java/run/axon")
  [[ -d "$AXON_ROOT/sdk/python/axon_sdk" ]] && python_receipt_paths+=("$AXON_ROOT/sdk/python/axon_sdk")
  [[ -d "$AXON_ROOT/sdk/node/src" ]] && node_receipt_paths+=("$AXON_ROOT/sdk/node/src")
  [[ -d "$swift_invocation" ]] && swift_receipt_paths+=("$swift_invocation")

  if rg -n 'AuthorityBinding\.self\(callerBinding\.ura\)|ReceiptProofFacts\.empty\(\)\);' "$java_axiom" "$java_bundle"; then
    fail "Java receipt construction/parsing still synthesizes authority or proof facts"
  fi

  if rg -n 'ReceiptProofFacts\.empty\(\)' "$java_local_runtime"; then
    fail "Java LocalRuntime still emits receipts with empty proof facts"
  fi

  if ((${#java_receipt_paths[@]} > 0)) \
    && rg -n 'InvocationAuthorityProof\.empty\(\)|static\s+InvocationAuthorityProof\s+empty\s*\(' "${java_receipt_paths[@]}"; then
    fail "Java SDK/tests/examples still expose or use empty authority proof facts"
  fi

  if rg -n 'field\(default_factory=ReceiptProofFacts\)|AuthorityBinding\.self_\(r\.caller_binding\.ura\)|proof_facts if .*else .*ReceiptProofFacts\(\)' "$python_axiom" "$AXON_ROOT/sdk/python/axon_sdk/invocation/audit.py"; then
    fail "Python receipt construction still defaults authority or proof facts"
  fi

  if ((${#python_receipt_paths[@]} > 0)) \
    && ! "$PYTHON_BIN" - "${python_receipt_paths[@]}" <<'PY'
import ast
import sys
from pathlib import Path

authority_fields = {
    "proof_type",
    "binding",
    "proof_payload",
    "proof_hash",
    "issuer",
    "signature",
    "admission_hook",
}

violations = []
for root in map(Path, sys.argv[1:]):
    paths = [root] if root.is_file() else sorted(root.rglob("*.py"))
    for path in paths:
        if "__pycache__" in path.parts:
            continue
        try:
            tree = ast.parse(path.read_text(), filename=str(path))
        except SyntaxError as exc:
            violations.append(f"{path}:{exc.lineno}:syntax_error:{exc.msg}")
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef) and node.name == "InvocationAuthorityProof":
                for item in node.body:
                    if isinstance(item, ast.AnnAssign) and item.value is not None:
                        field = item.target.id if isinstance(item.target, ast.Name) else "<unknown>"
                        violations.append(
                            f"{path}:{item.lineno}:InvocationAuthorityProof field default:{field}"
                        )
            if not isinstance(node, ast.Call):
                continue
            func = node.func
            name = None
            if isinstance(func, ast.Name):
                name = func.id
            elif isinstance(func, ast.Attribute):
                name = func.attr
            if name == "ReceiptProofFacts" and not node.args and not node.keywords:
                violations.append(f"{path}:{node.lineno}:empty ReceiptProofFacts()")
            if name == "InvocationAuthorityProof":
                keyword_names = {kw.arg for kw in node.keywords if kw.arg is not None}
                if node.args or keyword_names != authority_fields:
                    missing = ",".join(sorted(authority_fields - keyword_names))
                    violations.append(
                        f"{path}:{node.lineno}:incomplete InvocationAuthorityProof({missing})"
                    )

if violations:
    print("\n".join(violations))
    raise SystemExit(1)
PY
  then
    fail "Python SDK/tests/examples still default receipt or authority proof facts"
  fi

  if rg -n 'proofFacts \?\? EMPTY_RECEIPT_PROOF_FACTS|authorityBinding \?\? AuthorityBinding\.self_|readonly proofFacts\?:|proofFacts\?: ReceiptProofFacts|authorityBinding\?: AuthorityBinding' "$node_invocation" \
    --glob '!axiom-authority.test.ts'; then
    fail "Node receipt construction still allows omitted authority or proof facts"
  fi

  if rg -n 'EMPTY_RECEIPT_PROOF_FACTS' "$node_local_runtime"; then
    fail "Node LocalRuntime still emits receipts with empty proof facts"
  fi

  if ((${#node_receipt_paths[@]} > 0)) \
    && rg -n 'EMPTY_RECEIPT_PROOF_FACTS' "${node_receipt_paths[@]}" \
      --glob '!**/node_modules/**'; then
    fail "Node invocation package still exposes or uses empty receipt proof facts"
  fi

  if ((${#node_receipt_paths[@]} > 0)) \
    && rg -n 'EMPTY_AUTHORITY_PROOF' "${node_receipt_paths[@]}" \
      --glob '!**/node_modules/**'; then
    fail "Node invocation package still exposes or uses empty authority proof facts"
  fi

  if ((${#swift_receipt_paths[@]} > 0)) \
    && rg -n 'authorityBinding: AuthorityBinding\? = nil|proofFacts: ReceiptProofFacts = \.empty|\?\? \.selfAuthority|public static let empty\s*=\s*ReceiptProofFacts|ReceiptProofFacts\.empty|proofFacts:\s*\.empty|try\s+ReceiptProofFacts\(\s*\)' "${swift_receipt_paths[@]}" \
      --glob '!**/.build/**'; then
    fail "Swift receipt construction still defaults authority or proof facts"
  fi

  if ((${#swift_receipt_paths[@]} > 0)) \
    && rg -n 'public static let empty\s*=\s*InvocationAuthorityProof|InvocationAuthorityProof\.empty|authorityProof:\s*\.empty|try\s+InvocationAuthorityProof\(\s*\)|proofType:\s*String\s*=|binding:\s*AuthorityBinding\?\s*=\s*nil|proofPayload:\s*Data\s*=|proofHash:\s*Data\s*=|signature:\s*CalleeSignature\?\s*=\s*nil|admissionHook:\s*String\s*=' "${swift_receipt_paths[@]}" \
      --glob '!**/.build/**'; then
    fail "Swift authority proof construction still defaults authority proof facts"
  fi

  if rg -n 'normaliseAuthority\(r\.AuthorityBinding|ProofFacts:\s*ReceiptProofFacts\{|return ReceiptProofFacts\{' "$go_invocation" \
    --glob '!axiom.go'; then
    fail "Go receipt construction still omits constructor-backed proof facts"
  fi

  if rg -n 'EmptyReceiptProofFacts\(\)' "$go_local_runtime"; then
    fail "Go LocalRuntime still emits receipts with empty proof facts"
  fi

  if rg -n 'EmptyReceiptProofFacts\(\)' "$go_invocation"; then
    fail "Go invocation package still exposes or uses empty receipt proof facts"
  fi

  if rg -n 'InvocationAuthorityProof\{\}' "$go_invocation" --glob '!bundle.go'; then
    fail "Go invocation package still embeds zero-value authority proof facts"
  fi

  if [[ -d "$rust_invocation" ]] \
    && rg -n 'ReceiptProofFacts::default\(\)|proof_facts:\s*Default::default\(\)|ReceiptProofFacts\s*\{[^}]*\.\.Default::default\(\)' "$rust_invocation" -U; then
    fail "Rust invocation package still constructs default receipt proof facts"
  fi

  if [[ -d "$rust_invocation" ]] \
    && rg -n '(^|[^:])InvocationAuthorityProof::default\(\)|\.\.InvocationAuthorityProof::default\(\)|InvocationAuthorityProof\s*\{[^}]*\.\.Default::default\(\)' "$rust_invocation" -U; then
    fail "Rust invocation package still constructs default authority proof facts"
  fi

  if [[ -f "$rust_axiom" ]] \
    && ! "$PYTHON_BIN" - "$rust_axiom" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text()
if re.search(r"#\[derive\([^\]]*\bDefault\b[^\]]*\)\]\s*pub struct ReceiptProofFacts\b", text, re.S):
    print(f"{sys.argv[1]}: ReceiptProofFacts derives Default")
    raise SystemExit(1)
if re.search(r"#\[derive\([^\]]*\bDefault\b[^\]]*\)\]\s*pub struct InvocationAuthorityProof\b", text, re.S):
    print(f"{sys.argv[1]}: InvocationAuthorityProof derives Default")
    raise SystemExit(1)
PY
  then
    fail "Rust receipt or authority proof facts still expose a default constructor"
  fi

  if [[ -f "$runtime_client_admission" ]] \
    && rg -n '#\[derive\([^\]]*\bDefault\b[^\]]*\)\]\s*pub struct ReceiptProofFacts\b|authority_proof:\s*Option<|InvocationAuthorityProof::default\(\)' "$runtime_client_admission" -U; then
    fail "Rust runtime client transport adapter still defaults or omits receipt authority proof facts"
  fi
}

check_java_sdk_runtime_receipt_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local result="$cli_root/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  local receipt="$cli_root/sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java"
  local tests="$cli_root/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
  [[ -f "$result" ]] || fail "Java InvocationResult source is missing: ${result#$cli_root/}"
  [[ -f "$receipt" ]] || fail "Java RuntimeReceipt source is missing: ${receipt#$cli_root/}"
  [[ -f "$tests" ]] || fail "Java runtime seam tests are missing: ${tests#$cli_root/}"

  "$PYTHON_BIN" - "$result" "$receipt" "$tests" <<'PY'
import sys
from pathlib import Path

result_path, receipt_path, tests_path = map(Path, sys.argv[1:])
result = result_path.read_text(encoding="utf-8")
receipt = receipt_path.read_text(encoding="utf-8")
tests = tests_path.read_text(encoding="utf-8")

if "public final class RuntimeReceipt" not in receipt:
    raise SystemExit("java_runtime_receipt_projection:runtime_receipt_type_missing")
for fragment, label in {
    "validateProofFacts": "proof_fact_validator_missing",
    '"authority_proof"': "authority_proof_required_missing",
    '"parent_receipts"': "parent_receipts_required_missing",
    "receiptHash(": "hash_validator_missing",
    "base64Bytes(": "base64_validator_missing",
    "canonicalLifecycleState": "lifecycle_state_machine_missing",
}.items():
    if fragment not in receipt:
        raise SystemExit(f"java_runtime_receipt_projection:{label}")

if "RuntimeReceipt.fromMap(terminalReceipt)" not in result:
    raise SystemExit("java_runtime_receipt_projection:invocation_result_not_using_receipt_validator")
if "requiredTerminalReceipt(fields)" not in result:
    raise SystemExit("java_runtime_receipt_projection:terminal_receipt_not_required")
if "terminal_receipt is required" not in result:
    raise SystemExit("java_runtime_receipt_projection:terminal_receipt_required_error_missing")
if "terminal_receipt state does not match invocation terminal_state" not in result:
    raise SystemExit("java_runtime_receipt_projection:terminal_state_topology_missing")
if 'fields.containsKey("receipt")' not in result or "retired receipt alias is not accepted" not in result:
    raise SystemExit("java_runtime_receipt_projection:retired_receipt_alias_not_rejected")
legacy_patterns = {
    "terminalReceiptValue instanceof Map<?, ?> map ? copyStringMap(map) : Map.of()": "malformed_terminal_receipt_downgrade",
    "optionalReceipt(fields, \"terminal_receipt\")": "optional_terminal_receipt_decoder",
    "terminalReceipt = terminalReceipt == null ? Map.of() : Map.copyOf(terminalReceipt);\n    if (ok": "constructor_without_receipt_validation",
}
for pattern, label in legacy_patterns.items():
    if pattern in result:
        raise SystemExit(f"java_runtime_receipt_projection:{label}")

for required_test in (
    "runtimeReceiptProofFactsAreMandatory",
    "canonicalRuntimeReceiptFixture",
    "missingProof.remove(\"authority_proof\")",
    "terminal_receipt is required",
    "retired receipt alias",
):
    if required_test not in tests:
        raise SystemExit(f"java_runtime_receipt_projection:missing_test:{required_test}")
PY
}

check_node_sdk_runtime_receipt_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local runtime="$cli_root/sdk/node/index.js"
  local types="$cli_root/sdk/node/index.d.ts"
  local tests="$cli_root/sdk/node/test/runtime-core.test.mjs"
  local conformance="$cli_root/sdk/node/test/conformance-cases.test.mjs"
  [[ -f "$runtime" ]] || fail "Node runtime source is missing: ${runtime#$cli_root/}"
  [[ -f "$types" ]] || fail "Node runtime declarations are missing: ${types#$cli_root/}"
  [[ -f "$tests" ]] || fail "Node runtime tests are missing: ${tests#$cli_root/}"
  [[ -f "$conformance" ]] || fail "Node conformance tests are missing: ${conformance#$cli_root/}"

  "$PYTHON_BIN" - "$runtime" "$types" "$tests" "$conformance" <<'PY'
import sys
from pathlib import Path

runtime_path, types_path, tests_path, conformance_path = map(Path, sys.argv[1:])
runtime = runtime_path.read_text(encoding="utf-8")
types = types_path.read_text(encoding="utf-8")
tests = tests_path.read_text(encoding="utf-8")
conformance = conformance_path.read_text(encoding="utf-8")
test_corpus = tests + "\n" + conformance

if "export class RuntimeReceipt" not in runtime:
    raise SystemExit("node_runtime_receipt_projection:runtime_receipt_type_missing")
if "export class RuntimeReceipt" not in types:
    raise SystemExit("node_runtime_receipt_projection:runtime_receipt_declaration_missing")
for fragment, label in {
    "validateRuntimeReceiptProofFacts": "proof_fact_validator_missing",
    "raw.authority_proof": "authority_proof_required_missing",
    "requireRuntimeReceiptParents(raw.parent_receipts)": "parent_receipts_required_missing",
    "canonicalRuntimeReceiptState": "lifecycle_state_machine_missing",
    "canonicalRuntimeReceiptType": "receipt_type_state_binding_missing",
    "runtimeReceiptHash": "hash_validator_missing",
    "validateRuntimeBase64": "base64_validator_missing",
}.items():
    if fragment not in runtime:
        raise SystemExit(f"node_runtime_receipt_projection:{label}")

if "RuntimeReceipt.fromObject(objectValue(value, \"terminal_receipt\"))" not in runtime:
    raise SystemExit("node_runtime_receipt_projection:invocation_result_not_using_receipt_validator")
if 'Object.hasOwn(decoded, "receipt")' not in runtime or "retired receipt alias is not accepted" not in runtime:
    raise SystemExit("node_runtime_receipt_projection:retired_receipt_alias_not_rejected")
for forbidden, label in {
    "delete result.receipt": "retired_receipt_alias_delete",
    "terminal_receipt: { receipt_ref": "opaque_terminal_receipt_fixture",
    "receipt_ref:": "opaque_receipt_ref_fixture",
}.items():
    if forbidden in runtime or forbidden in test_corpus:
        raise SystemExit(f"node_runtime_receipt_projection:{label}")

for required_test in (
    "runtime receipt proof facts are mandatory",
    "canonicalRuntimeReceipt",
    "delete missingProof.authority_proof",
    "receipt_type",
    "retired receipt alias is not accepted",
):
    if required_test not in test_corpus:
        raise SystemExit(f"node_runtime_receipt_projection:missing_test:{required_test}")
PY
}

check_swift_sdk_runtime_receipt_projection_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local runtime="$cli_root/sdk/swift/Sources/RuntimeSDK/Runtime.swift"
  local tests="$cli_root/sdk/swift/Tests/RuntimeSDKTests/RuntimeCoreSeamTests.swift"
  [[ -f "$runtime" ]] || fail "Swift Runtime source is missing: ${runtime#$cli_root/}"
  [[ -f "$tests" ]] || fail "Swift runtime tests are missing: ${tests#$cli_root/}"

  "$PYTHON_BIN" - "$runtime" "$tests" <<'PY'
import sys
from pathlib import Path

runtime = Path(sys.argv[1]).read_text(encoding="utf-8")
tests = Path(sys.argv[2]).read_text(encoding="utf-8")

if 'object.keys.contains("receipt")' not in runtime or "retired receipt alias is not accepted" not in runtime:
    raise SystemExit("swift_runtime_receipt_projection:retired_receipt_alias_not_rejected")
if "runtimeRequiredTerminalReceipt(object)" not in runtime:
    raise SystemExit("swift_runtime_receipt_projection:terminal_receipt_not_required")
if "terminal_receipt is required" not in runtime:
    raise SystemExit("swift_runtime_receipt_projection:terminal_receipt_required_error_missing")
if "terminal_receipt must be an object" not in runtime:
    raise SystemExit("swift_runtime_receipt_projection:terminal_receipt_type_error_missing")
for retired, label in {
    "runtimeStringMap(object[\"terminal_receipt\"])": "optional_terminal_receipt_decoder",
    "private func runtimeStringMap(_ value: Any?) -> [String: String]": "malformed_terminal_receipt_downgrade",
    "private func runtimeRequiredStringMap(_ object: [String: Any], _ field: String, _ label: String)": "generic_terminal_receipt_projection",
    "XCTAssertTrue(legacyOnly.terminalReceipt.isEmpty)": "retired_alias_empty_projection_test",
}.items():
    if retired in runtime or retired in tests:
        raise SystemExit(f"swift_runtime_receipt_projection:{label}")
for required_test in (
    "testInvocationResultUsesTerminalReceipt",
    '"receipt":{"receipt_ref":"legacy-only"}',
    '"terminal_state":"Completed"}',
    '"terminal_receipt":"bad"',
):
    if required_test not in tests:
        raise SystemExit(f"swift_runtime_receipt_projection:missing_test:{required_test}")
PY
}

check_sdk_runtime_receipt_type_state_binding_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local go_runtime="$cli_root/sdk/go/runtime.go"
  local go_tests="$cli_root/sdk/go/runtime_test.go"
  local py_runtime="$cli_root/sdk/python/easynet_sdk/runtime.py"
  local py_tests="$cli_root/sdk/python/tests/test_runtime.py"
  local java_receipt="$cli_root/sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java"
  local java_tests="$cli_root/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
  local node_runtime="$cli_root/sdk/node/index.js"
  local node_tests="$cli_root/sdk/node/test/runtime-core.test.mjs"
  [[ -f "$go_runtime" ]] || fail "Go runtime source is missing: ${go_runtime#$cli_root/}"
  [[ -f "$go_tests" ]] || fail "Go runtime tests are missing: ${go_tests#$cli_root/}"
  [[ -f "$py_runtime" ]] || fail "Python runtime source is missing: ${py_runtime#$cli_root/}"
  [[ -f "$py_tests" ]] || fail "Python runtime tests are missing: ${py_tests#$cli_root/}"
  [[ -f "$java_receipt" ]] || fail "Java RuntimeReceipt source is missing: ${java_receipt#$cli_root/}"
  [[ -f "$java_tests" ]] || fail "Java runtime tests are missing: ${java_tests#$cli_root/}"
  [[ -f "$node_runtime" ]] || fail "Node runtime source is missing: ${node_runtime#$cli_root/}"
  [[ -f "$node_tests" ]] || fail "Node runtime tests are missing: ${node_tests#$cli_root/}"

  "$PYTHON_BIN" - "$go_runtime" "$go_tests" "$py_runtime" "$py_tests" "$java_receipt" "$java_tests" "$node_runtime" "$node_tests" <<'PY'
import sys
from pathlib import Path

go_runtime, go_tests, py_runtime, py_tests, java_receipt, java_tests, node_runtime, node_tests = [
    Path(path).read_text(encoding="utf-8") for path in sys.argv[1:]
]

checks = {
    "go": (
        go_runtime,
        go_tests,
        "r.ReceiptType != canonicalReceiptType(state)",
        "runtime receipt receipt_type does not match its lifecycle state",
        '"terminal", "failed", "Completed"',
    ),
    "python": (
        py_runtime,
        py_tests,
        "self.receipt_type != _canonical_receipt_type(lifecycle_state)",
        "runtime receipt receipt_type does not match its lifecycle state",
        '("terminal", "failed", "Completed")',
    ),
    "java": (
        java_receipt,
        java_tests,
        "!receiptType.equals(canonicalReceiptType(lifecycleState))",
        "runtime receipt receipt_type does not match its lifecycle state",
        'mismatchedType.put("receipt_type", "terminal")',
    ),
    "node": (
        node_runtime,
        node_tests,
        "this.receiptType !== canonicalRuntimeReceiptType(lifecycleState)",
        "runtime receipt receipt_type does not match its lifecycle state",
        'state: "Failed"',
    ),
}

for language, (runtime, tests, binding_fragment, error_fragment, test_fragment) in checks.items():
    if binding_fragment not in runtime:
        raise SystemExit(f"sdk_runtime_receipt_type_state_binding:{language}:binding_missing")
    if error_fragment not in runtime:
        raise SystemExit(f"sdk_runtime_receipt_type_state_binding:{language}:error_missing")
    if test_fragment not in tests or "receipt_type" not in tests:
        raise SystemExit(f"sdk_runtime_receipt_type_state_binding:{language}:negative_test_missing")

for corpus, language in ((go_tests, "go"), (py_tests, "python")):
    if '"terminal", "Completed"' in corpus or '"terminal", "completed"' in corpus:
        raise SystemExit(f"sdk_runtime_receipt_type_state_binding:{language}:legacy_terminal_fixture")
PY
}

if [[ "${1:-}" == "--ura-only" ]]; then
  check_ura_vocabulary_contract
  check_axon_protocol_pack_ura_vector_contract
  check_axon_normative_ura_document_contract
  check_axon_proto_ura_vocabulary_contract
  check_axon_sdk_product_neutral_ura_error_contract
  check_axon_active_ura_source_test_contract
  check_active_ura_transport_classification_contract "$ROOT/src" "$ROOT/tests" "$ROOT/include"
  echo "canonical-runtime-convergence-v2 URA gate ok"
  exit 0
fi

if [[ "${1:-}" == "--self-test-ura" ]]; then
  tmp="$(mktemp -d "$ROOT/target/canonical-runtime-convergence-v2-ura.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  run_ura_vocabulary_self_test "$tmp"
  echo "canonical-runtime-convergence-v2 URA self-test ok"
  exit 0
fi

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$ROOT/target/canonical-runtime-convergence-v2.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  "$PYTHON_BIN" "$EDGE_ADAPTER_POLICY" --self-test >/dev/null
  cp "$MANIFEST" "$tmp/manifest.json"
  cp "$MATRIX" "$tmp/matrix.json"
  cp "$MATRIX" "$tmp/lifecycle-reference-drift.json"
  "$PYTHON_BIN" - "$tmp/lifecycle-reference-drift.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["canonical_lifecycle_contract"]["transition_vectors"]["sha256"] = "0" * 64
path.write_text(json.dumps(data))
PY
  if ( MATRIX="$tmp/lifecycle-reference-drift.json"; check_manifest_contract ) >/dev/null 2>&1; then
    fail "self-test expected canonical lifecycle reference drift gate to fail"
  fi
  cp "$MATRIX" "$tmp/duplicate-lifecycle-contract.json"
  "$PYTHON_BIN" - "$tmp/duplicate-lifecycle-contract.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["lifecycle_transition_contract"] = {}
path.write_text(json.dumps(data))
PY
  if ( MATRIX="$tmp/duplicate-lifecycle-contract.json"; check_manifest_contract ) >/dev/null 2>&1; then
    fail "self-test expected duplicate lifecycle contract gate to fail"
  fi
  cp "$MATRIX" "$tmp/duplicate-lifecycle-cell.json"
  "$PYTHON_BIN" - "$tmp/duplicate-lifecycle-cell.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["cells"][0]["lifecycle_vector_actions"] = []
path.write_text(json.dumps(data))
PY
  if ( MATRIX="$tmp/duplicate-lifecycle-cell.json"; check_manifest_contract ) >/dev/null 2>&1; then
    fail "self-test expected duplicate lifecycle cell claim gate to fail"
  fi
  "$PYTHON_BIN" - "$tmp/manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["languages"]["rust"].append("canonical_invocation_bytes")
data["languages"]["rust"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
plain = {"canonical_invocation_bytes"}
if plain & set(manifest["languages"]["rust"]):
    raise SystemExit("canonical_plain_helper_leak")
PY
  then
    fail "self-test expected canonical helper leak to fail"
  fi
  cp "$MANIFEST" "$tmp/plain-legacy-manifest.json"
  "$PYTHON_BIN" - "$tmp/plain-legacy-manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["non_canonical"]["members"]["rust"].append("axiom.sign_invocation")
data["non_canonical"]["members"]["rust"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/plain-legacy-manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
plain = {"axiom.sign_invocation"}
if plain & set(manifest["non_canonical"]["members"]["rust"]):
    raise SystemExit("plain_helper_legacy_export")
PY
  then
    fail "self-test expected legacy plain helper export to fail"
  fi
  cp "$MANIFEST" "$tmp/fallback-manifest.json"
  "$PYTHON_BIN" - "$tmp/fallback-manifest.json" <<'PY'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
data = json.loads(path.read_text())
data["languages"]["go"].append("GeneratedSubjectAuth")
data["languages"]["go"].sort()
path.write_text(json.dumps(data))
PY
  if "$PYTHON_BIN" - "$tmp/fallback-manifest.json" "$tmp/matrix.json" <<'PY' >/dev/null 2>&1
import json
import sys
from pathlib import Path
manifest = json.loads(Path(sys.argv[1]).read_text())
fallback = {"GeneratedSubjectAuth"}
if fallback & set(manifest["languages"]["go"]):
    raise SystemExit("fallback_signer_helper_leak")
PY
  then
    fail "self-test expected fallback signer leak to fail"
  fi
  mkdir -p "$tmp/axon/sdk/node/src/invocation"
  mkdir -p "$tmp/axon/sdk/java/src/main/java/run/axon/sdk/invocation"
  mkdir -p "$tmp/axon/sdk/python/axon_sdk/invocation"
  mkdir -p "$tmp/axon/sdk/swift/Sources/AxonSDK/Invocation"
  mkdir -p "$tmp/axon/sdk/go/axon/invocation"
  mkdir -p "$tmp/axon/core/proto/axon/v1"
  mkdir -p "$tmp/axon/core/runtime-rs/client-sdk/proto/axon/v1"
  mkdir -p "$tmp/axon/sdk/rust/proto/axon/v1"
  mkdir -p "$tmp/axon/sdk/rust/src"
  mkdir -p "$tmp/axon/sdk/rust/src/invocation/local_runtime"
  mkdir -p "$tmp/axon/sdk/go/axon"
  mkdir -p "$tmp/axon/sdk/python/axon_sdk"
  mkdir -p "$tmp/axon/core/runtime-rs" "$tmp/axon/core/runtime-rs/client-sdk/src/domain"
  printf '[package]\nname = "axon-rust-test"\nversion = "0.0.0"\n\n[features]\n' \
    > "$tmp/axon/sdk/rust/Cargo.toml"
  printf 'pub mod invocation;\n' > "$tmp/axon/sdk/rust/src/lib.rs"
  touch "$tmp/axon/sdk/rust/src/invocation/mod.rs"
  touch "$tmp/axon/sdk/rust/src/invocation/axiom.rs"
  touch "$tmp/axon/sdk/rust/src/invocation/local_runtime/mod.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/core/runtime-rs/build.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/core/runtime-rs/client-sdk/build.rs"
  printf 'const CANONICAL_AXON_PROTO_FILES: &[&str] = &[];\n' > "$tmp/axon/sdk/rust/build.rs"
  mkdir -p "$tmp/axon/document/rfcs" "$tmp/axon/sdk"
  printf 'Withdrawn from Axon canonical protocol\n' > "$tmp/axon/document/rfcs/004-mcp-binding.md"
  printf '## Product Boundary\n' > "$tmp/axon/sdk/SDK_PARITY.md"
  touch "$tmp/axon/sdk/java/src/main/java/run/axon/sdk/invocation/Axiom.java"
  touch "$tmp/axon/sdk/java/src/main/java/run/axon/sdk/invocation/Bundle.java"
  touch "$tmp/axon/sdk/java/src/main/java/run/axon/sdk/invocation/LocalRuntime.java"
  touch "$tmp/axon/sdk/python/axon_sdk/invocation/axiom.py"
  touch "$tmp/axon/sdk/python/axon_sdk/invocation/audit.py"
  touch "$tmp/axon/sdk/python/axon_sdk/invocation/local_runtime.py"
  touch "$tmp/axon/sdk/node/src/invocation/local-runtime.ts"
  touch "$tmp/axon/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  touch "$tmp/axon/sdk/go/axon/invocation/axiom.go"
  touch "$tmp/axon/sdk/go/axon/invocation/local_runtime.go"
  printf 'export interface ReceiptBody { readonly proofFacts?: ReceiptProofFacts; }\n' \
    > "$tmp/axon/sdk/node/src/invocation/axiom.d.ts"
  if ! rg -n 'proofFacts\?: ReceiptProofFacts' "$tmp/axon/sdk/node/src/invocation" >/dev/null; then
    fail "self-test expected receipt proof-fact default gate to fail"
  fi
  printf '' > "$tmp/axon/sdk/node/src/invocation/axiom.d.ts"
  cp -R "$tmp/axon" "$tmp/axon-receipt-runtime"
  printf 'class LocalRuntime { void emit() { Axiom.ReceiptProofFacts.empty(); } }\n' \
    > "$tmp/axon-receipt-runtime/sdk/java/src/main/java/run/axon/sdk/invocation/LocalRuntime.java"
  if ( AXON_ROOT="$tmp/axon-receipt-runtime"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Java LocalRuntime empty proof facts gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-java-authority-helper"
  printf 'class Axiom { static class InvocationAuthorityProof { static InvocationAuthorityProof empty() { return null; } } }\n' \
    > "$tmp/axon-java-authority-helper/sdk/java/src/main/java/run/axon/sdk/invocation/Axiom.java"
  if ( AXON_ROOT="$tmp/axon-java-authority-helper"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Java empty authority proof helper gate to fail"
  fi
  mkdir -p "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk" \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/test/java/run/runtime/sdk"
  cp "$ROOT/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java" \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  cp "$ROOT/sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java" \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java"
  cp "$ROOT/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java" \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
  perl -0pi -e 's/requiredTerminalReceipt\(fields\)/optionalReceipt(fields, "terminal_receipt")/' \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  perl -0pi -e 's/private static Map<String, Object> requiredTerminalReceipt\(Map<String, Object> fields\)/private static Map<String, Object> optionalReceipt(Map<String, Object> fields, String field)/' \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  perl -0pi -e 's/throw SDKError\.validation\("invocation_result", "terminal_receipt is required"\);/return Map.of();/' \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  perl -0pi -e 's/fields\.containsKey\("terminal_receipt"\)/fields.containsKey(field)/g; s/fields\.get\("terminal_receipt"\)/fields.get(field)/g; s/"terminal_receipt must be an object"/field + " must be an object"/g' \
    "$tmp/cli-java-receipt-legacy/sdk/java/src/main/java/run/runtime/sdk/InvocationResult.java"
  if ( CLI_ROOT="$tmp/cli-java-receipt-legacy"; check_java_sdk_runtime_receipt_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected Java SDK receipt projection bypass gate to fail"
  fi
  mkdir -p "$tmp/cli-node-receipt-legacy/sdk/node/test"
  cat >"$tmp/cli-node-receipt-legacy/sdk/node/index.js" <<'EOF'
function invocationResultFromJSON(raw) {
  const decoded = JSON.parse(raw);
  const result = { ...decoded };
  delete result.receipt;
  if (decoded.terminal_receipt !== undefined && decoded.terminal_receipt !== null) {
    result.terminalReceipt = decoded.terminal_receipt;
  }
  return result;
}
EOF
  printf 'export class RuntimeClient {}\n' > "$tmp/cli-node-receipt-legacy/sdk/node/index.d.ts"
  printf 'test("invocation result receipt", () => ({terminal_receipt: { receipt_ref: "opaque" }}));\n' \
    > "$tmp/cli-node-receipt-legacy/sdk/node/test/runtime-core.test.mjs"
  printf 'test("conformance terminal receipt facts are explicit", () => ({receipt_ref: "opaque"}));\n' \
    > "$tmp/cli-node-receipt-legacy/sdk/node/test/conformance-cases.test.mjs"
  if ( CLI_ROOT="$tmp/cli-node-receipt-legacy"; check_node_sdk_runtime_receipt_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected Node SDK receipt projection bypass gate to fail"
  fi
  mkdir -p "$tmp/cli-ffi-json-legacy/src/ffi/invocation"
  cat >"$tmp/cli-ffi-json-legacy/src/ffi/invocation/mod.rs" <<'EOF'
fn invocation_outcome_json_with_tuple() -> serde_json::Value {
    let output_json = if result_content_type_is_json(&result.output_content_type) {
        serde_json::from_slice::<serde_json::Value>(&result.output).ok()
    } else {
        None
    };
    serde_json::json!({"output_json": output_json})
}

fn stream_chunk_json() -> Result<serde_json::Value, String> {
    let payload_json = if chunk.content_type == "application/json" {
        serde_json::from_slice::<serde_json::Value>(&chunk.payload).ok()
    } else {
        None
    };
    Ok(serde_json::json!({"payload_json": payload_json}))
}
EOF
  if ( CLI_ROOT="$tmp/cli-ffi-json-legacy"; check_ffi_invocation_json_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI JSON projection downgrade gate to fail"
  fi
  mkdir -p "$tmp/cli-ffi-last-error-legacy/src/ffi/errors"
  cat >"$tmp/cli-ffi-last-error-legacy/src/ffi/errors/mod.rs" <<'EOF'
struct LastErrorRecord {
    message: std::ffi::CString,
    code: Option<i32>,
}

pub(crate) fn set_last_error(msg: impl Into<String>) {
    set_last_error_record(None, msg);
}

fn set_last_error_record(code: Option<i32>, msg: impl Into<String>) {
    let _ = code.unwrap_or(ERR_GENERIC);
    let _ = msg.into();
}

fn typed_error_json(code: Option<i32>, message: &str) -> serde_json::Value {
    serde_json::json!({
        "message": message,
        "details": {"abi_code": code.unwrap_or(ERR_GENERIC)}
    })
}

fn runtime_error_json() {
    let message = last_error_message().unwrap_or_default();
}

fn last_error_json_projects_legacy_message_as_generic() {}
EOF
  if ( CLI_ROOT="$tmp/cli-ffi-last-error-legacy"; check_ffi_last_error_typed_tls_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI last-error typed TLS gate to fail"
  fi
  mkdir -p "$tmp/cli-sdk-receipt-type-legacy/sdk/go" \
    "$tmp/cli-sdk-receipt-type-legacy/sdk/python/easynet_sdk" \
    "$tmp/cli-sdk-receipt-type-legacy/sdk/python/tests" \
    "$tmp/cli-sdk-receipt-type-legacy/sdk/java/src/main/java/run/runtime/sdk" \
    "$tmp/cli-sdk-receipt-type-legacy/sdk/java/src/test/java/run/runtime/sdk" \
    "$tmp/cli-sdk-receipt-type-legacy/sdk/node/test"
  cat >"$tmp/cli-sdk-receipt-type-legacy/sdk/go/runtime.go" <<'EOF'
func (r RuntimeReceipt) ValidateSummary() error {
  _, err := r.LifecycleState()
  return err
}
EOF
  printf 'func TestRuntimeReceipt(t *testing.T) { canonicalRuntimeReceiptFixture("inv", "terminal", "Completed", 1) }\n' \
    > "$tmp/cli-sdk-receipt-type-legacy/sdk/go/runtime_test.go"
  cat >"$tmp/cli-sdk-receipt-type-legacy/sdk/python/easynet_sdk/runtime.py" <<'EOF'
def validate_summary(self):
    self.lifecycle_state
EOF
  printf 'def test_runtime_receipt():\n    canonical_runtime_receipt("inv", "terminal", "Completed", 1)\n' \
    > "$tmp/cli-sdk-receipt-type-legacy/sdk/python/tests/test_runtime.py"
  cat >"$tmp/cli-sdk-receipt-type-legacy/sdk/java/src/main/java/run/runtime/sdk/RuntimeReceipt.java" <<'EOF'
public final class RuntimeReceipt {
  private void validateSummary() { canonicalLifecycleState(state); }
}
EOF
  printf 'class RuntimeCoreSeamTest { void test() { canonicalRuntimeReceiptFixture("inv", "completed", "Completed", 1); } }\n' \
    > "$tmp/cli-sdk-receipt-type-legacy/sdk/java/src/test/java/run/runtime/sdk/RuntimeCoreSeamTest.java"
  cat >"$tmp/cli-sdk-receipt-type-legacy/sdk/node/index.js" <<'EOF'
export class RuntimeReceipt {
  validateSummary() { canonicalRuntimeReceiptState(this.state); }
}
EOF
  printf 'test("runtime receipt proof facts are mandatory", () => ({ receipt_type: "completed" }));\n' \
    > "$tmp/cli-sdk-receipt-type-legacy/sdk/node/test/runtime-core.test.mjs"
  if ( CLI_ROOT="$tmp/cli-sdk-receipt-type-legacy"; check_sdk_runtime_receipt_type_state_binding_contract ) >/dev/null 2>&1; then
    fail "self-test expected SDK runtime receipt type/state binding gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-python-receipt-runtime"
  printf 'binding = AxiomBinding(proof_facts=ReceiptProofFacts())\n' \
    > "$tmp/axon-python-receipt-runtime/sdk/python/axon_sdk/invocation/local_runtime.py"
  if ( AXON_ROOT="$tmp/axon-python-receipt-runtime"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Python LocalRuntime empty proof facts gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-python-authority-default-class"
  printf 'class InvocationAuthorityProof:\n    proof_type: str = ""\n    proof_hash: bytes = b"0" * 32\n' \
    > "$tmp/axon-python-authority-default-class/sdk/python/axon_sdk/invocation/axiom.py"
  if ( AXON_ROOT="$tmp/axon-python-authority-default-class"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Python authority proof dataclass default gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-python-authority-partial-call"
  printf 'proof = InvocationAuthorityProof(proof_hash=b"0" * 32)\n' \
    > "$tmp/axon-python-authority-partial-call/sdk/python/axon_sdk/invocation/partial_authority.py"
  if ( AXON_ROOT="$tmp/axon-python-authority-partial-call"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Python partial authority proof call gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-receipt-runtime"
  printf 'const binding = { proofFacts: EMPTY_RECEIPT_PROOF_FACTS };\n' \
    > "$tmp/axon-node-receipt-runtime/sdk/node/src/invocation/local-runtime.ts"
  if ( AXON_ROOT="$tmp/axon-node-receipt-runtime"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Node LocalRuntime empty proof facts gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-receipt-helper"
  printf 'export const EMPTY_RECEIPT_PROOF_FACTS = Object.freeze({});\n' \
    > "$tmp/axon-node-receipt-helper/sdk/node/src/invocation/axiom.ts"
  if ( AXON_ROOT="$tmp/axon-node-receipt-helper"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Node empty proof facts helper gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-authority-helper"
  printf 'export const EMPTY_AUTHORITY_PROOF = Object.freeze({});\n' \
    > "$tmp/axon-node-authority-helper/sdk/node/src/invocation/axiom.ts"
  if ( AXON_ROOT="$tmp/axon-node-authority-helper"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Node empty authority proof helper gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-receipt-runtime"
  printf 'binding := AxiomBinding{ProofFacts: EmptyReceiptProofFacts()}\n' \
    > "$tmp/axon-go-receipt-runtime/sdk/go/axon/invocation/local_runtime.go"
  if ( AXON_ROOT="$tmp/axon-go-receipt-runtime"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Go LocalRuntime empty proof facts gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-receipt-helper"
  printf 'func EmptyReceiptProofFacts() ReceiptProofFacts { return ReceiptProofFacts{} }\n' \
    > "$tmp/axon-go-receipt-helper/sdk/go/axon/invocation/axiom.go"
  if ( AXON_ROOT="$tmp/axon-go-receipt-helper"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Go empty proof facts helper gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-authority-zero"
  printf 'func f() { _ = InvocationAuthorityProof{} }\n' \
    > "$tmp/axon-go-authority-zero/sdk/go/axon/invocation/authority_anchor_test.go"
  if ( AXON_ROOT="$tmp/axon-go-authority-zero"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Go zero authority proof gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-receipt-empty"
  printf 'public struct ReceiptProofFacts { public static let empty = ReceiptProofFacts() }\nlet binding = AxiomBinding(proofFacts: .empty)\n' \
    > "$tmp/axon-swift-receipt-empty/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-receipt-empty"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Swift empty proof facts helper gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-authority-empty"
  printf 'public struct InvocationAuthorityProof { public static let empty = InvocationAuthorityProof() }\nlet facts = ReceiptProofFacts(authorityProof: .empty)\n' \
    > "$tmp/axon-swift-authority-empty/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-authority-empty"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Swift empty authority proof gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-authority-default-init"
  printf 'public init(proofType: String = "", binding: AuthorityBinding? = nil, proofPayload: Data = Data(), proofHash: Data = Data(repeating: 0, count: 32), signature: CalleeSignature? = nil, admissionHook: String = "") {}\n' \
    > "$tmp/axon-swift-authority-default-init/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-authority-default-init"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Swift authority proof default initializer gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-receipt-default-init"
  printf 'let facts = try ReceiptProofFacts()\n' \
    > "$tmp/axon-swift-receipt-default-init/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-receipt-default-init"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Swift empty proof facts constructor gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-receipt-default-call"
  printf 'fn f() { let facts = ReceiptProofFacts::default(); let body = ReceiptBody { proof_facts: Default::default() }; }\n' \
    > "$tmp/axon-rust-receipt-default-call/sdk/rust/src/invocation/handle.rs"
  if ( AXON_ROOT="$tmp/axon-rust-receipt-default-call"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Rust default receipt proof facts call gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-receipt-default-derive"
  printf '#[derive(Debug, Clone, PartialEq, Eq, Default)]\npub struct ReceiptProofFacts {}\n' \
    > "$tmp/axon-rust-receipt-default-derive/sdk/rust/src/invocation/axiom.rs"
  if ( AXON_ROOT="$tmp/axon-rust-receipt-default-derive"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Rust ReceiptProofFacts Default derive gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-authority-default-call"
  printf 'fn f() { let proof = InvocationAuthorityProof::default(); let proof2 = InvocationAuthorityProof { ..Default::default() }; }\n' \
    > "$tmp/axon-rust-authority-default-call/sdk/rust/src/invocation/axiom.rs"
  if ( AXON_ROOT="$tmp/axon-rust-authority-default-call"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Rust default authority proof call gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-authority-default-derive"
  printf '#[derive(Debug, Clone, PartialEq, Eq, Default)]\npub struct InvocationAuthorityProof {}\n' \
    > "$tmp/axon-rust-authority-default-derive/sdk/rust/src/invocation/axiom.rs"
  if ( AXON_ROOT="$tmp/axon-rust-authority-default-derive"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Rust InvocationAuthorityProof Default derive gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-runtime-client-receipt-default"
  printf '#[derive(Debug, Clone, Default)]\npub struct ReceiptProofFacts { pub authority_proof: Option<pb::InvocationAuthorityProof> }\nfn authority_proof() { let _ = InvocationAuthorityProof::default(); }\n' \
    > "$tmp/axon-runtime-client-receipt-default/core/runtime-rs/client-sdk/src/domain/admission.rs"
  if ( AXON_ROOT="$tmp/axon-runtime-client-receipt-default"; check_receipt_proof_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected Rust runtime client receipt proof default gate to fail"
  fi
  make_schema_fixture() {
    local root="$1"
    local cli_root="$2"
    local checker="$AXON_ROOT/scripts/checks/check_proto_derivation.sh"
    local sync_owner="$AXON_ROOT/scripts/proto/sync_canonical_proto.sh"
    local codegen_provisioner="$AXON_ROOT/scripts/proto/ensure_codegen_toolchain.sh"
    local codegen_lock="$AXON_ROOT/scripts/proto/codegen-requirements.lock"
    if [[ ! -f "$checker" ]]; then
      fail "self-test requires real Axon proto derivation gate: ${checker#$AXON_ROOT/}"
    fi
    if [[ ! -x "$sync_owner" ]]; then
      fail "self-test requires real Axon proto sync owner: ${sync_owner#$AXON_ROOT/}"
    fi
    if [[ ! -x "$codegen_provisioner" || ! -f "$codegen_lock" ]]; then
      fail "self-test requires the locked Axon proto codegen toolchain"
    fi

    mkdir -p "$root/scripts/checks" \
      "$root/scripts/proto" \
      "$root/core/proto/axon/v1" \
      "$root/sdk/rust/proto/axon/v1" \
      "$root/core/runtime-rs/client-sdk/src" \
      "$cli_root/sdk/go/internal/axonpb" \
      "$cli_root/sdk/python/easynet_sdk/_axon_pb/axon/v1"
    cp "$checker" "$root/scripts/checks/check_proto_derivation.sh"
    cp "$sync_owner" "$root/scripts/proto/sync_canonical_proto.sh"
    cp "$codegen_provisioner" "$root/scripts/proto/ensure_codegen_toolchain.sh"
    cp "$codegen_lock" "$root/scripts/proto/codegen-requirements.lock"
    chmod +x \
      "$root/scripts/checks/check_proto_derivation.sh" \
      "$root/scripts/proto/sync_canonical_proto.sh" \
      "$root/scripts/proto/ensure_codegen_toolchain.sh"
    touch \
      "$cli_root/sdk/python/easynet_sdk/_axon_pb/__init__.py" \
      "$cli_root/sdk/python/easynet_sdk/_axon_pb/axon/__init__.py" \
      "$cli_root/sdk/python/easynet_sdk/_axon_pb/axon/v1/__init__.py"

    cp \
      "$AXON_ROOT/core/proto/axon/v1/types.proto" \
      "$AXON_ROOT/core/proto/axon/v1/invoke.proto" \
      "$root/core/proto/axon/v1/"

    EASYNET_CLI_ROOT="$cli_root" \
      AXON_PROTO_DERIVATION_ROOT="$root" \
      bash "$root/scripts/checks/check_proto_derivation.sh" --derive >/dev/null
  }

  make_schema_fixture "$tmp/axon-schema-good" "$tmp/cli-schema-good"
  if ! (
    AXON_ROOT="$tmp/axon-schema-good"
    CLI_ROOT="$tmp/cli-schema-good"
    check_schema_source_derivation_contract
  ) >/dev/null 2>&1; then
    fail "self-test expected schema-source derivation fixture to pass"
  fi
  cp -R "$tmp/axon-schema-good" "$tmp/axon-schema-mirror-bad"
  printf '\n// mirror drift\n' \
    >> "$tmp/axon-schema-mirror-bad/sdk/rust/proto/axon/v1/invoke.proto"
  if (
    AXON_ROOT="$tmp/axon-schema-mirror-bad"
    CLI_ROOT="$tmp/cli-schema-good"
    check_schema_source_derivation_contract
  ) >/dev/null 2>&1; then
    fail "self-test expected schema-source mirror drift gate to fail"
  fi
  cp -R "$tmp/axon-schema-good" "$tmp/axon-schema-third-root-bad"
  mkdir -p "$tmp/axon-schema-third-root-bad/product/proto"
  cp "$tmp/axon-schema-third-root-bad/core/proto/axon/v1/types.proto" \
    "$tmp/axon-schema-third-root-bad/product/proto/product.proto"
  if (
    AXON_ROOT="$tmp/axon-schema-third-root-bad"
    CLI_ROOT="$tmp/cli-schema-good"
    check_schema_source_derivation_contract
  ) >/dev/null 2>&1; then
    fail "self-test expected undeclared third proto root gate to fail"
  fi
  cp -R "$tmp/axon-schema-good" "$tmp/axon-schema-client-proto-bad"
  mkdir -p "$tmp/axon-schema-client-proto-bad/core/runtime-rs/client-sdk/proto/axon/v1"
  cp "$tmp/axon-schema-client-proto-bad/core/proto/axon/v1/types.proto" \
    "$tmp/axon-schema-client-proto-bad/core/runtime-rs/client-sdk/proto/axon/v1/types.proto"
  if (
    AXON_ROOT="$tmp/axon-schema-client-proto-bad"
    CLI_ROOT="$tmp/cli-schema-good"
    check_schema_source_derivation_contract
  ) >/dev/null 2>&1; then
    fail "self-test expected transport-client proto authority gate to fail"
  fi
  cp -R "$tmp/axon-schema-good" "$tmp/axon-schema-reverse-import-bad"
  printf '\nimport "product/voice.proto";\n' \
    >> "$tmp/axon-schema-reverse-import-bad/core/proto/axon/v1/invoke.proto"
  cp "$tmp/axon-schema-reverse-import-bad/core/proto/axon/v1/invoke.proto" \
    "$tmp/axon-schema-reverse-import-bad/sdk/rust/proto/axon/v1/invoke.proto"
  if (
    AXON_ROOT="$tmp/axon-schema-reverse-import-bad"
    CLI_ROOT="$tmp/cli-schema-good"
    check_schema_source_derivation_contract
  ) >/dev/null 2>&1; then
    fail "self-test expected reverse product import gate to fail"
  fi
  make_benchmark_fixture() {
    local root="$1"
    mkdir -p "$root/scripts/checks" "$root/sdk/rust/benches"
    cat > "$root/scripts/checks/check_benchmark_baselines.py" <<'PY'
#!/usr/bin/env python3
import argparse
import json

parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True)
parser.add_argument("--baseline", required=True)
arguments = parser.parse_args()
with open(arguments.baseline, encoding="utf-8") as baseline:
    document = json.load(baseline)
raise SystemExit(0 if document == {"fixture_valid": True} else 1)
PY
    printf '{"fixture_valid":true}\n' \
      > "$root/sdk/rust/benches/baseline-v2.json"
  }

  make_benchmark_fixture "$tmp/axon-benchmark-good"
  if ! ( AXON_ROOT="$tmp/axon-benchmark-good"; check_axon_benchmark_baseline_contract ) >/dev/null 2>&1; then
    fail "self-test expected benchmark baseline coverage fixture to pass"
  fi
  cp -R "$tmp/axon-benchmark-good" "$tmp/axon-benchmark-bad"
  printf '{"fixture_valid":false}\n' \
    > "$tmp/axon-benchmark-bad/sdk/rust/benches/baseline-v2.json"
  if ( AXON_ROOT="$tmp/axon-benchmark-bad"; check_axon_benchmark_baseline_contract ) >/dev/null 2>&1; then
    fail "self-test expected benchmark baseline coverage gate to fail"
  fi
  cp -R "$tmp/axon-benchmark-good" "$tmp/axon-benchmark-missing"
  rm "$tmp/axon-benchmark-missing/sdk/rust/benches/baseline-v2.json"
  if ( AXON_ROOT="$tmp/axon-benchmark-missing"; check_axon_benchmark_baseline_contract ) >/dev/null 2>&1; then
    fail "self-test expected missing benchmark baseline gate to fail"
  fi
  mkdir -p "$tmp/axon-product/sdk/rust/src"
  cp -R "$tmp/axon/core" "$tmp/axon-product/core"
  cp -R "$tmp/axon/document" "$tmp/axon-product/document"
  cp -R "$tmp/axon/sdk/SDK_PARITY.md" "$tmp/axon-product/sdk/SDK_PARITY.md"
  cp "$tmp/axon/sdk/rust/build.rs" "$tmp/axon-product/sdk/rust/build.rs"
  printf 'pub mod audio;\n' > "$tmp/axon-product/sdk/rust/src/lib.rs"
  touch "$tmp/axon-product/sdk/rust/src/audio.rs"
  mkdir -p "$tmp/axon-product/sdk/go/easynet/mcp"
  touch "$tmp/axon-product/sdk/go/easynet/tool_adapter.go"
  mkdir -p "$tmp/axon-product/sdk/python/axon_sdk/presets/remote_control"
  touch "$tmp/axon-product/sdk/python/axon_sdk/audio.py"
  mkdir -p "$tmp/axon-product/sdk/node/src/mcp"
  touch "$tmp/axon-product/sdk/node/src/tool_adapter.ts"
  mkdir -p "$tmp/axon-product/sdk/react/src"
  touch "$tmp/axon-product/sdk/react/src/tool_adapter.ts"
  printf 'export { useAbilityTools } from "./tool_adapter.js";\n' \
    > "$tmp/axon-product/sdk/react/src/index.ts"
  mkdir -p "$tmp/axon-product/sdk/java/src/main/java/run/easynet/axon/mcp"
  touch "$tmp/axon-product/sdk/java/src/main/java/run/easynet/axon/AbilityToolAdapter.java"
  mkdir -p "$tmp/axon-product/sdk/swift/Sources/EasyNetAxon"
  touch "$tmp/axon-product/sdk/swift/Sources/EasyNetAxon/ToolAdapter.swift"
  if ( AXON_ROOT="$tmp/axon-product"; check_axon_product_protocol_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon product protocol boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-plain-proof"
  printf 'pub(crate) fn canonical_invocation_bytes() {}\n' \
    > "$tmp/axon-plain-proof/sdk/rust/src/invocation/axiom.rs"
  printf 'def canonical_invocation_bytes(env):\n  return b""\n' \
    > "$tmp/axon-plain-proof/sdk/python/axon_sdk/invocation/axiom.py"
  if ( AXON_ROOT="$tmp/axon-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-active-plain-proof-doc"
  mkdir -p "$tmp/axon-active-plain-proof-doc/document/rfcs" \
    "$tmp/axon-active-plain-proof-doc/sdk/conformance/cases/axiom"
  printf 'Reuse verify_invocation_signature from sdk/rust.\n' \
    > "$tmp/axon-active-plain-proof-doc/document/rfcs/001-pr2-acceptance-checklist.md"
  printf '{"overview":"step 3 calls verify_signature"}\n' \
    > "$tmp/axon-active-plain-proof-doc/sdk/conformance/cases/axiom/axiom-admission-pipeline.json"
  if ( AXON_ROOT="$tmp/axon-active-plain-proof-doc"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon active plain proof document gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-legacy-plain-proof"
  printf 'pub(crate) fn legacy_plain_invocation_bytes() {}\npub(crate) fn run_legacy_plain_admission() {}\n' \
    > "$tmp/axon-rust-legacy-plain-proof/sdk/rust/src/invocation/axiom.rs"
  if ( AXON_ROOT="$tmp/axon-rust-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Rust legacy plain proof boundary gate to fail"
  fi
  mkdir -p "$tmp/cli-route-selector-legacy/src/daemon/invocation/routing"
  cat >"$tmp/cli-route-selector-legacy/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
fn route_selector_from_query(query_name: &str, ability_name: &str) -> Option<RouteSelector> {
    if ability_name.trim().is_empty() {
        if let Some(selector) = ability_selector_from_descriptor_ref(query_name) {
            return Some(RouteSelector {
                query_name: selector.ability_ura().to_string(),
                owner_ura: selector.owner_ura().to_string(),
                ability_ura: selector.ability_ura().to_string(),
                public_name: selector.public_name().to_string(),
            });
        }
    }
    None
}

fn route_selector_from_descriptor_ref(
    owner_ura: &str,
    descriptor_ref: &str,
) -> Option<RouteSelector> {
    let selector = ability_selector_from_descriptor_ref(descriptor_ref)?;
    if selector.owner_ura() != owner_ura {
        return None;
    }
    None
}

fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Option<crate::core::ura::AbilitySelector> {
    let descriptor_ref =
        axon_sdk::invocation::canonical_ability_descriptor_ref(descriptor_ref).ok()?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .ok()?;
    crate::core::ura::AbilitySelector::parse(&ability_ura).ok()
}

fn selected_execution_for_owner() {}
EOF
  if ( CLI_ROOT="$tmp/cli-route-selector-legacy"; check_route_resolver_descriptor_ref_selector_contract ) >/dev/null 2>&1; then
    fail "self-test expected route descriptor-ref selector fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-namespace-authority-legacy/src/daemon/invocation/routing" \
    "$tmp/cli-namespace-authority-legacy/src/daemon/invocation/dispatch"
  cat >"$tmp/cli-namespace-authority-legacy/src/daemon/invocation/routing/route_resolver.rs" <<'EOF'
use serde_json::{json, Value};

fn authority_for_query(query_name: &str) -> Value {
    let realm = crate::core::ura::parse_ura(query_name)
        .ok()
        .map(|parsed| parsed.realm)
        .filter(|realm| !realm.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    json!({
        "authority_ura": crate::core::ura::hub_ura(&realm),
        "zone_ref": format!("realm:{realm}"),
        "algorithm": "daemon-local",
        "signature": "",
        "issued_unix_ms": 0,
    })
}

fn authority_realm_for_query(query_name: &str) -> Option<String> {
    crate::core::ura::parse_ura(query_name).ok().map(|parsed| parsed.realm)
}

#[cfg(test)]
mod tests {
    #[test]
    fn authority_projection_uses_route_ref_embedded_ability_realm() {}

    #[test]
    fn authority_projection_uses_descriptor_ref_embedded_ability_realm() {}

    #[test]
    fn authority_projection_does_not_default_invalid_query_to_localhost() {}
}
EOF
  cat >"$tmp/cli-namespace-authority-legacy/src/daemon/invocation/dispatch/federation_wrappers.rs" <<'EOF'
fn namespace_resolve_input_failure(query: &serde_json::Value, detail: &str) -> serde_json::Value {
    let query_name = query.get("query_name").and_then(serde_json::Value::as_str).unwrap_or_default();
    let realm = crate::core::ura::parse_ura(query_name)
        .ok()
        .map(|parsed| parsed.realm)
        .filter(|realm| !realm.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    serde_json::json!({
        "authority": {
            "authority_ura": crate::core::ura::hub_ura(&realm),
            "zone_ref": format!("realm:{realm}")
        },
        "negative": {"detail": detail}
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn namespace_resolve_input_failure_does_not_fabricate_localhost_authority() {}
}
EOF
  if ( CLI_ROOT="$tmp/cli-namespace-authority-legacy"; check_namespace_resolver_authority_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected namespace resolver authority projection fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-daemon-route-projection-legacy/src/daemon/invocation/dispatch" \
    "$tmp/cli-daemon-route-projection-legacy/src/daemon/axon_bridge"
  cat >"$tmp/cli-daemon-route-projection-legacy/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'EOF'
pub(crate) fn dispatch_function_name_for_route_table(function_name: &str, envelope: Option<&Envelope>) -> String {
    descriptor_ref_public_name_for_callee(function_name, envelope).unwrap_or_else(|| function_name.to_string())
}

fn descriptor_ref_public_name_for_callee(function_name: &str, envelope: Option<&Envelope>) -> Option<String> {
    let callee_ura = envelope?.callee.as_ref().map(|callee| callee.ura.trim()).filter(|callee| !callee.is_empty())?;
    let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(function_name).ok()?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(&descriptor_ref).ok()?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).ok()?;
    if selector.owner_ura() != callee_ura {
        return None;
    }
    Some(selector.public_name().to_string())
}

fn missing_invocation_attempt_ledger() {}
EOF
  printf '%s\n' \
    'fn route_table_match_projects_descriptor_ref_to_public_name() {}' \
    > "$tmp/cli-daemon-route-projection-legacy/src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"
  printf '%s\n' \
    'pub(crate) fn ability_ura_from_descriptor_ref(descriptor_ref: &str) -> Result<String, AxonError> { Ok(String::new()) }' \
    > "$tmp/cli-daemon-route-projection-legacy/src/daemon/axon_bridge/descriptor_ref.rs"
  if ( CLI_ROOT="$tmp/cli-daemon-route-projection-legacy"; check_daemon_invocation_service_descriptor_ref_route_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon invocation service descriptor projection fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-local-session-descriptor-authority-legacy/src/daemon/invocation/dispatch"
  cat >"$tmp/cli-local-session-descriptor-authority-legacy/src/daemon/invocation/dispatch/local_session_dispatcher.rs" <<'EOF'
fn descriptor_ref_for_version(
    callee_ura: &str,
    ability: &str,
    descriptor_version: &str,
) -> String {
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        descriptor_version,
    )
    .unwrap()
}

fn descriptor_ref_for_call_mode(
    callee_ura: &str,
    ability: &str,
    descriptor_version: &str,
    mode: axon_sdk::invocation::CallMode,
) -> String {
    crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
        callee_ura,
        ability,
        mode,
    )
    .unwrap_or_else(|_| descriptor_ref_for_version(callee_ura, ability, descriptor_version))
}
EOF
  if ( CLI_ROOT="$tmp/cli-local-session-descriptor-authority-legacy"; check_local_session_descriptor_ref_test_authority_contract ) >/dev/null 2>&1; then
    fail "self-test expected local session descriptor-ref synthesis gate to fail"
  fi
  mkdir -p "$tmp/cli-ffi-descriptor-owner-legacy/src/ffi/invocation"
  cat >"$tmp/cli-ffi-descriptor-owner-legacy/src/ffi/invocation/mod.rs" <<'EOF'
fn descriptor_resolution_error_projection(message: &str) -> (i32, ErrorProjection) {
    (
        ERR_NOT_FOUND,
        ErrorProjection {
            code: "DESCRIPTOR_NOT_FOUND",
            stage: "routing",
            retry: "never",
        },
    )
}

/// Allocate a mutable Invocation builder handle.
fn runtime_resolve_descriptor_ref_json(
    session: &crate::ffi::client::handle::ClientSession,
    request_json: &str,
) -> anyhow::Result<serde_json::Value> {
    let request: serde_json::Value = serde_json::from_str(request_json)?;
    let object = request.as_object().unwrap();
    let callee_ura = object.get("callee_ura").and_then(serde_json::Value::as_str).unwrap();
    let ability = object.get("ability").and_then(serde_json::Value::as_str).unwrap();
    let call_mode = object.get("call_mode").and_then(serde_json::Value::as_str).unwrap();
    let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, ability).unwrap();
    let runtime_owner_ura = runtime_owner_ura_from_session(session).ok();
    if let Some(owner_ura) = runtime_owner_ura.as_deref() {
        let catalog = runtime_descriptor_catalog_entries(session, owner_ura);
    }
    let caller_ura = descriptor_ref_request_required_string(object, "caller_ura")?.to_string();
    RemoteSystemInvocationIssuer::root_plan(&target_call, caller_ura, subject, args, timeout)?;
    Ok(serde_json::Value::Null)
}

#[cfg(feature = "axon-pb")]
fn descriptor_ref_request_required_string() {}

#[test]
fn descriptor_resolution_errors_project_canonical_runtime_codes() {}
EOF
  if ( CLI_ROOT="$tmp/cli-ffi-descriptor-owner-legacy"; check_ffi_descriptor_runtime_owner_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI descriptor runtime owner fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-ffi-meta-descriptor-probe/src/ffi/invocation"
  cat >"$tmp/cli-ffi-meta-descriptor-probe/src/ffi/invocation/mod.rs" <<'EOF'
enum DescriptorResolutionError {
    RuntimeOwnerUnavailable(String),
    DescriptorNotFound(String),
}
struct ErrorProjection {
    code: &'static str,
}
impl DescriptorResolutionError {
    fn abi_projection(&self) -> (i32, ErrorProjection) {
        let code = match self {
            Self::RuntimeOwnerUnavailable(_) => "CALLER_IDENTITY_UNAVAILABLE",
            Self::DescriptorNotFound(_) => "DESCRIPTOR_NOT_FOUND",
        };
        (0, ErrorProjection { code })
    }
}
pub unsafe extern "C" fn runtime_resolve_descriptor_ref() {
    let error = DescriptorResolutionError::DescriptorNotFound(String::new());
    let _ = error.abi_projection();
}
fn runtime_resolve_descriptor_ref_json(
    session: &crate::ffi::client::handle::ClientSession,
    request_json: &str,
) -> Result<serde_json::Value, DescriptorResolutionError> {
    let _request: serde_json::Value = serde_json::from_str(request_json).unwrap();
    let _runtime_owner_ura = runtime_owner_ura_from_session(session).map_err(|error| {
        DescriptorResolutionError::RuntimeOwnerUnavailable(format!(
            "resolve descriptor_ref runtime owner: {error}"
        ))
    })?;
    Err(DescriptorResolutionError::DescriptorNotFound(
        "descriptor_ref not found in runtime realm catalog".to_string(),
    ))
}

#[cfg(feature = "axon-pb")]
fn runtime_system_descriptor_catalog_entries() {}
fn runtime_meta_descriptor_catalog_entries() {}

#[test]
fn runtime_descriptor_resolver_requires_runtime_owner_for_realm_catalog() {}
#[test]
fn runtime_descriptor_resolver_does_not_remote_probe_realm_catalog_miss() {}
#[test]
fn descriptor_resolution_errors_project_canonical_runtime_codes() {}
EOF
  if ( CLI_ROOT="$tmp/cli-ffi-meta-descriptor-probe"; check_ffi_descriptor_runtime_owner_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI meta descriptor probe gate to fail"
  fi
  mkdir -p "$tmp/cli-ffi-descriptor-notfound-vocabulary-legacy/src/ffi/invocation"
  cat >"$tmp/cli-ffi-descriptor-notfound-vocabulary-legacy/src/ffi/invocation/mod.rs" <<'EOF'
enum DescriptorResolutionError {
    OwnerOffline(String),
    DescriptorNotFound(String),
}

impl DescriptorResolutionError {
    fn from_remote_probe_rejection(
        error: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationFailure,
    ) -> Self {
        match error {
            crate::daemon::invocation::routing::remote_invoke::RemoteInvocationFailure::DaemonRejected {
                code: tonic::Code::NotFound,
                ..
            } => Self::OwnerOffline(error.to_string()),
            crate::daemon::invocation::routing::remote_invoke::RemoteInvocationFailure::InvocationRejected {
                ref code,
                ..
            } if matches!(
                code.as_str(),
                "ROUTE_NEGATIVE" | "NOT_FOUND" | "DESCRIPTOR_OWNER_OFFLINE"
            ) => Self::OwnerOffline(error.to_string()),
            _ => Self::DescriptorNotFound(error.to_string()),
        }
    }

    fn message(&self) -> &str {
        ""
    }
}

#[test]
fn descriptor_resolution_errors_project_canonical_runtime_codes() {}

#[test]
fn descriptor_remote_probe_not_found_requires_typed_descriptor_vocabulary() {}
EOF
  if ( CLI_ROOT="$tmp/cli-ffi-descriptor-notfound-vocabulary-legacy"; check_ffi_descriptor_probe_not_found_vocabulary_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI descriptor probe generic NOT_FOUND vocabulary gate to fail"
  fi
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/daemon/federation/read_model"
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/daemon/ability/descriptors"
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/daemon/ability/builtins/governance"
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/ffi/invocation"
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/cli/daemon_client"
  mkdir -p "$tmp/cli-opaque-hub-catalog/src/cli/commands/groups"
  cat >"$tmp/cli-opaque-hub-catalog/src/daemon/ability/descriptors/surface.rs" <<'EOF'
pub enum DescriptorError {}

pub struct AbilityDescriptor {}

impl AbilityDescriptor {
    pub fn descriptor_ref(&self) -> Option<String> {
        axon_sdk::invocation::canonical_ability_descriptor_ref("legacy").ok()
    }
}
EOF
  cat >"$tmp/cli-opaque-hub-catalog/src/daemon/federation/read_model/hub_published_abilities.rs" <<'EOF'
use std::collections::BTreeMap;
use crate::daemon::federation::client::ability_contract::HubAbilityEntry;

pub struct HubPublishedAbilityStore {
    entries: BTreeMap<String, HubAbilityEntry>,
}

impl HubPublishedAbilityStore {
    pub fn seed_from_snapshot(&self) {}
    pub fn apply_diff(&self) {}
}
EOF
  cat >"$tmp/cli-opaque-hub-catalog/src/daemon/ability/builtins/governance/meta.rs" <<'EOF'
fn list_abilities_handler() {
    if scope.include_realm {
        for entry in hub_published_abilities.snapshot() {
            merged.push(entry.descriptor);
        }
    }
    scope.apply(&mut merged);
}
EOF
  cat >"$tmp/cli-opaque-hub-catalog/src/ffi/invocation/mod.rs" <<'EOF'
fn dedupe_descriptor_catalog_entries(entries: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for entry in entries {
        if entry.get("descriptor_ref").is_none() {
            continue;
        }
        out.push(entry);
    }
    out
}
EOF
  cat >"$tmp/cli-opaque-hub-catalog/src/cli/daemon_client/ability_catalog.rs" <<'EOF'
fn schema_bound_catalogue_entry(entry: &serde_json::Value, index: usize) -> anyhow::Result<serde_json::Value> {
    if entry.get("descriptor_ref").is_none() {
        anyhow::bail!("missing descriptor_ref");
    }
    Ok(entry.clone())
}
EOF
  cat >"$tmp/cli-opaque-hub-catalog/src/cli/commands/groups/ability.rs" <<'EOF'
fn run_show(entry: serde_json::Value, args: ShowArgs) {
    let name = entry.get("name").and_then(Value::as_str).unwrap_or(&args.ability_ura);
    let version = entry.get("ability_version").and_then(Value::as_str).unwrap_or("-");
    let owner = entry.get("owner_ura").and_then(Value::as_str)
        .or_else(|| name.split_once('.').map(|(owner, _)| owner))
        .unwrap_or("-");
    let schema = entry.get("input_schema")
        .or_else(|| entry.get("schema_summary").and_then(|s| s.get("input")));
}
EOF
  if ( CLI_ROOT="$tmp/cli-opaque-hub-catalog"; check_canonical_ability_catalog_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected opaque hub ability catalog gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-python-private-plain-proof"
  printf 'def _canonical_invocation_bytes(env):\n  return b""\n' \
    > "$tmp/axon-python-private-plain-proof/sdk/python/axon_sdk/invocation/axiom.py"
  if ( AXON_ROOT="$tmp/axon-python-private-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Python private plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-python-legacy-plain-proof"
  printf 'def _legacy_plain_invocation_bytes(env):\n  return b""\ndef _run_legacy_plain_admission(env, sig, resolver, replay, now_ms):\n  return None\n' \
    > "$tmp/axon-python-legacy-plain-proof/sdk/python/axon_sdk/invocation/axiom.py"
  if ( AXON_ROOT="$tmp/axon-python-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Python legacy plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-plain-proof"
  mkdir -p "$tmp/axon-go-plain-proof/sdk/go/axon/invocation"
  printf 'package invocation\nfunc CanonicalInvocationBytes() []byte { return nil }\nfunc canonicalInvocationBytes() []byte { return nil }\n' \
    > "$tmp/axon-go-plain-proof/sdk/go/axon/invocation/axiom.go"
  if ( AXON_ROOT="$tmp/axon-go-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Go plain proof boundary gate to fail"
  fi
  mkdir -p "$tmp/cli-go-plain-proof/sdk/go"
  printf 'package easynet\nfunc CanonicalInvocationBytes() []byte { return nil }\n' \
    > "$tmp/cli-go-plain-proof/sdk/go/invocation_canonical.go"
  if ( AXON_ROOT="$tmp/axon" CLI_ROOT="$tmp/cli-go-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected CLI Go plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-legacy-plain-proof"
  mkdir -p "$tmp/axon-go-legacy-plain-proof/sdk/go/axon/invocation"
  printf 'package invocation\nfunc legacyPlainInvocationBytes() []byte { return nil }\n' \
    > "$tmp/axon-go-legacy-plain-proof/sdk/go/axon/invocation/axiom.go"
  if ( AXON_ROOT="$tmp/axon-go-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Go legacy plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-go-legacy-plain-test-fixture"
  mkdir -p "$tmp/axon-go-legacy-plain-test-fixture/sdk/go/axon/invocation"
  printf 'package invocation\nfunc legacyPlainInvocationBytes() []byte { return nil }\n' \
    > "$tmp/axon-go-legacy-plain-test-fixture/sdk/go/axon/invocation/legacy_plain_fixtures_test.go"
  if ( AXON_ROOT="$tmp/axon-go-legacy-plain-test-fixture"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Go legacy plain proof test fixture gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-plain-proof"
  mkdir -p "$tmp/axon-node-plain-proof/sdk/node/src/invocation"
  printf 'export function canonicalInvocationBytes(env) { return Buffer.alloc(0); }\n' \
    > "$tmp/axon-node-plain-proof/sdk/node/src/invocation/axiom.ts"
  if ( AXON_ROOT="$tmp/axon-node-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Node plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-legacy-plain-proof"
  mkdir -p "$tmp/axon-node-legacy-plain-proof/sdk/node/src/invocation"
  printf 'export function legacyPlainInvocationBytes(env) { return Buffer.alloc(0); }\n' \
    > "$tmp/axon-node-legacy-plain-proof/sdk/node/src/invocation/axiom.ts"
  if ( AXON_ROOT="$tmp/axon-node-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Node legacy plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-node-legacy-plain-script"
  mkdir -p "$tmp/axon-node-legacy-plain-script/sdk/node/scripts"
  printf 'export function legacyPlainInvocationBytes(env) { return Buffer.alloc(0); }\n' \
    > "$tmp/axon-node-legacy-plain-script/sdk/node/scripts/legacy-plain-fixtures.mjs"
  if ( AXON_ROOT="$tmp/axon-node-legacy-plain-script"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Node legacy plain proof script gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-java-plain-proof"
  mkdir -p "$tmp/axon-java-plain-proof/sdk/java/src/main/java/run/axon/sdk/invocation"
  printf 'package run.axon.sdk.invocation; public final class Axiom { public static byte[] canonicalInvocationBytes(Object env) { return new byte[0]; } }\n' \
    > "$tmp/axon-java-plain-proof/sdk/java/src/main/java/run/axon/sdk/invocation/Axiom.java"
  if ( AXON_ROOT="$tmp/axon-java-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Java plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-java-legacy-plain-proof"
  mkdir -p "$tmp/axon-java-legacy-plain-proof/sdk/java/src/main/java/run/axon/sdk/invocation"
  printf 'package run.axon.sdk.invocation; final class Axiom { static byte[] legacyPlainInvocationBytes(Object env) { return new byte[0]; } }\n' \
    > "$tmp/axon-java-legacy-plain-proof/sdk/java/src/main/java/run/axon/sdk/invocation/Axiom.java"
  if ( AXON_ROOT="$tmp/axon-java-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Java legacy plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-plain-proof"
  mkdir -p "$tmp/axon-swift-plain-proof/sdk/swift/Sources/AxonSDK/Invocation"
  printf 'import Foundation\npublic func canonicalInvocationBytes(_ env: Any) -> Data { Data() }\n' \
    > "$tmp/axon-swift-plain-proof/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Swift plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-swift-legacy-plain-proof"
  mkdir -p "$tmp/axon-swift-legacy-plain-proof/sdk/swift/Sources/AxonSDK/Invocation"
  printf 'import Foundation\nfunc legacyPlainInvocationBytes(_ env: Any) -> Data { Data() }\n' \
    > "$tmp/axon-swift-legacy-plain-proof/sdk/swift/Sources/AxonSDK/Invocation/Axiom.swift"
  if ( AXON_ROOT="$tmp/axon-swift-legacy-plain-proof"; check_axon_plain_proof_public_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Swift legacy plain proof boundary gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-rust-local-fast"
  printf 'local-fast-probes = []\n' >> "$tmp/axon-rust-local-fast/sdk/rust/Cargo.toml"
  printf '#[cfg(feature = "local-fast-probes")]\npub fn new_local_fast() {}\n' \
    > "$tmp/axon-rust-local-fast/sdk/rust/src/invocation/local_runtime/mod.rs"
  mkdir -p "$tmp/axon-rust-local-fast/sdk/rust/examples"
  printf 'use axon::invocation::LocalReceiptSigningAuthorityProvider;\n' \
    > "$tmp/axon-rust-local-fast/sdk/rust/examples/local_fast.rs"
  if ( AXON_ROOT="$tmp/axon-rust-local-fast"; check_axon_rust_local_fast_signer_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon Rust local-fast signer boundary gate to fail"
  fi
  mkdir -p "$tmp/axon-fallback/core/runtime-rs/client-sdk/src/domain/easynet"
  printf 'impl AxonClient { pub fn generate_subject_auth() -> EasyNetUserAuth { todo!() } }\n' \
    > "$tmp/axon-fallback/core/runtime-rs/client-sdk/src/domain/easynet/semantic.rs"
  if ( AXON_ROOT="$tmp/axon-fallback"; check_axon_process_local_signer_fallback_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon process-local signer fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-local-fast/src"
  printf '[features]\nlocal-fast-probes = ["axon-sdk/local-fast-probes"]\n' \
    > "$tmp/cli-local-fast/Cargo.toml"
  printf 'let runtime = LocalRuntime::new_local_fast();\n' \
    > "$tmp/cli-local-fast/src/probe.rs"
  if ( CLI_ROOT="$tmp/cli-local-fast"; check_cli_rust_local_fast_signer_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected CLI Rust local-fast signer consumer gate to fail"
  fi
  mkdir -p "$tmp/cli-unsigned-submit/src/daemon/invocation/dispatch" \
    "$tmp/cli-unsigned-submit/src/ffi/invocation"
  cat > "$tmp/cli-unsigned-submit/src/daemon/invocation/dispatch/client.rs" <<'EOF'
pub async fn invoke(&self, invocation: DaemonInvocation) -> Result<Response> {}
pub async fn invoke_stream(&self, invocation: DaemonInvocation) -> Result<Stream> {}
pub async fn invoke_bidi(&self, invocation: DaemonInvocation, streams: Vec<Stream>) -> Result<Bidi> {}
EOF
  cat > "$tmp/cli-unsigned-submit/src/daemon/invocation/dispatch/request.rs" <<'EOF'
fn envelope(&self) -> axon_sdk::pb::axon::v1::Envelope {}
fn into_bidi_open_frame(self) { let mac = signature.unwrap_or_default(); }
/// Builder
EOF
  cat > "$tmp/cli-unsigned-submit/src/ffi/invocation/mod.rs" <<'EOF'
async fn bind(&self, invocation: DaemonInvocation) -> Result<DaemonInvocation> { invocation }
fn runtime_meta_descriptor_catalog_entries() { client.invoke(invocation); }
fn descriptor_catalog_entry_from_descriptor() {}
EOF
  if ( CLI_ROOT="$tmp/cli-unsigned-submit"; check_cli_signed_submission_boundary_contract ) >/dev/null 2>&1; then
    fail "self-test expected unsigned CLI submission boundary gate to fail"
  fi
  mkdir -p "$tmp/cli-bare-runtime/src/daemon/invocation/dispatch"
  printf 'enum RuntimeBinding { CanonicalOnly(LocalRuntime), Daemon(DaemonRuntimeAssembly) }\n' \
    > "$tmp/cli-bare-runtime/src/daemon/invocation/dispatch/deps.rs"
  printf 'pub fn with_local_runtime(self, runtime: LocalRuntime) -> Self { self }\n' \
    > "$tmp/cli-bare-runtime/src/daemon/invocation/dispatch/daemon_invocation_service.rs"
  if ( CLI_ROOT="$tmp/cli-bare-runtime"; check_daemon_runtime_assembly_contract ) >/dev/null 2>&1; then
    fail "self-test expected bare daemon LocalRuntime construction gate to fail"
  fi
  mkdir -p "$tmp/cli-sidecar-template/src/cli/commands/groups"
  cp "$ROOT/src/cli/commands/groups/plugin_template.rs" \
    "$tmp/cli-sidecar-template/src/cli/commands/groups/plugin_template.rs"
  perl -0pi -e 's/serve_exec_plugin\(handle\)/json.loads(sys.stdin.readline())/' \
    "$tmp/cli-sidecar-template/src/cli/commands/groups/plugin_template.rs"
  if ( CLI_ROOT="$tmp/cli-sidecar-template"; check_plugin_sidecar_helper_matrix_contract ) >/dev/null 2>&1; then
    fail "self-test expected naked sidecar frame template gate to fail"
  fi
  mkdir -p "$tmp/cli-browser-mock/src/daemon/ability/builtins/device_control" \
    "$tmp/cli-browser-mock/ability-descriptors/system/device_control" \
    "$tmp/cli-browser-mock/src/daemon/ability/catalog" \
    "$tmp/cli-browser-mock/src/daemon/ability/names" \
    "$tmp/cli-browser-mock/src/daemon/ability/wire" \
    "$tmp/cli-browser-mock/src/daemon/invocation/dispatch" \
    "$tmp/cli-browser-mock/sdk/go" \
    "$tmp/cli-browser-mock/tools/scripts" \
    "$tmp/cli-browser-mock/tests/scripts" \
    "$tmp/cli-browser-mock/tests"
  printf 'pub mod browser;\n' \
    > "$tmp/cli-browser-mock/src/daemon/ability/builtins/device_control/mod.rs"
  printf 'pub fn register() { browser_session_ability::register(&mut reg); }\n' \
    > "$tmp/cli-browser-mock/src/daemon/ability/catalog/build.rs"
  printf 'DeviceBrowser browser.open_session\n' \
    > "$tmp/cli-browser-mock/src/daemon/ability/conformance.rs"
  printf 'pub const BROWSER_OPEN_SESSION: &str = "browser.open_session";\n' \
    > "$tmp/cli-browser-mock/src/daemon/ability/names/device_control.rs"
  printf 'const PLACEHOLDER_WEBP: &[u8] = &[]; // is_placeholder V0 MOCK\n' \
    > "$tmp/cli-browser-mock/src/daemon/ability/builtins/device_control/browser.rs"
  printf 'name = "browser.open_session"\ndescription = "[V0 MOCK]"\ncapability_state = "cutover_ready"\n' \
    > "$tmp/cli-browser-mock/ability-descriptors/system/device_control/browser.open_session.ability.toml"
  printf '#!/usr/bin/env bash\n' \
    > "$tmp/cli-browser-mock/tools/scripts/check-browser-session-service-boundary.sh"
  if ( CLI_ROOT="$tmp/cli-browser-mock"; check_retired_browser_mock_surface_contract ) >/dev/null 2>&1; then
    fail "self-test expected retired browser placeholder ability gate to fail"
  fi
  mkdir -p "$tmp/cli-federation-directory-v1-descriptor/ability-descriptors/system/federation"
  printf 'name = "federation.subscribe_directory"\ndescription = "Subscribe to legacy federation directory snapshots and deltas."\ncall_mode = "stream"\ncapability_state = "cutover_ready"\n' \
    > "$tmp/cli-federation-directory-v1-descriptor/ability-descriptors/system/federation/federation.subscribe_directory.ability.toml"
  if ( CLI_ROOT="$tmp/cli-federation-directory-v1-descriptor"; check_retired_federation_directory_v1_stream_contract ) >/dev/null 2>&1; then
    fail "self-test expected retired federation directory v1 descriptor gate to fail"
  fi
  mkdir -p "$tmp/cli-ability-deploy-product/src/daemon/ability/builtins/device_control/ability_management"
  printf 'fn deploy() { /* EasyRemote writes namespace here */ }\n' \
    > "$tmp/cli-ability-deploy-product/src/daemon/ability/builtins/device_control/ability_management/ops.rs"
  if ( CLI_ROOT="$tmp/cli-ability-deploy-product"; check_ability_deploy_product_neutrality_contract ) >/dev/null 2>&1; then
    fail "self-test expected ability.deploy product vocabulary gate to fail"
  fi
  mkdir -p "$tmp/cli-ability-manifest-exec/src/daemon/ability/builtins/agents" \
    "$tmp/cli-ability-manifest-exec/src/daemon/ability/builtins/governance" \
    "$tmp/cli-ability-manifest-exec/src/daemon/ability"
  printf 'fn exec() { /* owning agent'\''s chat handler" (legacy default) */ }\n' \
    > "$tmp/cli-ability-manifest-exec/src/daemon/ability/manifest.rs"
  printf 'no executable binding and cannot enter the live capability catalog\n' \
    > "$tmp/cli-ability-manifest-exec/src/daemon/ability/builtins/agents/authoring.rs"
  printf 'manifest without [exec] must not be routed through an LLM-mediated handler\n' \
    > "$tmp/cli-ability-manifest-exec/src/daemon/ability/builtins/agents/chat.rs"
  printf 'manifest without [exec] must remain discovery-only, not a runtime binding\n' \
    > "$tmp/cli-ability-manifest-exec/src/daemon/ability/builtins/governance/teach.rs"
  if ( CLI_ROOT="$tmp/cli-ability-manifest-exec"; check_ability_manifest_exec_absence_contract ) >/dev/null 2>&1; then
    fail "self-test expected ability manifest exec absence gate to fail"
  fi
  mkdir -p "$tmp/cli-local-runtime-identity/src/bin" \
    "$tmp/cli-local-runtime-identity/src/daemon/execution/loop_instance" \
    "$tmp/cli-local-runtime-identity/src/daemon/execution"
  printf 'fn spawn_schedule_tick() { let _ = easynet_cli::core::ura::device_ura("default", "self"); }\n' \
    > "$tmp/cli-local-runtime-identity/src/bin/easynet-daemon.rs"
  printf 'pub struct KernelLoopInvocationDriver; impl KernelLoopInvocationDriver { fn invoke(&self) { let _ = crate::core::ura::resource_dot_ura("default", "loop.x", "body/1"); } }\n' \
    > "$tmp/cli-local-runtime-identity/src/daemon/execution/loop_instance/mod.rs"
  printf 'pub struct LocalRuntimeInvocationIdentity;\n' \
    > "$tmp/cli-local-runtime-identity/src/daemon/execution/runtime_identity.rs"
  if ( CLI_ROOT="$tmp/cli-local-runtime-identity"; check_daemon_local_runtime_identity_contract ) >/dev/null 2>&1; then
    fail "self-test expected local runtime identity default-URA gate to fail"
  fi
  mkdir -p "$tmp/cli-local-invoke-fallback/src/daemon/invocation/routing" \
    "$tmp/cli-local-invoke-fallback/src/support/platform" \
    "$tmp/cli-local-invoke-fallback/src/daemon/ability/builtins/integrations/mcp" \
    "$tmp/cli-local-invoke-fallback/src/daemon/ability/builtins/integrations/a2a" \
    "$tmp/cli-local-invoke-fallback/src/daemon/ability/catalog/profiles"
  cat > "$tmp/cli-local-invoke-fallback/src/daemon/invocation/routing/target.rs" <<'EOF'
fn daemon_system_subject_ura_for_descriptor() {}
pub(crate) fn daemon_system_subject_ura(&self) -> anyhow::Result<String> { todo!() }
pub fn local_root_for_target() {}
EOF
  cat > "$tmp/cli-local-invoke-fallback/src/support/platform/local_invoke.rs" <<'EOF'
pub enum LocalInvokeErrorKind { DaemonOffline }
pub fn classify_invoke_error(err: &anyhow::Error) -> LocalInvokeErrorKind { todo!() }
fn invoke_target_root_derived_subject_timeout() {}
fn root_context_for_target() {}
fn local_system_context_for_agent_target_uses_agent_owner_subject() {}
fn local_system_context_for_hub_target_uses_ability_subject() {}
// fallback executor permission
EOF
  printf 'fn bridge() { local_root_for_target(); }\n' \
    > "$tmp/cli-local-invoke-fallback/src/daemon/ability/builtins/integrations/mcp/bridge.rs"
  printf 'fn bridge() { local_root_for_target(); }\n' \
    > "$tmp/cli-local-invoke-fallback/src/daemon/ability/builtins/integrations/a2a/bridge.rs"
  printf 'fn profile() { root_context_for_target(); }\n' \
    > "$tmp/cli-local-invoke-fallback/src/daemon/ability/catalog/profiles/mcp.rs"
  if ( CLI_ROOT="$tmp/cli-local-invoke-fallback"; check_local_ability_target_subject_policy_contract ) >/dev/null 2>&1; then
    fail "self-test expected local invoke fallback classifier gate to fail"
  fi
  mkdir -p "$tmp/cli-kernel-session-read-model/src/daemon/boot/kernel" \
    "$tmp/cli-kernel-session-read-model/src/daemon/execution"
  printf 'fn dispatch_via_local_runtime() { Session { node: NodeId::new("self"), tenant: TenantId::default_v1() }; }\n' \
    > "$tmp/cli-kernel-session-read-model/src/daemon/boot/kernel/mod.rs"
  printf 'pub struct LocalRuntimeSessionProjection;\n' \
    > "$tmp/cli-kernel-session-read-model/src/daemon/execution/runtime_identity.rs"
  if ( CLI_ROOT="$tmp/cli-kernel-session-read-model"; check_kernel_runtime_session_read_model_contract ) >/dev/null 2>&1; then
    fail "self-test expected kernel session read-model default projection gate to fail"
  fi
  mkdir -p "$tmp/cli-session-binding/src/bin" \
    "$tmp/cli-session-binding/src/daemon/execution/session" \
    "$tmp/cli-session-binding/src/daemon/ability/builtins/device_control"
  printf 'fn main() { let _ = kernel.session_service(); }\n' \
    > "$tmp/cli-session-binding/src/bin/easynet-daemon.rs"
  printf 'pub struct SessionService; impl SessionService { pub fn admit(&self, session: Session) { let _ = Session { node: NodeId::new("self"), tenant: TenantId::default_v1(), ..session }; } }\n' \
    > "$tmp/cli-session-binding/src/daemon/execution/session/mod.rs"
  printf 'fn list_handler(svc: &SessionService) { let _ = NodeId::new("self"); }\n' \
    > "$tmp/cli-session-binding/src/daemon/ability/builtins/device_control/session.rs"
  if ( CLI_ROOT="$tmp/cli-session-binding"; check_daemon_runtime_session_binding_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon runtime session binding default gate to fail"
  fi
  mkdir -p "$tmp/cli-discuss-binding/src/bin" \
    "$tmp/cli-discuss-binding/src/daemon/execution/mission/discuss" \
    "$tmp/cli-discuss-binding/src/daemon/ability/builtins/automation"
  printf 'fn main() { let _ = kernel.discuss_service(); }\n' \
    > "$tmp/cli-discuss-binding/src/bin/easynet-daemon.rs"
  printf 'pub struct DiscussService; impl DiscussService { pub fn create(&self) { let _ = DiscussRoom { origin_node: NodeId::new("self"), tenant: TenantId::default_v1() }; } }\n' \
    > "$tmp/cli-discuss-binding/src/daemon/execution/mission/discuss/mod.rs"
  printf 'fn create_handler(svc: &DiscussService) { svc.create(); }\n' \
    > "$tmp/cli-discuss-binding/src/daemon/ability/builtins/automation/discuss.rs"
  if ( CLI_ROOT="$tmp/cli-discuss-binding"; check_daemon_runtime_discuss_binding_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon runtime discuss binding default gate to fail"
  fi
  mkdir -p "$tmp/cli-tenant-store-binding/src/bin" \
    "$tmp/cli-tenant-store-binding/src/daemon/execution/schedule" \
    "$tmp/cli-tenant-store-binding/src/daemon/execution/loop_instance" \
    "$tmp/cli-tenant-store-binding/src/daemon/ability/builtins/automation"
  printf 'fn main() { let tenant = TenantId::default_v1(); kernel.schedule_service().bind(&tenant); kernel.loop_service().bind(&tenant); }\n' \
    > "$tmp/cli-tenant-store-binding/src/bin/easynet-daemon.rs"
  printf 'pub struct ScheduleService; impl ScheduleService { pub fn add(&self, mut entry: ScheduleEntry) { entry.tenant = TenantId::default_v1(); } }\n' \
    > "$tmp/cli-tenant-store-binding/src/daemon/execution/schedule/mod.rs"
  printf 'pub struct LoopService; impl LoopService { pub fn create(&self) { let tenant = TenantId::default_v1(); } }\n' \
    > "$tmp/cli-tenant-store-binding/src/daemon/execution/loop_instance/mod.rs"
  printf 'fn add_handler(svc: &ScheduleService) { let entry = ScheduleEntry { tenant: TenantId::default_v1() }; svc.add(entry); }\n' \
    > "$tmp/cli-tenant-store-binding/src/daemon/ability/builtins/automation/schedule.rs"
  if ( CLI_ROOT="$tmp/cli-tenant-store-binding"; check_daemon_runtime_tenant_store_binding_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon runtime tenant-store default gate to fail"
  fi
  mkdir -p "$tmp/schedule-store-current-schema-legacy/src/daemon/execution/schedule"
  cat >"$tmp/schedule-store-current-schema-legacy/src/daemon/execution/schedule/mod.rs" <<'EOF'
fn schedule_entry_round_trips_with_prompt_field() {
    // Legacy entry without the prompt field should parse with prompt=None.
}
EOF
  cat >"$tmp/schedule-store-current-schema-legacy/src/daemon/execution/schedule/store.rs" <<'EOF'
#[derive(Debug, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(flatten)]
    entry: ScheduleEntry,
}

fn default_schema_version() -> u32 { 1 }
EOF
  if ( CLI_ROOT="$tmp/schedule-store-current-schema-legacy"; check_schedule_store_current_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected schedule store current-schema gate to fail"
  fi
  mkdir -p "$tmp/cli-directory-fallback/sdk/go" \
    "$tmp/cli-directory-fallback/sdk/python/easynet_sdk" \
    "$tmp/cli-directory-fallback/sdk/python/tests"
  cp "$ROOT/sdk/go/directory.go" "$tmp/cli-directory-fallback/sdk/go/directory.go"
  cp "$ROOT/sdk/go/directory_test.go" "$tmp/cli-directory-fallback/sdk/go/directory_test.go"
  cp "$ROOT/sdk/python/easynet_sdk/directory.py" \
    "$tmp/cli-directory-fallback/sdk/python/easynet_sdk/directory.py"
  cp "$ROOT/sdk/python/tests/test_directory.py" \
    "$tmp/cli-directory-fallback/sdk/python/tests/test_directory.py"
  perl -0pi -e 's/return DirectoryResolution\{\}, invalidDirectory\("Directory answer must be an object", nil\)/return DirectoryResolution{}, nil/' \
    "$tmp/cli-directory-fallback/sdk/go/directory.go"
  perl -0pi -e 's/            raise _invalid\("Directory answer must be an object"\)/            pass/' \
    "$tmp/cli-directory-fallback/sdk/python/easynet_sdk/directory.py"
  perl -0pi -e 's/return nil, invalidDirectory\("Directory "\+key\+" item must be an object", nil\)/continue/' \
    "$tmp/cli-directory-fallback/sdk/go/directory.go"
  perl -0pi -e 's/if not isinstance\(value, Mapping\):\n        raise _invalid\(f"Directory \{name\} must be an object"\)\n    return dict\(value\)/return _required_mapping(value, f"Directory {name}")/' \
    "$tmp/cli-directory-fallback/sdk/python/easynet_sdk/directory.py"
  if ( CLI_ROOT="$tmp/cli-directory-fallback"; check_sdk_directory_projection_fail_closed_contract ) >/dev/null 2>&1; then
    fail "self-test expected SDK Directory provider-output fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-principal-fallback/sdk/go" \
    "$tmp/cli-principal-fallback/sdk/python/easynet_sdk" \
    "$tmp/cli-principal-fallback/sdk/python/tests"
  cp "$ROOT/sdk/go/principal.go" "$tmp/cli-principal-fallback/sdk/go/principal.go"
  cp "$ROOT/sdk/go/principal_test.go" "$tmp/cli-principal-fallback/sdk/go/principal_test.go"
  cp "$ROOT/sdk/python/easynet_sdk/principal.py" \
    "$tmp/cli-principal-fallback/sdk/python/easynet_sdk/principal.py"
  cp "$ROOT/sdk/python/tests/test_principal.py" \
    "$tmp/cli-principal-fallback/sdk/python/tests/test_principal.py"
  printf '\nfunc principalStringFromMap(raw map[string]any, key string) string { return "" }\n' \
    >> "$tmp/cli-principal-fallback/sdk/go/principal.go"
  printf '\ndef _text(value: object) -> str:\n    return ""\n' \
    >> "$tmp/cli-principal-fallback/sdk/python/easynet_sdk/principal.py"
  if ( CLI_ROOT="$tmp/cli-principal-fallback"; check_sdk_principal_projection_fail_closed_contract ) >/dev/null 2>&1; then
    fail "self-test expected SDK Principal provider-output fallback gate to fail"
  fi
  mkdir -p "$tmp/runtime-owner-signer-legacy/src/daemon/identity"
  printf '%s\n' \
    'pub struct RuntimeSigningIdentity;' \
    'impl RuntimeSigningIdentity {' \
    '  pub fn load(owner_ura: impl Into<String>, provider: Arc<dyn SelfIdentity>) -> Result<Self, SelfIdentityError> {' \
    '    let owner_ura = owner_ura.into();' \
    '    let owner_ura = owner_ura.trim();' \
    '    if owner_ura.is_empty() { return Err(SelfIdentityError::InvalidOwner); }' \
    '    let public_key = provider.public_key(owner_ura)?;' \
    '    Ok(Self::from_public_projection(owner_ura, public_key, provider))' \
    '  }' \
    '}' \
    '#[async_trait::async_trait]' \
    'impl CanonicalSigner for RuntimeSigningIdentity {}' \
    > "$tmp/runtime-owner-signer-legacy/src/daemon/identity/self_identity.rs"
  if ( CLI_ROOT="$tmp/runtime-owner-signer-legacy"; check_runtime_owner_signer_custody_contract ) >/dev/null 2>&1; then
    fail "self-test expected runtime-owner signer User custody gate to fail"
  fi
  mkdir -p "$tmp/remote-invocation-signer-first-legacy/src/daemon/invocation/routing"
  cat >"$tmp/remote-invocation-signer-first-legacy/src/daemon/invocation/routing/remote_invoke.rs" <<'EOF'
pub(crate) fn invoke_remote_target(request: RemoteInvocationRequest<'_>) -> anyhow::Result<Value> {
    let socket_path = crate::support::platform::local_daemon_grpc::resolve_socket_path();
    ensure_remote_invocation_daemon_accepting(&socket_path)?;
    let signer = load_remote_invocation_caller_signer(request.caller_ura.as_str())?;
    invoke_remote_target_on_ready_socket(request, signer, socket_path)
}

pub(crate) fn load_remote_invocation_caller_signer(caller_ura: &str) -> anyhow::Result<RemoteInvocationCallerSigner> {
    crate::daemon::identity::self_identity::load_runtime_caller_signer(caller_ura.to_string())
}

pub(crate) fn invoke_remote_target_stream(request: RemoteInvocationRequest<'_>) -> anyhow::Result<Vec<Frame>> {
    let caller_ura = request.caller_ura;
    let socket_path = crate::support::platform::local_daemon_grpc::resolve_socket_path();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) { anyhow::bail!("daemon not running"); }
    let signer = crate::daemon::identity::self_identity::load_runtime_caller_signer(caller_ura.clone())?;
    Ok(Vec::new())
}

pub(crate) fn invoke_remote_target_bidi_json_frames(request: RemoteInvocationRequest<'_>) -> anyhow::Result<Vec<Frame>> {
    let caller_ura = request.caller_ura;
    let socket_path = crate::support::platform::local_daemon_grpc::resolve_socket_path();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) { anyhow::bail!("daemon not running"); }
    let signer = crate::daemon::identity::self_identity::load_runtime_caller_signer(caller_ura.clone())?;
    Ok(Vec::new())
}

fn checked_remote_invocation_ura() {}
EOF
  if ( CLI_ROOT="$tmp/remote-invocation-signer-first-legacy"; check_remote_invocation_signer_first_contract ) >/dev/null 2>&1; then
    fail "self-test expected remote invocation signer-first gate to fail"
  fi
  mkdir -p "$tmp/runtime-identity-vocabulary-legacy/src/daemon/identity" \
    "$tmp/runtime-identity-vocabulary-legacy/src/daemon/ability/authority"
  printf '/// Product URA owned by the local daemon process advertised in control.json.\n' \
    > "$tmp/runtime-identity-vocabulary-legacy/src/daemon/identity/local_invocation.rs"
  printf '/// Product-level authority fact for a local hosted-agent call.\n' \
    > "$tmp/runtime-identity-vocabulary-legacy/src/daemon/ability/authority/mod.rs"
  if ( CLI_ROOT="$tmp/runtime-identity-vocabulary-legacy"; check_daemon_runtime_identity_vocabulary_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon runtime identity vocabulary gate to fail"
  fi
  run_ura_vocabulary_self_test "$tmp/ura-vocabulary"
  cp -R "$tmp/axon" "$tmp/axon-uri-vector"
  mkdir -p "$tmp/axon-uri-vector/packaging/protocol-pack/conformance-vectors"
  printf '{"description":"Cross-language URI canonicalization vectors","vectors":[{"input_uri":"easynet:///r/example/agent/a","canonical_uri":"easynet:///r/example/agent/a"}]}\n' \
    > "$tmp/axon-uri-vector/packaging/protocol-pack/conformance-vectors/easynet-uri-v1.json"
  if ( AXON_ROOT="$tmp/axon-uri-vector"; check_axon_protocol_pack_ura_vector_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon URI vector terminology gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-uri-docs"
  mkdir -p "$tmp/axon-uri-docs/document/concepts" "$tmp/axon-uri-docs/document/rfcs"
  printf 'message AgentIdentity { string uri = 1; }\nIdentity messages carry URI + profile.\n' \
    > "$tmp/axon-uri-docs/document/concepts/AXIOM.tex"
  printf 'caller.uri\nSystemAgent canonical URI format\n' \
    > "$tmp/axon-uri-docs/document/rfcs/001-envelope-axiom-alignment.md"
  printf 'AgentUri owner\nEvery Agent has a URI.\n' \
    > "$tmp/axon-uri-docs/document/concepts/ONTOLOGY_AGENT_ABILITY.md"
  printf 'inputs.caller.uri\nempty URI\n' \
    > "$tmp/axon-uri-docs/document/rfcs/001-pr2-acceptance-checklist.md"
  printf 'message AgentIdentity { string uri = 1; }\n{"peer_uri":"easynet:///r/example/agent/a"}\nfind_peer_by_uri(agent_ura)\n' \
    > "$tmp/axon-uri-docs/document/rfcs/002-keyring-and-keyresolver.md"
  mkdir -p "$tmp/axon-uri-docs/sdk"
  printf 'agents have "uri": "easynet:///r/example/agent/a"\n' \
    > "$tmp/axon-uri-docs/sdk/SDK_INTERFACE_SPEC.md"
  printf 'envelope.caller.uri\n' \
    > "$tmp/axon-uri-docs/sdk/FEDERATION_INVOKE_SCHEMAS.md"
  mkdir -p "$tmp/axon-uri-docs/sdk/conformance/cases/axiom"
  printf 'fixed caller URIs\n' \
    > "$tmp/axon-uri-docs/sdk/conformance/cases/axiom/README.md"
  printf '{"description":"byte-identical URIs"}\n' \
    > "$tmp/axon-uri-docs/sdk/conformance/cases/axiom/axiom-identity-composite-required.json"
  if ( AXON_ROOT="$tmp/axon-uri-docs"; check_axon_normative_ura_document_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon normative URI document gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-uri-proto"
  mkdir -p "$tmp/axon-uri-proto/core/proto/axon/v1" \
    "$tmp/axon-uri-proto/core/runtime-rs/client-sdk/proto/axon/v1" \
    "$tmp/axon-uri-proto/sdk/rust/proto/axon/v1"
  printf 'syntax = "proto3";\n// canonical device URIs should be enumerated.\nmessage DeviceList {}\n' \
    > "$tmp/axon-uri-proto/core/proto/axon/v1/federation.proto"
  cp "$tmp/axon-uri-proto/core/proto/axon/v1/federation.proto" \
    "$tmp/axon-uri-proto/core/runtime-rs/client-sdk/proto/axon/v1/federation.proto"
  cp "$tmp/axon-uri-proto/core/proto/axon/v1/federation.proto" \
    "$tmp/axon-uri-proto/sdk/rust/proto/axon/v1/federation.proto"
  if ( AXON_ROOT="$tmp/axon-uri-proto"; check_axon_proto_ura_vocabulary_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon proto URI terminology gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-sdk-product-ura"
  mkdir -p "$tmp/axon-sdk-product-ura/sdk/node/src" \
    "$tmp/axon-sdk-product-ura/sdk/swift/Sources/EasyNetAxon/Invocation"
  printf 'throw new AxonConfigError(`subject_ura must be an EasyNet URA: ${normalized}`);\n' \
    > "$tmp/axon-sdk-product-ura/sdk/node/src/index.ts"
  printf 'private let SYSTEM_URI = "easynet:///r/_system/agents/local@1"\n' \
    > "$tmp/axon-sdk-product-ura/sdk/swift/Sources/EasyNetAxon/Invocation/LocalRuntime.swift"
  if ( AXON_ROOT="$tmp/axon-sdk-product-ura"; check_axon_sdk_product_neutral_ura_error_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon SDK product-specific URA error gate to fail"
  fi
  cp -R "$tmp/axon" "$tmp/axon-active-ura-source"
  mkdir -p "$tmp/axon-active-ura-source/core/runtime-rs/dendrite-bridge/docs" \
    "$tmp/axon-active-ura-source/sdk/go/easynet"
  printf 'type SigningConfig struct { CallerURI string }\nInvokeAbility(handle, tenantID, resourceURI, payloadJSON, metadata, timeoutMs)\n' \
    > "$tmp/axon-active-ura-source/core/runtime-rs/dendrite-bridge/docs/AUTHENTICATED_INVOCATION.md"
  printf 'package easynet\nfunc TestSignedInvokeRequest_RejectsEmptyCalleeURI() {}\n' \
    > "$tmp/axon-active-ura-source/sdk/go/easynet/signed_invoke_request_test.go"
  printf 'package easynet\nfunc TestNormalizeHubEndpointConvertsAxonURI() {}\n' \
    > "$tmp/axon-active-ura-source/sdk/go/easynet/ability_lifecycle_server_test.go"
  if ( AXON_ROOT="$tmp/axon-active-ura-source"; check_axon_active_ura_source_test_contract ) >/dev/null 2>&1; then
    fail "self-test expected Axon active source/test URI terminology gate to fail"
  fi
  AXON_ROOT="$tmp/axon"
  ( AXON_ROOT="$CANONICAL_LIFECYCLE_AXON_ROOT"; check_lifecycle_evidence_freshness_contract )
  ( AXON_ROOT="$CANONICAL_LIFECYCLE_AXON_ROOT"; check_manifest_contract )
  mkdir -p "$tmp/go-sdk-ura-alias/sdk/go" "$tmp/go-sdk-ura-alias/sdk/conformance"
  printf 'package easynet\ntype URA struct{}\ntype Ura = URA\n' \
    > "$tmp/go-sdk-ura-alias/sdk/go/ura.go"
  printf '{"languages":{"go":["URA","Ura"]}}\n' \
    > "$tmp/go-sdk-ura-alias/sdk/conformance/canonical-public-api.json"
  printf '{"cells":[{"shape_evidence":[{"item":"Ura"}]}]}\n' \
    > "$tmp/go-sdk-ura-alias/sdk/conformance/sdk-parity-matrix.json"
  if ( check_go_sdk_public_ura_alias_contract "$tmp/go-sdk-ura-alias" ) >/dev/null 2>&1; then
    fail "self-test expected Go SDK Ura alias gate to fail"
  fi
  mkdir -p "$tmp/go-sdk-product-resource/sdk/go"
  printf '%s\n' \
    'package easynet' \
    'var productResourceNamespaces = map[ResourceNamespace]struct{}{}' \
    'func productResourceURA(realm, userID, namespace, path string) string { return "" }' \
    'func projectProductResourcePath(kind URAKind, path string) (ResourceNamespace, string) { return "", path }' \
    > "$tmp/go-sdk-product-resource/sdk/go/resource_namespace.go"
  printf '%s\n' \
    'package easynet' \
    'func ResourceURA(realm, userID, namespace, path string) string { return productResourceURA(realm, userID, namespace, path) }' \
    > "$tmp/go-sdk-product-resource/sdk/go/ura.go"
  if ( check_go_sdk_runtime_resource_namespace_contract "$tmp/go-sdk-product-resource" ) >/dev/null 2>&1; then
    fail "self-test expected Go SDK product resource namespace gate to fail"
  fi
  mkdir -p "$tmp/python-sdk-product-addressing/sdk/python/easynet_sdk"
  printf '%s\n' \
    'def _product_ura_kind(canonical_kind: str) -> str:' \
    '    return "hub" if canonical_kind == "authority" else canonical_kind' \
    'def _product_ability_owner_kind(canonical_kind: str) -> str:' \
    '    return "hub" if canonical_kind == "authority" else canonical_kind' \
    > "$tmp/python-sdk-product-addressing/sdk/python/easynet_sdk/axon_addressing.py"
  if ( check_python_sdk_runtime_addressing_kind_contract "$tmp/python-sdk-product-addressing" ) >/dev/null 2>&1; then
    fail "self-test expected Python SDK product addressing-kind gate to fail"
  fi
  mkdir -p "$tmp/advertise-agent-legacy/src/daemon/invocation/dispatch"
  printf '%s\n' \
    '#[derive(Debug, Clone, Deserialize)]' \
    'pub struct AdvertiseAgentRequest {' \
    '  pub signing_authority: Option<AdvertiseSigningAuthorityRequest>,' \
    '  pub host_ura: Option<String>,' \
    '}' \
    'impl AdvertiseAgentRequest { fn host_ura(&self) -> Option<&str> { self.host_ura.as_deref() } }' \
    > "$tmp/advertise-agent-legacy/src/daemon/invocation/dispatch/federation_wrappers.rs"
  if ( check_advertise_agent_ingress_contract "$tmp/advertise-agent-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected advertise_agent retired host_ura ingress gate to fail"
  fi
  mkdir -p "$tmp/agent-start-model-legacy/src/daemon/ability/builtins/agents"
  printf '%s\n' \
    'fn start_agent_locked(args: Value) {' \
    '  let model_present = args.get("model_present").and_then(Value::as_bool).unwrap_or_else(|| args.get("model").is_some());' \
    '}' \
    'pub fn start_agent_input_schema() -> Value { json!({"properties":{"model":{"type":"string"}}}) }' \
    > "$tmp/agent-start-model-legacy/src/daemon/ability/builtins/agents/lifecycle.rs"
  if ( check_agent_start_model_intent_contract "$tmp/agent-start-model-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected agent.start model_present inference gate to fail"
  fi
  mkdir -p "$tmp/invocation-history-attempt-key/src/daemon/ability/builtins/governance"
  printf '%s\n' \
    'fn get_history(&self, args: Value) -> anyhow::Result<Value> {' \
    '  if let Some(attempt_id) = args.get("key").and_then(|key| key.get("attempt_id")).and_then(non_empty_str) {' \
    '    let path = attempt_ledger_path_from_config();' \
    '    let attempt = InvocationAttemptLedger::open(&path)?.get(attempt_id)?;' \
    '    return Ok(json!({"diagnostic_record": attempt}));' \
    '  }' \
    '  Ok(json!({}))' \
    '}' \
    '' \
    'fn get_record(&self, args: Value) -> anyhow::Result<Value> { Ok(json!({})) }' \
    '' \
    'fn key_schema() -> Value {' \
    '  json!({"properties":{"ura":{},"request_id":{},"trace_id":{},"attempt_id":{}}})' \
    '}' \
    '' \
    'fn filter_schema() -> Value { json!({}) }' \
    > "$tmp/invocation-history-attempt-key/src/daemon/ability/builtins/governance/invocation_history.rs"
  if ( check_invocation_history_get_key_contract "$tmp/invocation-history-attempt-key" ) >/dev/null 2>&1; then
    fail "self-test expected invocation.history.get attempt_id key gate to fail"
  fi
  mkdir -p "$tmp/invocation-history-ledger-ura-legacy/src/daemon/ability/builtins/governance"
  cat >"$tmp/invocation-history-ledger-ura-legacy/src/daemon/ability/builtins/governance/invocation_history.rs" <<'EOF'
fn ledger_resource_ura() -> Option<String> {
    let hosted_identity = AgentAggregateRepository::load_hosted_identity_status().ok()?;
    let parsed = crate::core::ura::parse_ura(hosted_identity.host_device_agent_ura()?).ok()?;
    let owner = format!("device.{}", parsed.device_id()?);
    Some(crate::core::ura::resource_dot_ura(&parsed.realm, &owner, "billing/invocations"))
}

fn fetch_records_from_path() {}

#[test]
fn history_key_schema_excludes_attempt_id() {}

#[test]
fn get_history_rejects_attempt_id_key() {}
EOF
  if ( check_invocation_history_ledger_ura_contract "$tmp/invocation-history-ledger-ura-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected invocation.history ledger_ura projection fallback gate to fail"
  fi
  mkdir -p "$tmp/core-ura-realm-projection-legacy/src/core/ura" \
    "$tmp/core-ura-realm-projection-legacy/src/daemon/keyring" \
    "$tmp/core-ura-realm-projection-legacy/src/daemon/invocation/admission"
  printf '%s\n' \
    'pub use axon_sdk::ura::*;' \
    'pub fn hub_ura(realm: &str) -> String { authority_ura(realm) }' \
    > "$tmp/core-ura-realm-projection-legacy/src/core/ura/mod.rs"
  printf '%s\n' \
    'use crate::core::ura::{parse_ura, URAKind};' \
    'fn issue(source_user_ura: &str) { let _ = parse_realm_from_user_ura(source_user_ura); }' \
    'fn parse_realm_from_user_ura(ura: &str) -> Option<String> {' \
    '  let parsed = parse_ura(ura).ok()?;' \
    '  (parsed.kind == URAKind::User).then_some(parsed.realm)' \
    '}' \
    > "$tmp/core-ura-realm-projection-legacy/src/daemon/keyring/abilities.rs"
  printf '%s\n' \
    'use crate::core::ura::{parse_ura, URAKind};' \
    '/// 2. **federated fallback**: otherwise, look up a binding.' \
    '/// duplicated rather than re-exported to keep the resolver local.' \
    'fn parse_realm_from_user_ura(ura: &str) -> Option<String> {' \
    '  let parsed = parse_ura(ura).ok()?;' \
    '  (parsed.kind == URAKind::User).then_some(parsed.realm)' \
    '}' \
    > "$tmp/core-ura-realm-projection-legacy/src/daemon/keyring/resolver.rs"
  printf '%s\n' \
    'pub(crate) fn parse_realm_from_ura(ura: &str) -> Option<String> {' \
    '  crate::core::ura::parse_ura(ura).ok().map(|parsed| parsed.realm)' \
    '}' \
    > "$tmp/core-ura-realm-projection-legacy/src/daemon/invocation/admission/runtime_trust.rs"
  printf '%s\n' \
    'pub(crate) fn parse_realm_from_ura(ura: &str) -> Option<String> {' \
    '  crate::daemon::invocation::admission::runtime_trust::parse_realm_from_ura(ura)' \
    '}' \
    > "$tmp/core-ura-realm-projection-legacy/src/daemon/invocation/admission/register_device_pubkey.rs"
  printf '%s\n' \
    'fn resolve(agent_ura: &str) {' \
    '  let _ = crate::daemon::invocation::admission::register_device_pubkey::parse_realm_from_ura(agent_ura);' \
    '}' \
    > "$tmp/core-ura-realm-projection-legacy/src/daemon/invocation/admission/federated_key_resolver.rs"
  if ( check_core_ura_realm_projection_contract "$tmp/core-ura-realm-projection-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected core URA realm projection gate to fail"
  fi
  mkdir -p "$tmp/federation-realm-resolver-legacy/src/daemon/federation"
  cat >"$tmp/federation-realm-resolver-legacy/src/daemon/federation/resolver.rs" <<'EOF'
pub enum AdmissionMode { LocalFast, Federated }
pub enum UraScope { Prv, Org }
pub struct ResolverConfig { pub static_hubs: std::collections::HashMap<String, Vec<String>>, pub easynet_rendezvous: Vec<String> }
pub struct RealmResolution {
    pub mode: AdmissionMode,
    pub scope: UraScope,
    pub hub_endpoints: Vec<String>,
    pub realm: String,
}
/// anything else   → Local mode by default (preserves pre-RFC-002 behaviour)
pub fn resolve(realm: &str, cfg: &ResolverConfig) -> RealmResolution {
    let lower = realm.to_ascii_lowercase();
    if lower.ends_with(".localhost") || lower == "localhost" {
        return RealmResolution { mode: AdmissionMode::LocalFast, scope: UraScope::Prv, hub_endpoints: vec![], realm: realm.to_string() };
    }
    if lower.contains('.') {
        return RealmResolution { mode: AdmissionMode::Federated, scope: UraScope::Org, hub_endpoints: vec![], realm: realm.to_string() };
    }
    // Bare token (legacy `tenant-test`, `acme`, etc.). Backward-compat:
    // treat as Local-fast under prv scope.
    RealmResolution { mode: AdmissionMode::LocalFast, scope: UraScope::Prv, hub_endpoints: vec![], realm: realm.to_string() }
}
#[cfg(test)]
mod tests {
    #[test] fn bare_token_falls_back_to_local() {}
}
EOF
  if ( check_federation_realm_resolver_contract "$tmp/federation-realm-resolver-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected federation realm resolver fallback gate to fail"
  fi
  mkdir -p "$tmp/resolve-key-request-dto-legacy/src/daemon/federation/client" \
    "$tmp/resolve-key-request-dto-legacy/src/daemon/invocation/admission" \
    "$tmp/resolve-key-request-dto-legacy/src/cli/commands"
  printf '%s\n' \
    '#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]' \
    'pub struct ResolveKeyRequest {' \
    '  pub agent_ura: String,' \
    '  pub presented_pubkey_b64: Option<String>,' \
    '}' \
    > "$tmp/resolve-key-request-dto-legacy/src/daemon/federation/wire_contract.rs"
  printf '%s\n' \
    '#[derive(Debug, Clone, Serialize)]' \
    'pub struct ResolveKeyArgs {' \
    '  pub agent_ura: String,' \
    '}' \
    > "$tmp/resolve-key-request-dto-legacy/src/daemon/federation/client/ability_contract.rs"
  printf '%s\n' \
    'fn resolve_federated(agent_ura: &str, pk: Option<&str>) {' \
    '  let mut args = serde_json::json!({ "agent_ura": agent_ura });' \
    '  if let Some(pk) = pk { args["presented_pubkey_b64"] = serde_json::Value::String(pk.to_string()); }' \
    '}' \
    > "$tmp/resolve-key-request-dto-legacy/src/daemon/invocation/admission/federated_key_resolver.rs"
  printf '%s\n' \
    'fn join(target: Target) {' \
    '  let resolve_args = crate::daemon::federation::client::ability_contract::ResolveKeyArgs { agent_ura: target.hub_ura.clone() };' \
    '}' \
    > "$tmp/resolve-key-request-dto-legacy/src/cli/commands/join.rs"
  if ( check_resolve_key_request_dto_contract "$tmp/resolve-key-request-dto-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected resolve_key request DTO ownership gate to fail"
  fi
  mkdir -p "$tmp/invocation-history-filter-legacy/src/daemon/ability/builtins/governance"
  cat >"$tmp/invocation-history-filter-legacy/src/daemon/ability/builtins/governance/invocation_history.rs" <<'EOF'
fn fetch_key_from_value(value: &Value) -> anyhow::Result<InvocationLedgerFetchKey> {
    if let Some(ura) = value.get("ura").and_then(non_empty_str) {
        return Ok(InvocationLedgerFetchKey::InvocationUra(ura.to_string()));
    }
    anyhow::bail!("key")
}

fn apply_filter_object(mut query: InvocationLedgerQuery, object: &serde_json::Map<String, Value>) -> anyhow::Result<InvocationLedgerQuery> {
    if let Some(caller) = optional_filter_string(object, "caller_ura")? { query = query.caller_ura(caller); }
    if let Some(ability) = optional_filter_string(object, "ability_ura")? { query = query.ability_ura(ability); }
    if let Some(state) = optional_filter_string(object, "state")? { query = query.state(state); }
    Ok(query)
}

fn validate_filter_keys() {}

fn query_from_args_rejects_malformed_scope_uras_before_ledger_read() {}
fn query_from_args_rejects_malformed_key_ura_before_ledger_read() {}
fn list_history_rejects_malformed_ability_set_filters() {}
EOF
  if ( check_invocation_history_filter_scope_contract "$tmp/invocation-history-filter-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected invocation.history filter scope gate to fail"
  fi
  mkdir -p "$tmp/cli-invocation-history-read-legacy/src/cli/commands/groups"
  cat >"$tmp/cli-invocation-history-read-legacy/src/cli/commands/groups/invocation.rs" <<'EOF'
fn run_stats(args: StatsArgs) -> anyhow::Result<()> {
    invoke_invocation_ability(ABILITY_HISTORY_LIST, json!({ "limit": args.limit }))
}

fn fetch_history_list(args: &ListArgs) -> anyhow::Result<HistoryListResponse> {
    invoke_invocation_ability(ABILITY_HISTORY_LIST, history_list_args(args))
}

fn fetch_history_record(id: &str) -> anyhow::Result<HistoryGetResponse> {
    invoke_invocation_ability(ABILITY_HISTORY_GET, json!({ "key": history_key_for_id(id) }))
}

fn fetch_trace_graph_by_trace_id(trace_id: &str) -> anyhow::Result<TraceGetResponse> {
    invoke_invocation_ability(ABILITY_TRACE_GET, json!({ "key": { "trace_id": trace_id } }))
}

fn invoke_invocation_ability<T>(ability: &str, args: Value) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    crate::support::platform::local_invoke::invoke_local_ability(ability, args)
}

fn history_list_args(args: &ListArgs) -> Value { json!({ "limit": args.limit }) }
fn history_key_for_id(id: &str) -> Value { json!({ "request_id": id }) }
EOF
  if ( check_cli_invocation_history_read_model_contract "$tmp/cli-invocation-history-read-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected CLI invocation history read-model gate to fail"
  fi
  mkdir -p "$tmp/local-runtime-state-read-subject-legacy/src/support/platform"
  cat >"$tmp/local-runtime-state-read-subject-legacy/src/support/platform/local_invoke.rs" <<'EOF'
pub struct LocalRuntimeStateReadIssuer;

impl LocalRuntimeStateReadIssuer {
    fn subject_ura() -> anyhow::Result<String> {
        crate::daemon::identity::local_invocation::local_daemon_ura()
    }
}

fn runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity() {}
fn runtime_state_read_subject_rejects_missing_user_id_before_device_fallback() {}

/// Invoke a canonical local target with public-ingress tuple facts.
EOF
  if ( check_local_runtime_state_read_subject_contract "$tmp/local-runtime-state-read-subject-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected local runtime-state read subject gate to fail"
  fi
  mkdir -p "$tmp/runtime-state-kind-default-legacy/src/daemon/persistence"
  cat >"$tmp/runtime-state-kind-default-legacy/src/daemon/persistence/config.rs" <<'EOF'
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    #[default]
    DaemonOnly,
}

pub struct RuntimeState {
    pub endpoint: String,
    #[serde(default)]
    pub runtime_kind: RuntimeKind,
    pub pid: Option<u32>,
}

fn runtime_state_defaults_to_daemon_when_kind_missing() {}
EOF
  if ( check_runtime_state_kind_required_contract "$tmp/runtime-state-kind-default-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected runtime-state runtime_kind default gate to fail"
  fi
  mkdir -p "$tmp/sdk-history-authority-legacy/sdk/go" \
    "$tmp/sdk-history-authority-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-history-authority-legacy/sdk/python/tests"
  printf '%s\n' \
    'type SessionAuthority struct{}' \
    'func validateSessionHistorySessionBinding(authority *SessionAuthority, subjectURA string) error {' \
    '  if !sessionHistoryAuthoritySubjectMatches(authority, subjectURA) { return nil }' \
    '  return nil' \
    '}' \
    'func sessionHistoryAuthoritySubjectMatches(authority *SessionAuthority, subjectURA string) bool { return true }' \
    'func runtimeCallDetails() {}' \
    'func validateSessionHistoryFilterBinding() {}' \
    > "$tmp/sdk-history-authority-legacy/sdk/go/authorized_runtime_session.go"
  printf '%s\n' \
    'func TestAuthorizedRuntimeSessionHistoryAllowsUserOwnedResourceSubjectBeforeReceiptProvider() {}' \
    'func TestAuthorizedRuntimeSessionHistoryRejectsPathSubstringOwnerSubjectBeforeReceiptProvider() {}' \
    > "$tmp/sdk-history-authority-legacy/sdk/go/authorized_runtime_session_test.go"
  printf '%s\n' \
    'from ._session_authority_subjects import session_authority_admits_subject' \
    'def _validate_session_history_authority_binding(authority, subject_ura):' \
    '    if not _session_history_authority_subject_matches(authority, subject_ura):' \
    '        return None' \
    'def _session_history_authority_subject_matches(authority, subject_ura):' \
    '    return authority.subject_ura.strip() == subject_ura.strip()' \
    'def _validate_runtime_call_required(value, field_name):' \
    '    return None' \
    'def _validate_session_history_filter_binding():' \
    '    return None' \
    > "$tmp/sdk-history-authority-legacy/sdk/python/easynet_sdk/authorized_runtime_session.py"
  printf '%s\n' \
    'def session_authority_admits_subject(authority, subject_ura):' \
    '    subject = parse_ura(subject_ura.strip())' \
    '    owner_id = subject.components.get("owner_id")' \
    '    owner_user_id = authority.session_owner_user_id.strip()' \
    '    return owner_id == f"user.{owner_user_id}" or owner_id.startswith("agent.")' \
    > "$tmp/sdk-history-authority-legacy/sdk/python/easynet_sdk/_session_authority_subjects.py"
  printf '%s\n' \
    'def test_history_allows_user_owned_resource_subject_before_receipt_provider(): pass' \
    'def test_history_rejects_path_substring_owner_subject_before_receipt_provider(): pass' \
    > "$tmp/sdk-history-authority-legacy/sdk/python/tests/test_authorized_runtime_session.py"
  if ( check_sdk_history_authority_subject_contract "$tmp/sdk-history-authority-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK history authority canonical-admission gate to fail"
  fi
  mkdir -p "$tmp/sdk-descriptor-resolution-error-legacy/sdk/go" \
    "$tmp/sdk-descriptor-resolution-error-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-descriptor-resolution-error-legacy/sdk/python/tests"
  printf '%s\n' \
    'func descriptorResolutionFromError(err error) DescriptorResolution {' \
    '  if strings.Contains(fmt.Sprint(err), "offline") { return DescriptorResolution{State: DescriptorOwnerOffline} }' \
    '  if IsCode(err, ErrAbilityNotFound) || IsCode(err, ErrNotFound) {' \
    '    return DescriptorResolution{State: DescriptorNotFound}' \
    '  }' \
    '  if IsCode(err, ErrDescriptorNotFound) { return DescriptorResolution{State: DescriptorNotFound} }' \
    '  if IsCode(err, ErrDescriptorOwnerOffline) { return DescriptorResolution{State: DescriptorOwnerOffline} }' \
    '  return DescriptorResolution{State: DescriptorUnavailable}' \
    '}' \
    'func sessionIntentDetails() {}' \
    > "$tmp/sdk-descriptor-resolution-error-legacy/sdk/go/authorized_runtime_session.go"
  printf '%s\n' \
    'func TestAuthorizedRuntimeDescriptorResolutionRequiresDescriptorVocabulary(t *testing.T) {}' \
    'func TestAuthorizedRuntimeDescriptorResolutionRequiresTypedOwnerOffline(t *testing.T) {}' \
    > "$tmp/sdk-descriptor-resolution-error-legacy/sdk/go/authorized_runtime_session_test.go"
  printf '%s\n' \
    'def _descriptor_resolution_from_error(error):' \
    '    if "offline" in text.lower():' \
    '        return DescriptorResolution(DescriptorResolutionState.OWNER_OFFLINE)' \
    '    if error.code in {ErrorCode.ABILITY_NOT_FOUND, ErrorCode.NOT_FOUND, ErrorCode.DESCRIPTOR_NOT_FOUND}:' \
    '        return DescriptorResolution(DescriptorResolutionState.NOT_FOUND)' \
    '    if error.code == ErrorCode.DESCRIPTOR_OWNER_OFFLINE:' \
    '        return DescriptorResolution(DescriptorResolutionState.OWNER_OFFLINE)' \
    '    return DescriptorResolution(DescriptorResolutionState.UNAVAILABLE)' \
    '' \
    'def _intent_details(intent): pass' \
    > "$tmp/sdk-descriptor-resolution-error-legacy/sdk/python/easynet_sdk/authorized_runtime_session.py"
  printf '%s\n' \
    'def test_descriptor_resolution_requires_descriptor_vocabulary(): pass' \
    'def test_descriptor_resolution_requires_typed_owner_offline(): pass' \
    > "$tmp/sdk-descriptor-resolution-error-legacy/sdk/python/tests/test_authorized_runtime_session.py"
  if ( check_sdk_descriptor_resolution_error_vocabulary_contract "$tmp/sdk-descriptor-resolution-error-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK descriptor resolution legacy not-found vocabulary gate to fail"
  fi
  mkdir -p "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/go" \
    "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/python/tests"
  printf '%s\n' \
    'func abilityDescriptorNotFound(abilityURA string) error {' \
    '  return &SDKError{Code: ErrNotFound}' \
    '}' \
    > "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/go/ability_descriptor.go"
  printf 'func TestRuntimeAbilityDescriptorProviderGetReportsDescriptorNotFound(t *testing.T) {}\n' \
    > "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/go/ability_descriptor_test.go"
  printf '%s\n' \
    'def _not_found(ability_ura):' \
    '    return SDKError(code=ErrorCode.NOT_FOUND)' \
    '' \
    'def _invalid_descriptor(): pass' \
    > "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/python/easynet_sdk/ability_descriptor.py"
  printf 'def test_runtime_ability_descriptor_provider_get_reports_descriptor_not_found(): pass\n' \
    > "$tmp/sdk-ability-descriptor-not-found-legacy/sdk/python/tests/test_ability_descriptor.py"
  if ( check_sdk_ability_descriptor_not_found_vocabulary_contract "$tmp/sdk-ability-descriptor-not-found-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK ability descriptor generic NOT_FOUND gate to fail"
  fi
  mkdir -p "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/go" \
    "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/python/easynet_sdk/providers/easynet" \
    "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/python/tests"
  printf '%s\n' \
    'func LoadRuntimeSigningIdentity() error {' \
    '  return signer.publicKey(owner)' \
    '}' \
    'func EnsureRuntimeSigningIdentity() error {' \
    '  return signer.ensure(owner)' \
    '}' \
    'func runtimeIdentityError(err error) error {' \
    '  return err' \
    '}' \
    'func (c runtimeKeyringClient) sign() {}' \
    > "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/go/runtime_identity.go"
  printf 'func TestRuntimeSigningIdentityProjectsMissingKeyAsCallerSignerUnavailable(t *testing.T) {}\n' \
    > "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/go/runtime_identity_test.go"
  printf '%s\n' \
    'def _runtime_identity_error(error):' \
    '    return SDKError(code=error.code, stage="runtime_identity")' \
    '' \
    'def load_runtime_signing_identity(): pass' \
    > "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/python/easynet_sdk/providers/easynet/keyring.py"
  printf 'def test_rejection_projects_missing_runtime_identity_to_caller_signer_unavailable(): pass\n' \
    > "$tmp/sdk-runtime-identity-signer-not-found-legacy/sdk/python/tests/test_runtime_identity.py"
  if ( check_sdk_runtime_identity_signer_not_found_contract "$tmp/sdk-runtime-identity-signer-not-found-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK runtime identity signer not-found gate to fail"
  fi
  mkdir -p "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/go/provider/easynet" \
    "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/python/easynet_sdk/providers/easynet" \
    "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/python/tests"
  printf '%s\n' \
    'func providerRuntimeInstanceID(decoded map[string]any) (string, error) {' \
    '  deviceID := providerIdentityString(decoded, "device_id")' \
    '  nodeID := providerIdentityString(decoded, "node_id")' \
    '  if deviceID != "" && nodeID != "" && deviceID != nodeID {' \
    '    return "", fmt.Errorf("daemon credentials contain conflicting device_id and node_id")' \
    '  }' \
    '  if deviceID != "" { return deviceID, nil }' \
    '  return nodeID, nil' \
    '}' \
    > "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/go/provider/easynet/identity.go"
  printf 'func TestProviderMapsDaemonNodeIDAliasToCanonicalRuntimeIdentity(t *testing.T) {}\n' \
    > "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/go/provider/easynet/lifecycle_test.go"
  printf '%s\n' \
    'def _runtime_instance_id(raw: Mapping[str, object]) -> str:' \
    '    device_id = _text(raw, "device_id")' \
    '    node_id = _text(raw, "node_id")' \
    '    if device_id and node_id and device_id != node_id:' \
    '        raise ValueError("daemon credentials contain conflicting device_id and node_id")' \
    '    return device_id or node_id' \
    > "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/python/easynet_sdk/providers/easynet/identity.py"
  printf 'def test_easynet_provider_maps_daemon_node_id_alias_to_canonical_projection(): pass\n' \
    > "$tmp/sdk-easynet-provider-identity-alias-legacy/sdk/python/tests/test_runtime_environment.py"
  if ( check_sdk_easynet_provider_identity_alias_contract "$tmp/sdk-easynet-provider-identity-alias-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK EasyNet provider identity alias gate to fail"
  fi
  mkdir -p "$tmp/sdk-python-transport-stream-content-type-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-python-transport-stream-content-type-legacy/sdk/python/tests"
  printf '%s\n' \
    'def _stream_event_dict(event: StreamEvent) -> dict[str, object]:' \
    '    return {' \
    '        "content_type": event.payload_content_type,' \
    '        "payload_content_type": event.payload_content_type,' \
    '    }' \
    > "$tmp/sdk-python-transport-stream-content-type-legacy/sdk/python/easynet_sdk/transport.py"
  printf '%s\n' \
    'def test_invocation_result_adapter_delegates_stream_and_bidi(self):' \
    '    self.assertIn("content_type", event)' \
    > "$tmp/sdk-python-transport-stream-content-type-legacy/sdk/python/tests/test_transport.py"
  if ( check_sdk_python_transport_stream_event_projection_contract "$tmp/sdk-python-transport-stream-content-type-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK Python transport stream content-type alias gate to fail"
  fi
  mkdir -p "$tmp/sdk-python-invocation-result-adapter-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-python-invocation-result-adapter-legacy/sdk/python/tests"
  printf '%s\n' \
    'def _result_response_dict(result: Mapping[str, object]) -> dict[str, object]:' \
    '    if result.get("ok") is not True:' \
    '        raise SDKError()' \
    '    terminal_state = _terminal_state_name(result.get("terminal_state"))' \
    '    return {' \
    '        "state": _terminal_state_code(terminal_state),' \
    '        "result_content_type": result.get("output_content_type"),' \
    '        "result_base64": result.get("output_base64"),' \
    '        "result_json": result.get("output_json"),' \
    '        "sdk_runtime_result": dict(result),' \
    '    }' \
    '' \
    '_TERMINAL_STATE_CODES = {"completed": 5}' \
    '' \
    'def _terminal_state_name(value): return value or "Unspecified"' \
    '' \
    'def _terminal_state_code(value): return _TERMINAL_STATE_CODES.get(value, 0)' \
    > "$tmp/sdk-python-invocation-result-adapter-legacy/sdk/python/easynet_sdk/transport.py"
  printf '%s\n' \
    'def test_invocation_result_adapter_projects_runtime_result_shape(self):' \
    '    self.assertEqual(result["result_content_type"], "application/json")' \
    '    self.assertEqual(result["sdk_runtime_result"]["terminal_state"], "Completed")' \
    > "$tmp/sdk-python-invocation-result-adapter-legacy/sdk/python/tests/test_transport.py"
  if ( check_sdk_python_invocation_result_adapter_projection_contract "$tmp/sdk-python-invocation-result-adapter-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK Python invocation result adapter legacy wrapper gate to fail"
  fi
  mkdir -p "$tmp/principal-lifecycle-fallback/src/cli/commands/groups"
  printf '%s\n' \
    'fn principal_ability_realm_source(args: &Value) -> anyhow::Result<&str> {' \
    '  args.pointer("/request/principal_ura")' \
    '    .or_else(|| args.get("principal_ura"))' \
    '    .and_then(Value::as_str)' \
    '    .ok_or_else(|| anyhow!("missing"))' \
    '}' \
    > "$tmp/principal-lifecycle-fallback/src/cli/commands/groups/principal.rs"
  if ( check_principal_lifecycle_cli_schema_contract "$tmp/principal-lifecycle-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected PrincipalLifecycle CLI top-level fallback gate to fail"
  fi
  mkdir -p "$tmp/auth-agents-legacy/src/cli/commands"
  printf '%s\n' \
    'pub fn run_agents(args: AgentsArgs) -> anyhow::Result<()> {' \
    '  for a in &resp.items {' \
    '    let agent_id = a.get("agent_id").or_else(|| a.get("ura"));' \
    '    let name = a.get("display_name").or_else(|| a.get("name"));' \
    '  }' \
    '  Ok(())' \
    '}' \
    '' \
    '// ── device remove' \
    > "$tmp/auth-agents-legacy/src/cli/commands/auth.rs"
  if ( check_auth_agents_backend_shape_contract "$tmp/auth-agents-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected auth agents retired row alias gate to fail"
  fi
  mkdir -p "$tmp/principal-lifecycle-command-log-legacy/src/daemon/invocation/admission"
  printf '%s\n' \
    'struct PrincipalStore {' \
    '  #[serde(default)]' \
    '  principals: BTreeMap<String, PrincipalRecord>,' \
    '}' \
    'struct PrincipalRecord {' \
    '  #[serde(default, skip_serializing_if = "Option::is_none")]' \
    '  enrollment_proof: Option<PrincipalProofRef>,' \
    '  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]' \
    '  consumed_recovery_proofs: BTreeMap<String, i64>,' \
    '  #[serde(default)]' \
    '  enrollments: Vec<EnrollmentCapability>,' \
    '  #[serde(default)]' \
    '  grants: Vec<AuthorizationGrant>,' \
    '  #[serde(default)]' \
    '  command_log: BTreeMap<String, u64>,' \
    '}' \
    'fn existing_principal_store_requires_principals_fact() {}' \
    'const STORE_MESSAGE: &str = "missing field `principals`";' \
    'const STORE_FAIL_CLOSED: &str = "existing principal store without principals must fail closed";' \
    'fn principal_record_requires_enrollment_proof_fact() {}' \
    'const ENROLLMENT_PROOF_MESSAGE: &str = "missing field `enrollment_proof`";' \
    'const ENROLLMENT_PROOF_FAIL_CLOSED: &str = "principal record without enrollment_proof must fail closed";' \
    'fn principal_record_requires_lifecycle_collection_facts() {}' \
    'const COLLECTION_MESSAGE: &str = "missing field `{field}`";' \
    'const COLLECTION_FAIL_CLOSED: &str = "principal record without lifecycle collections must fail closed";' \
    'fn principal_record_requires_idempotency_command_log_fact() {}' \
    'const MESSAGE: &str = "missing field `command_log`";' \
    'const FAIL_CLOSED: &str = "principal record without command_log must fail closed";' \
    > "$tmp/principal-lifecycle-command-log-legacy/src/daemon/invocation/admission/principal_lifecycle.rs"
  if ( check_principal_lifecycle_store_idempotency_schema_contract "$tmp/principal-lifecycle-command-log-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected principal lifecycle command_log legacy default gate to fail"
  fi
  mkdir -p "$tmp/pages-identity-legacy/src/daemon/ability/builtins/resources/pages"
  printf '%s\n' \
    'pub struct PagesIdentity { pub user: Option<String>, pub realm: Option<String>, pub listener_port: Option<u16> }' \
    'impl PagesIdentity {' \
    '  pub fn from_env() -> Self {' \
    '    let user = crate::daemon::persistence::config::load_credentials().ok().and_then(|c| c.username);' \
    '    let listener_port = std::env::var("EASYNET_PAGES_PORT").ok().and_then(|s| s.parse::<u16>().ok());' \
    '    Self { user, realm: None, listener_port }' \
    '  }' \
    '}' \
    > "$tmp/pages-identity-legacy/src/daemon/ability/builtins/resources/pages/identity.rs"
  if ( check_pages_identity_credentials_contract "$tmp/pages-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected Pages identity credential fallback gate to fail"
  fi
  mkdir -p "$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/resources/pages" \
    "$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/governance" \
    "$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/integrations" \
    "$tmp/pages-user-root-realm-legacy/src/daemon/ability/catalog" \
    "$tmp/pages-user-root-realm-legacy/src/daemon/persistence" \
    "$tmp/pages-user-root-realm-legacy/src/bin"
  cat >"$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/resources/pages/identity.rs" <<'EOF'
pub struct PagesIdentity { pub user: Option<String>, pub realm: Option<String>, pub listener_port: Option<u16> }
impl PagesIdentity {
  pub fn try_from_env() -> anyhow::Result<Self> {
    let _creds = crate::daemon::persistence::config::load_credentials_optional()?;
    let _port = pages_listener_port_from_env()?;
    let _message = "EASYNET_PAGES_PORT must be greater than 0";
    Ok(Self { user: Some("alice".into()), realm: None, listener_port: None })
  }
}
fn pages_listener_port_from_env() -> anyhow::Result<Option<u16>> { Ok(None) }
#[cfg(test)]
mod tests {
  #[test] fn pages_identity_missing_credentials_is_unpaired_state() {}
  #[test] fn pages_identity_rejects_malformed_credentials_instead_of_defaulting() {}
  #[test] fn pages_identity_rejects_invalid_port_instead_of_defaulting() {}
  #[test] fn pages_identity_user_root_projection_requires_realm() {}
  #[test] fn pages_identity_user_root_projection_accepts_complete_identity() {}
}
EOF
  cat >"$tmp/pages-user-root-realm-legacy/src/daemon/persistence/config.rs" <<'EOF'
pub fn load_credentials_optional() -> anyhow::Result<Option<Credentials>> { Ok(None) }
#[cfg(test)]
mod tests { #[test] fn load_credentials_optional_rejects_malformed_existing_file() {} }
EOF
  printf '%s\n' 'fn boot() { PagesIdentity::try_from_env(); }' \
    > "$tmp/pages-user-root-realm-legacy/src/bin/easynet-daemon.rs"
  printf '%s\n' 'fn smoke() { PagesIdentity::try_from_env(); }' \
    > "$tmp/pages-user-root-realm-legacy/src/bin/real-user-smoke.rs"
  cat >"$tmp/pages-user-root-realm-legacy/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build(pages_identity: PagesIdentity) -> anyhow::Result<()> {
  if let Some(user) = pages_identity.user.clone() {
    let realm = pages_identity
      .realm
      .clone()
      .unwrap_or_else(|| crate::core::ura::REALM_EASYNET.to_string());
    api_key_ability::register(&mut reg, &user);
  }
  openai_compat_ability::set_identity(pages_identity.clone());
  Ok(())
}
EOF
  cat >"$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/governance/api_key.rs" <<'EOF'
fn realm() -> String {
  std::env::var("EASYNET_PAGES_REALM")
    .unwrap_or_else(|_| crate::core::ura::REALM_EASYNET.to_string())
}
pub fn register(reg: &mut AxonAbilityCatalog, user: &str) {}
EOF
  cat >"$tmp/pages-user-root-realm-legacy/src/daemon/ability/builtins/integrations/openai_compat.rs" <<'EOF'
static OPENAI_IDENTITY: ProcessSingleton<OpenAICompatIdentity> = ProcessSingleton::last_writer_wins();
impl OpenAICompatIdentity {
  fn from_pages_identity(identity: PagesIdentity) -> Self {
    Self { user: identity.user, realm: identity.realm.unwrap_or_else(|| crate::core::ura::REALM_EASYNET.to_string()) }
  }
}
fn compatibility_file_identity(identity: Option<&OpenAICompatIdentity>) -> anyhow::Result<(String, String)> { todo!() }
EOF
  printf '%s\n' '#[test] fn unrelated_registry_test() {}' \
    > "$tmp/pages-user-root-realm-legacy/src/daemon/ability/catalog/assembly_tests.rs"
  if ( check_pages_identity_credentials_contract "$tmp/pages-user-root-realm-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected Pages user-root realm default gate to fail"
  fi
  mkdir -p "$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/resources/pages" \
    "$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/governance" \
    "$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/integrations" \
    "$tmp/pages-cli-identity-legacy/src/daemon/ability/catalog" \
    "$tmp/pages-cli-identity-legacy/src/daemon/persistence" \
    "$tmp/pages-cli-identity-legacy/src/cli/commands" \
    "$tmp/pages-cli-identity-legacy/src/bin"
  cat >"$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/resources/pages/identity.rs" <<'EOF'
pub struct PagesUserRootIdentity { pub user: String, pub realm: String }
pub struct PagesIdentity { pub user: Option<String>, pub realm: Option<String>, pub listener_port: Option<u16> }
impl PagesIdentity {
  pub fn try_from_env() -> anyhow::Result<Self> {
    let _creds = crate::daemon::persistence::config::load_credentials_optional()?;
    let _port = pages_listener_port_from_env()?;
    let _message = "EASYNET_PAGES_PORT must be greater than 0";
    Ok(Self { user: Some("alice".into()), realm: Some("localhost".into()), listener_port: None })
  }
  pub fn user_root_identity(&self) -> anyhow::Result<Option<PagesUserRootIdentity>> { Ok(Some(PagesUserRootIdentity { user: "alice".into(), realm: "localhost".into() })) }
}
fn pages_listener_port_from_env() -> anyhow::Result<Option<u16>> { Ok(None) }
#[cfg(test)]
mod tests {
  #[test] fn pages_identity_missing_credentials_is_unpaired_state() {}
  #[test] fn pages_identity_rejects_malformed_credentials_instead_of_defaulting() {}
  #[test] fn pages_identity_rejects_invalid_port_instead_of_defaulting() {}
  #[test] fn pages_identity_user_root_projection_requires_realm() {}
  #[test] fn pages_identity_user_root_projection_accepts_complete_identity() {}
}
EOF
  cat >"$tmp/pages-cli-identity-legacy/src/daemon/persistence/config.rs" <<'EOF'
pub struct Credentials;
pub fn load_credentials_optional() -> anyhow::Result<Option<Credentials>> { Ok(None) }
#[cfg(test)]
mod tests { #[test] fn load_credentials_optional_rejects_malformed_existing_file() {} }
EOF
  printf '%s\n' 'fn boot() { PagesIdentity::try_from_env(); }' \
    > "$tmp/pages-cli-identity-legacy/src/bin/easynet-daemon.rs"
  printf '%s\n' 'fn smoke() { PagesIdentity::try_from_env(); }' \
    > "$tmp/pages-cli-identity-legacy/src/bin/real-user-smoke.rs"
  cat >"$tmp/pages-cli-identity-legacy/src/daemon/ability/catalog/build.rs" <<'EOF'
fn build(pages_identity: PagesIdentity) -> anyhow::Result<()> {
  if let Some(PagesUserRootIdentity { user, realm: pages_realm }) = pages_identity.user_root_identity()? {
    api_key_ability::register(&mut reg, &user, &pages_realm);
  }
  openai_compat_ability::set_identity(pages_identity.clone())?;
  Ok(())
}
EOF
  cat >"$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/governance/api_key.rs" <<'EOF'
pub fn register(reg: &mut AxonAbilityCatalog, user: &str, realm: &str) {}
fn route(u1: String, r1: String, u2: String, r2: String, u3: String, r3: String, args: Value) {
  handle_create(&u1, &r1, args);
  handle_list(&u2, &r2, args);
  handle_revoke(&u3, &r3, args);
}
#[test] fn create_stamps_registered_realm_without_product_default_lookup() {}
EOF
  cat >"$tmp/pages-cli-identity-legacy/src/daemon/ability/builtins/integrations/openai_compat.rs" <<'EOF'
static OPENAI_IDENTITY: ProcessSingleton<Option<OpenAICompatIdentity>> = ProcessSingleton::last_writer_wins();
impl OpenAICompatIdentity {
  fn from_pages_identity(identity: PagesIdentity) -> anyhow::Result<Self> { Ok(Self { user: identity.user, realm: identity.realm }) }
}
fn openai_file_user_root_identity(identity: Option<&OpenAICompatIdentity>) -> anyhow::Result<(String, String)> { todo!() }
#[test] fn openai_runtime_rejects_partial_user_identity_without_realm() {}
EOF
  printf '%s\n' '#[test] fn user_rooted_registry_rejects_paired_identity_without_realm() {}' \
    > "$tmp/pages-cli-identity-legacy/src/daemon/ability/catalog/assembly_tests.rs"
  cat >"$tmp/pages-cli-identity-legacy/src/cli/commands/pages.rs" <<'EOF'
fn current_user() -> anyhow::Result<String> {
  crate::daemon::persistence::config::load_credentials().ok().and_then(|c| c.username).ok_or_else(|| anyhow::anyhow!("missing"))
}
fn current_realm() -> String {
  std::env::var("EASYNET_PAGES_REALM").unwrap_or_else(|_| crate::core::ura::REALM_EASYNET.to_string())
}
EOF
  if ( check_pages_identity_credentials_contract "$tmp/pages-cli-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected Pages CLI identity fallback gate to fail"
  fi
  mkdir -p "$tmp/api-key-cli-identity-legacy/src/cli/commands"
  cat >"$tmp/api-key-cli-identity-legacy/src/cli/commands/api_key_cli.rs" <<'EOF'
fn current_user() -> anyhow::Result<String> {
    crate::daemon::persistence::config::load_credentials()
        .ok()
        .and_then(|c| c.username)
        .ok_or_else(|| anyhow::anyhow!("missing"))
}
#[cfg(test)]
mod tests {
    #[test] fn current_user_accepts_explicit_dev_override() {}
    #[test] fn current_user_reads_valid_paired_credentials() {}
    #[test] fn current_user_reports_unpaired_only_when_credentials_file_is_absent() {}
    #[test] fn current_user_rejects_malformed_existing_credentials() {}
    #[test] fn current_user_rejects_credentials_without_username() {}
}
EOF
  if ( check_api_key_cli_identity_contract "$tmp/api-key-cli-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected API key CLI identity fallback gate to fail"
  fi
  mkdir -p "$tmp/api-key-store-schema-legacy/src/daemon/ability/builtins/governance"
  cat >"$tmp/api-key-store-schema-legacy/src/daemon/ability/builtins/governance/api_key.rs" <<'EOF'
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub id_prefix: String,
    pub token_hash: String,
    pub user_ura: String,
    pub label: Option<String>,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
    pub last_used_at: Option<u64>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ApiKeyStore {
    #[serde(default)]
    pub keys: Vec<ApiKeyEntry>,
}
fn load_store() -> anyhow::Result<ApiKeyStore> {
    let text = std::fs::read_to_string("api_keys.toml").unwrap_or_default();
    Ok(toml::from_str(&text).unwrap_or_default())
}
#[cfg(test)]
mod tests {
    #[test] fn missing_store_is_fresh_install_empty_state() {}
    #[test] fn bearer_resolution_rejects_malformed_store_instead_of_unknown_token() {}
}
EOF
  if ( check_api_key_store_schema_contract "$tmp/api-key-store-schema-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected API key store schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/local-api-key-cache-legacy/src/daemon/ability/builtins/governance" \
    "$tmp/local-api-key-cache-legacy/src/cli/commands"
  printf '%s\n' \
    'pub fn read_local_default_token() -> Option<String> {' \
    '  let home = std::env::var("HOME").ok()?;' \
    '  let path = PathBuf::from(home).join(".easynet").join("api_keys.local.toml");' \
    '  let text = fs::read_to_string(path).ok()?;' \
    '  #[derive(Deserialize)]' \
    '  struct LocalTokens { #[serde(default)] default_token: Option<String> }' \
    '  let parsed: LocalTokens = toml::from_str(&text).ok()?;' \
    '  parsed.default_token' \
    '}' \
    'pub fn write_local_default_token(token: &str) -> anyhow::Result<()> { Ok(()) }' \
    > "$tmp/local-api-key-cache-legacy/src/daemon/ability/builtins/governance/api_key.rs"
  printf '%s\n' \
    'fn pick_token(arg: Option<String>) -> Option<String> {' \
    '  api_key::read_local_default_token()' \
    '}' \
    'fn run(args: LlmApiArgs) -> anyhow::Result<()> {' \
    '  let token = pick_token(args.key);' \
    '  Ok(())' \
    '}' \
    > "$tmp/local-api-key-cache-legacy/src/cli/commands/llm_api.rs"
  if ( check_local_api_key_cache_contract "$tmp/local-api-key-cache-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected local API key cache fallback gate to fail"
  fi
  mkdir -p "$tmp/runtime-trust-revoke-legacy/src/daemon/invocation/admission" \
    "$tmp/runtime-trust-revoke-legacy/src/daemon/invocation/dispatch"
  printf '%s\n' \
    'pub(crate) struct RuntimeTrustConnectionStateProjector;' \
    'impl RuntimeTrustConnectionStateProjector {' \
    '  pub(crate) fn from_local_credentials(source: impl Into<String>) -> Option<Self> {' \
    '    let credentials = crate::daemon::persistence::config::load_credentials().ok()?;' \
    '    Self::from_credentials(credentials, source)' \
    '  }' \
    '  pub(crate) fn from_credentials(credentials: Credentials, source: impl Into<String>) -> Option<Self> {' \
    '    let current_user_ura = credentials.user_ura().ok()?;' \
    '    Some(Self)' \
    '  }' \
    '}' \
    > "$tmp/runtime-trust-revoke-legacy/src/daemon/invocation/admission/runtime_trust_invalidator.rs"
  printf '%s\n' \
    'pub(crate) fn dispatch_revoke_user_pubkey(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {' \
    '  let outcome = handle_revoke_user_pubkey_with_outcome(arguments, &ctx.daemon_realm, &ctx.trust_anchor_path, &ctx.cell)?;' \
    '  RuntimeTrustInvalidator::new(self.directory.presence.clone(), self.directory.advertised_agents.clone())' \
    '    .with_connection_state_projector(RuntimeTrustConnectionStateProjector::from_local_credentials("daemon.runtime_trust"));' \
    '  Ok(outcome.body)' \
    '}' \
    > "$tmp/runtime-trust-revoke-legacy/src/daemon/invocation/dispatch/unary_dispatcher.rs"
  if ( check_runtime_trust_revoke_credentials_contract "$tmp/runtime-trust-revoke-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected runtime trust revoke credential fallback gate to fail"
  fi
  mkdir -p "$tmp/runtime-trust-user-key-legacy/src/daemon/invocation/admission" \
    "$tmp/runtime-trust-user-key-legacy/src/daemon/ability/catalog" \
    "$tmp/runtime-trust-user-key-legacy/src/cli/commands"
  cat >"$tmp/runtime-trust-user-key-legacy/src/daemon/invocation/admission/list_user_pubkeys.rs" <<'EOF'
use serde::Deserialize;
#[derive(Debug, Deserialize)]
struct ListArgs {
    agent_ura: String,
}
pub struct ListResponse {
    pub agent_ura: String,
}
pub(crate) fn handle() {
    let snapshot = runtime_trust.user_snapshot(&args.agent_ura);
}
#[cfg(test)]
mod tests {}
EOF
  cat >"$tmp/runtime-trust-user-key-legacy/src/daemon/invocation/admission/runtime_trust.rs" <<'EOF'
pub(crate) struct RuntimeTrustReader;
impl RuntimeTrustReader {
    pub(crate) fn user_snapshot(&self, agent_ura: &str) -> RuntimeTrustUserSnapshot {
        RuntimeTrustUserSnapshot { agent_ura: agent_ura.to_string() }
    }
}
pub(crate) struct RuntimeTrustUserSnapshot {
    pub(crate) agent_ura: String,
}
EOF
  cat >"$tmp/runtime-trust-user-key-legacy/src/daemon/ability/catalog/daemon_invocation_contracts.rs" <<'EOF'
fn schema() {
    ABILITY_IDENTITY_LIST_USER_PUBKEYS => object_schema(
        json!({"agent_ura": string_prop("User URA whose trusted keys should be listed.")}),
        &["agent_ura"],
        false,
    )
}
EOF
  printf 'fn contains(user_ura: &str) { invoke_local_ability("identity.list_user_pubkeys", serde_json::json!({ "agent_ura": user_ura })); }\n' \
    > "$tmp/runtime-trust-user-key-legacy/src/cli/commands/user_signing_identity.rs"
  printf 'fn check(user_ura: &str) { invoke_local_ability("identity.list_user_pubkeys", serde_json::json!({ "agent_ura": user_ura })); }\n' \
    > "$tmp/runtime-trust-user-key-legacy/src/cli/commands/doctor.rs"
  if ( check_runtime_trust_user_key_inventory_scope_contract "$tmp/runtime-trust-user-key-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected runtime trust user-key inventory agent_ura gate to fail"
  fi
  mkdir -p "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission" \
    "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/dispatch" \
    "$tmp/runtime-trust-user-key-write-legacy/src/daemon/ability/catalog" \
    "$tmp/runtime-trust-user-key-write-legacy/src/cli/commands" \
    "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/bidi/session_initiator"
  printf '%s\n' \
    '#[derive(Debug, Deserialize)]' \
    'struct RegisterArgs { agent_ura: String, public_key_b64: String, role: String }' \
    'pub(crate) struct RegisterPubkeyIntent { agent_ura: String, role: TrustedAgentRole }' \
    'impl RegisterPubkeyIntent { pub(crate) fn agent_ura(&self) -> &str { &self.agent_ura } }' \
    'fn decode_register_args(args: &[u8]) { let args: RegisterArgs = serde_json::from_slice(args).unwrap(); if args.agent_ura.is_empty() { panic!("identity.register_pubkey: agent_ura is required"); } }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission/register_device_pubkey.rs"
  printf '%s\n' \
    '#[derive(Debug, Deserialize)]' \
    'struct RevokeArgs { agent_ura: String, public_key_b64: String }' \
    'pub(crate) struct RevokeUserPubkeyIntent { agent_ura: String, public_key_b64: String }' \
    'impl RevokeUserPubkeyIntent { pub(crate) fn agent_ura(&self) -> &str { &self.agent_ura } }' \
    'fn parse_revoke_user_pubkey_intent(args: &[u8]) { let args: RevokeArgs = serde_json::from_slice(args).unwrap(); if args.agent_ura.is_empty() { panic!("identity.revoke_user_pubkey: agent_ura is required"); } }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission/revoke_user_pubkey.rs"
  printf '%s\n' \
    'pub(crate) fn register_pubkey(agent_ura: String) {}' \
    'pub(crate) fn revoke_user_pubkey(agent_ura: &str) {}' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission/runtime_trust.rs"
  printf '%s\n' \
    'fn gate(intent: &RegisterPubkeyIntent, revoke: &RevokeUserPubkeyIntent) { let _ = intent.agent_ura(); let _ = revoke.agent_ura(); }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission/identity_write_gate.rs"
  printf '%s\n' \
    'fn dispatch(intent: &RevokeUserPubkeyIntent) { invalidate_revoked_subject(intent.agent_ura(), None, true); }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/dispatch/unary_dispatcher.rs"
  printf '%s\n' \
    'ABILITY_IDENTITY_REGISTER_PUBKEY => object_schema(json!({"agent_ura": string_prop("Agent, Device, User, or Hub URA to trust."), "public_key_b64": string_prop("key"), "role": string_prop("role")}), &["agent_ura", "public_key_b64", "role"], false),' \
    'ABILITY_IDENTITY_REVOKE_USER_PUBKEY => object_schema(json!({"agent_ura": string_prop("User URA whose key row should be revoked."), "public_key_b64": string_prop("key")}), &["agent_ura", "public_key_b64"], false),' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/ability/catalog/daemon_invocation_contracts.rs"
  printf '%s\n' \
    'fn register(user_ura: &str) { invoke_local_ability("identity.register_pubkey", serde_json::json!({ "agent_ura": user_ura })); }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/cli/commands/user_signing_identity.rs"
  printf '%s\n' \
    'fn publish(user_ura: &str) { invoke_prelude_unary(client, request, "identity.register_pubkey"); let _ = serde_json::json!({ "agent_ura": user_ura }); }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  printf '%s\n' \
    'fn import_caller_trust(caller_ura: &str) { let register_args = serde_json::to_vec(&serde_json::json!({ "agent_ura": caller_ura, "public_key_b64": "k", "role": "device" })).unwrap(); }' \
    > "$tmp/runtime-trust-user-key-write-legacy/src/daemon/invocation/admission/device_trust_sync.rs"
  if ( check_runtime_trust_user_key_write_scope_contract "$tmp/runtime-trust-user-key-write-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected runtime trust user-key write scope gate to fail"
  fi
  mkdir -p "$tmp/device-trust-sync-caller-classification-legacy/src/daemon/invocation/admission"
  cat >"$tmp/device-trust-sync-caller-classification-legacy/src/daemon/invocation/admission/device_trust_sync.rs" <<'EOF'
enum DeviceTrustSyncStatus {
    NotSyncable,
}
enum SyncableCaller {
    Device,
}
impl DeviceTrustSync {
    fn syncable_caller(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> Option<SyncableCaller> {
        let parsed = crate::core::ura::parse_ura(caller_ura).ok()?;
        match parsed.kind {
            crate::core::ura::URAKind::Device => Some(SyncableCaller::Device),
            _ => None,
        }
    }
}
EOF
  if ( check_device_trust_sync_caller_classification_contract "$tmp/device-trust-sync-caller-classification-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected device trust sync caller classification fallback gate to fail"
  fi
  mkdir -p "$tmp/product-e2e-history-fallback/tools/scripts"
  printf '%s\n' \
    'provider_cli "invocation list --ability-ura '\''$ADD_URA'\'' --format json" >"$OUT_DIR/provider-invocation-list-add-after-cli.json"' \
    'provider_cli "invocation list --format json" >"$OUT_DIR/provider-invocation-list-all-after-cli.json"' \
    'def invocation_records(name: str): return []' \
    'def ability_invocation_records(exact_name: str, fallback_name: str, ability_ura: str):' \
    '    exact = invocation_records(exact_name)' \
    '    if exact:' \
    '        return exact' \
    '    return invocation_records(fallback_name)' \
    'all_invocation_records = ability_invocation_records("provider-invocation-list-add-after-cli.json", "provider-invocation-list-all-after-cli.json", ability_ura)' \
    > "$tmp/product-e2e-history-fallback/tools/scripts/docker-two-node-easyremote-cli-e2e.sh"
  if ( check_product_e2e_invocation_history_exact_scope_contract "$tmp/product-e2e-history-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected product e2e invocation history fallback gate to fail"
  fi
  mkdir -p "$tmp/observe-health-legacy/src/daemon/ability/builtins/governance"
  cat >"$tmp/observe-health-legacy/src/daemon/ability/builtins/governance/health.rs" <<'EOF'
fn handler(args: Value) -> anyhow::Result<Value> {
  let ts = chrono::Utc::now().timestamp_millis();
  Ok(json!({"status": "healthy", "details": {"replied_at_unix_ms": ts}, "components": {}, "echo": args, "replied_at_unix_ms": ts}))
}
pub fn description() -> &'static str {
  "Local health probe. Returns Axon observe.health status fields plus smoke diagnostics."
}
EOF
  if ( check_observe_health_contract_projection_contract "$tmp/observe-health-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected observe.health legacy diagnostics gate to fail"
  fi
  mkdir -p "$tmp/admission-owner-legacy/src/daemon/invocation/admission"
  printf '%s\n' \
    'pub(crate) fn resolve_owner(subject_ura: &str, callee_ura: &str, daemon_ura: Option<&str>, trust_anchor: &RealmTrustAnchor) -> OwnerResolution {' \
    '  OwnerResolver::resolve(&OwnerResolutionInput {' \
    '    subject: owner_fact_from_ura(subject_ura, daemon_ura, trust_anchor),' \
    '    callee: owner_fact_from_ura(callee_ura, daemon_ura, trust_anchor),' \
    '    device: owner_fact_from_trust_anchor(callee_ura, trust_anchor).or_else(|| owner_fact_from_local_device(callee_ura, daemon_ura)),' \
    '    session: None,' \
    '  })' \
    '}' \
    'fn owner_fact_from_local_device(ura: &str, daemon_ura: Option<&str>) -> Option<OwnerFact> {' \
    '  let parsed = parse_ura(ura).ok()?;' \
    '  let credentials = crate::daemon::persistence::config::load_credentials().ok()?;' \
    '  let user_id = credentials.user_id().ok()?.to_string();' \
    '  Some(OwnerFact::user(user_id.clone(), crate::core::ura::user_ura(&credentials.realm, &user_id)))' \
    '}' \
    > "$tmp/admission-owner-legacy/src/daemon/invocation/admission/policy_gate.rs"
  if ( check_admission_owner_credentials_contract "$tmp/admission-owner-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected admission owner credential fallback gate to fail"
  fi
  mkdir -p "$tmp/shared-local-owner-legacy/src/daemon/invocation/admission"
  printf '%s\n' \
    'pub(crate) fn local_device_owner_fact(ura: &str) -> Option<OwnerFact> {' \
    '  let parsed = parse_ura(ura).ok()?;' \
    '  if parsed.kind != URAKind::Device { return None; }' \
    '  let credentials = config::load_credentials().ok()?;' \
    '  let owner_user_id = credentials.user_id().ok()?.to_string();' \
    '  Some(OwnerFact::user(owner_user_id.clone(), user_ura(&credentials.realm, &owner_user_id)))' \
    '}' \
    > "$tmp/shared-local-owner-legacy/src/daemon/invocation/admission/owner_resolution.rs"
  printf '%s\n' \
    'pub(crate) fn principal_for(role: TrustedAgentRole, caller_ura: &str, trust_anchor: &RealmTrustAnchor) -> PrincipalProjection {' \
    '  let owner_user_id = trust_anchor.lookup_principal_owner(caller_ura).map(|owner| OwnerFact::user(owner.owner_user_id.clone(), owner.owner_ura.clone())).or_else(|| local_device_owner_fact(caller_ura)).and_then(|owner| owner.owner_user_id);' \
    '  PrincipalProjection { caller_user_id: owner_user_id }' \
    '}' \
    > "$tmp/shared-local-owner-legacy/src/daemon/invocation/admission/policy_gate.rs"
  printf '%s\n' \
    'pub(crate) enum BootstrapAuthorityDecision { Verified { authority_id: String }, NotApplicable }' \
    'fn verify() -> BootstrapAuthorityDecision {' \
    '  let owner = trust_anchor.lookup_principal_owner(caller_ura).map(|owner| OwnerFact::user(owner.owner_user_id.clone(), owner.owner_ura.clone())).or_else(|| local_device_owner_fact(caller_ura));' \
    '  BootstrapAuthorityDecision::NotApplicable' \
    '}' \
    > "$tmp/shared-local-owner-legacy/src/daemon/invocation/admission/bootstrap_authority.rs"
  printf '%s\n' \
    'fn map(decision: BootstrapAuthorityDecision) -> Option<String> {' \
    '  match decision { BootstrapAuthorityDecision::Verified { authority_id } => Some(authority_id), BootstrapAuthorityDecision::NotApplicable => None }' \
    '}' \
    > "$tmp/shared-local-owner-legacy/src/daemon/invocation/admission/admission_facade.rs"
  if ( check_shared_local_device_owner_projection_contract "$tmp/shared-local-owner-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected shared local device owner fallback gate to fail"
  fi
  mkdir -p "$tmp/node-session-authority-legacy/sdk/node/test"
  printf '%s\n' \
    'function validateSessionAuthority(authority) {' \
    '  rejectAllZeroAuthorityFields({ session_owner_user_id: authority.sessionOwnerUserID, subject_ura: authority.subjectURA });' \
    '  if (authority.signature.length === 0) throw invalidAuthority("session authority signature is required");' \
    '}' \
    'function validateSessionAuthorityRequest(request) {' \
    '  rejectAllZeroAuthorityFields({ session_owner_user_id: request.sessionOwnerUserID, subject_ura: request.subjectURA });' \
    '  rejectAuthorityPrivateKeyMetadata(request.metadata);' \
    '}' \
    'function sessionAuthorityAdmitsSubject(authority, subjectURA) { return authority.subjectURA.trim() === subjectURA.trim(); }' \
    > "$tmp/node-session-authority-legacy/sdk/node/index.js"
  printf '%s\n' \
    'test("authority metadata rejects all-zero session owners", () => {});' \
    > "$tmp/node-session-authority-legacy/sdk/node/test/runtime-core.test.mjs"
  if ( check_node_session_authority_subject_contract "$tmp/node-session-authority-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected Node session authority subject-binding gate to fail"
  fi
  mkdir -p "$tmp/admission-authority-raw-default-legacy/src/daemon/invocation/admission"
  cat >"$tmp/admission-authority-raw-default-legacy/src/daemon/invocation/admission/admission_facade.rs" <<'EOF'
#[derive(Debug, Deserialize)]
struct DelegationProofRaw {
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct SessionAuthorityRaw {
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    signature: String,
}

fn parse_and_verify_delegation_proof() {}
fn parse_and_verify_session_authority() {}
fn admission_authority_raw_wire_requires_payload_and_signature() {}
EOF
  if ( check_admission_authority_raw_wire_strict_contract "$tmp/admission-authority-raw-default-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected admission authority raw wire default gate to fail"
  fi
  mkdir -p "$tmp/admission-authority-ability-projection-legacy/src/daemon/invocation/admission"
  cat >"$tmp/admission-authority-ability-projection-legacy/src/daemon/invocation/admission/admission_facade.rs" <<'EOF'
impl AuthorityAbilityView {
    fn from_envelope(envelope: &Envelope, ability: &str) -> Result<Self, Status> {
        let callee_ura = envelope.callee.as_ref().unwrap().ura.as_str();
        let wire = ability.trim();
        let ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, wire)
                .map_err(axon_error_to_status)?;
        let public_name = AbilitySelector::parse(&ability_ura)
            .map(|selector| selector.public_name().to_string())
            .unwrap_or_else(|_| owner_local_ability_name(callee_ura, wire));
        Ok(Self { wire: wire.to_string(), public_name, ability_ura })
    }
}
EOF
  if ( check_admission_authority_ability_projection_contract "$tmp/admission-authority-ability-projection-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected admission authority ability projection fallback gate to fail"
  fi
  mkdir -p "$tmp/peer-envelope-subject-profile-legacy/src/daemon/invocation/admission"
  cat >"$tmp/peer-envelope-subject-profile-legacy/src/daemon/invocation/admission/peer_envelope_signer.rs" <<'EOF'
pub(crate) async fn sign_peer_request_envelope(
    envelope: &mut Envelope,
    ability: &str,
    descriptor_ref: &str,
    arguments: &[u8],
    local_realm: Option<&str>,
    hub_signer: Option<&dyn CanonicalSigner>,
) -> Result<String, Status> {
    let descriptor_subject_ura = descriptor_subject_ura_for("callee", "subject", ability)?;
    let profile = envelope
        .subject
        .as_ref()
        .map(|subject| subject.profile.clone())
        .filter(|profile| !profile.trim().is_empty())
        .unwrap_or_else(|| crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string());
    envelope.subject = Some(SubjectIdentity { ura: descriptor_subject_ura, profile });
    Ok(String::new())
}
EOF
  if ( check_peer_envelope_signer_subject_profile_contract "$tmp/peer-envelope-subject-profile-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected peer envelope signer subject profile fallback gate to fail"
  fi
  mkdir -p "$tmp/session-prelude-credentials-legacy/src/daemon/invocation/bidi/session_initiator"
  printf '%s\n' \
    'async fn sync_paired_user_trust_prelude(client: &mut InvocationClient<Channel>, signer: &dyn CanonicalSigner, sync: &UserTrustSync) -> Result<UserTrustBootstrapOutcome, UserTrustBootstrapError> {' \
    '  let Ok(creds) = crate::daemon::persistence::config::load_credentials() else {' \
    '    return Ok(UserTrustBootstrapOutcome::NotRequired);' \
    '  };' \
    '  let Ok(user_ura) = creds.user_ura() else {' \
    '    return Ok(UserTrustBootstrapOutcome::NotRequired);' \
    '  };' \
    '  Ok(UserTrustBootstrapOutcome::NotRequired)' \
    '}' \
    'fn resolved_public_keys(result: &[u8]) -> anyhow::Result<Vec<String>> { Ok(Vec::new()) }' \
    > "$tmp/session-prelude-credentials-legacy/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  if ( check_session_prelude_credentials_contract "$tmp/session-prelude-credentials-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected session prelude credential fallback gate to fail"
  fi
  mkdir -p "$tmp/session-prelude-hosted-owner-legacy/src/daemon/invocation/bidi/session_initiator"
  cat >"$tmp/session-prelude-hosted-owner-legacy/src/daemon/invocation/bidi/session_initiator/prelude.rs" <<'EOF'
async fn sync_paired_user_trust_prelude(client: &mut InvocationClient<Channel>, signer: &dyn CanonicalSigner, sync: &UserTrustSync) -> Result<UserTrustBootstrapOutcome, UserTrustBootstrapError> {
  let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()
    .map_err(|error| UserTrustBootstrapError::CredentialsUnavailable { message: format!("load paired credentials: {error}") })? else {
    return Ok(UserTrustBootstrapOutcome::NotRequired);
  };
  let user_ura = creds.user_ura()
    .map_err(|error| UserTrustBootstrapError::CredentialsUnavailable { message: format!("project paired user URA: {error}") })?;
  return Ok(UserTrustBootstrapOutcome::NotRequired);
}
fn resolved_public_keys(result: &[u8]) -> anyhow::Result<Vec<String>> { Ok(Vec::new()) }
async fn run_hosted_agent_advertise_prelude(client: &mut InvocationClient<Channel>, phase: &mut SessionPhaseTracker, hub_endpoint: &str, signer: &dyn CanonicalSigner, ability_descriptors: &[AbilityDescriptor]) -> Result<(), SessionError> {
  let user_segment = crate::daemon::persistence::config::load_credentials()
    .map_err(|error| SessionError::HostedAgentPreludeFailed { endpoint: hub_endpoint.to_string(), reason: format!("load credentials for hosted-agent owner projection: {error}") })?
    .username
    .filter(|value| !value.is_empty())
    .unwrap_or_default();
  Ok(())
}
async fn advertise_hosted_agent_entry() {}
#[cfg(test)]
mod tests {
  #[test] fn paired_user_trust_bootstrap_ignores_missing_credentials_only() {}
  #[test] fn paired_user_trust_bootstrap_rejects_malformed_credentials() {}
  #[test] fn hosted_agent_owner_segment_accepts_explicit_dev_override() {}
  #[test] fn hosted_agent_owner_segment_reads_valid_paired_credentials() {}
  #[test] fn hosted_agent_owner_segment_rejects_federation_native_credentials_without_username() {}
}
EOF
  if ( check_session_prelude_credentials_contract "$tmp/session-prelude-hosted-owner-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected hosted-agent prelude owner fallback gate to fail"
  fi
  mkdir -p "$tmp/start-attach-user-signer-legacy/src/daemon/control" \
    "$tmp/start-attach-user-signer-legacy/src/bin" \
    "$tmp/start-attach-user-signer-legacy/src/daemon/boot/lifecycle"
  printf '%s\n' \
    'pub mod flags {' \
    '  pub const BOOT_STATUS: &str = "boot_status";' \
    '  pub const CONTROL_DIAGNOSTICS: &str = "control_diagnostics";' \
    '}' \
    > "$tmp/start-attach-user-signer-legacy/src/daemon/control/discovery.rs"
  printf '%s\n' \
    'pub struct ControlRuntimeDiscovery { pub invocation_endpoint: std::path::PathBuf, pub daemon_identity: DaemonIdentity }' \
    'fn write_discovery_for(runtime: Option<ControlRuntimeDiscovery>) {' \
    '  let capability_flags = vec![flags::BOOT_STATUS.into(), flags::CONTROL_DIAGNOSTICS.into()];' \
    '}' \
    > "$tmp/start-attach-user-signer-legacy/src/daemon/control/server.rs"
  printf '%s\n' \
    'fn ready_runtime_discovery() -> anyhow::Result<server::ControlRuntimeDiscovery> {' \
    '  Ok(server::ControlRuntimeDiscovery { invocation_endpoint: resolved_local_uds_path_with_env_override(), daemon_identity: DaemonIdentity { mode: "device".into(), realm: "tenant".into(), node_id: None } })' \
    '}' \
    > "$tmp/start-attach-user-signer-legacy/src/bin/easynet-daemon.rs"
  printf '%s\n' \
    'impl DaemonDiscoverySnapshot {' \
    '  pub fn identity(&self) -> Option<&DaemonIdentity> { None }' \
    '}' \
    > "$tmp/start-attach-user-signer-legacy/src/daemon/boot/lifecycle/discovery.rs"
  printf '%s\n' \
    'pub enum RuntimeLifecycleError { StartRefusedIdentityMismatch, StartRefusedMissingDaemonIdentity }' \
    > "$tmp/start-attach-user-signer-legacy/src/daemon/boot/lifecycle/errors.rs"
  printf '%s\n' \
    'fn validate_attach_identity(request: &RuntimeStartRequest, report: &RuntimeStatusReport) -> Result<(), RuntimeLifecycleError> {' \
    '  let identity = report.daemon().identity().ok_or(RuntimeLifecycleError::StartRefusedMissingDaemonIdentity)?;' \
    '  validate_mode(request, identity)?; validate_realm(request, identity)?; validate_node_id(request, identity)?; Ok(())' \
    '}' \
    '#[cfg(test)] mod tests { fn start_preflight_attaches_when_projection_is_missing_but_daemon_is_live() {} }' \
    > "$tmp/start-attach-user-signer-legacy/src/daemon/boot/lifecycle/start.rs"
  if ( check_start_attach_user_signer_readiness_contract "$tmp/start-attach-user-signer-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected start attach user signer readiness gate to fail"
  fi
  mkdir -p "$tmp/start-ready-env-node-legacy/src/daemon/control" \
    "$tmp/start-ready-env-node-legacy/src/bin" \
    "$tmp/start-ready-env-node-legacy/src/daemon/boot/lifecycle"
  printf '%s\n' \
    'pub mod flags {' \
    '  pub const BOOT_STATUS: &str = "boot_status";' \
    '  pub const CONTROL_DIAGNOSTICS: &str = "control_diagnostics";' \
    '  pub const PAIRED_USER_RUNTIME_SIGNER: &str = "paired_user_runtime_signer";' \
    '}' \
    > "$tmp/start-ready-env-node-legacy/src/daemon/control/discovery.rs"
  printf '%s\n' \
    'pub struct ControlRuntimeDiscovery { pub invocation_endpoint: std::path::PathBuf, pub daemon_identity: DaemonIdentity, pub capability_flags: Vec<String> }' \
    'fn write_discovery_for(runtime: Option<ControlRuntimeDiscovery>) {' \
    '  let runtime = runtime.unwrap();' \
    '  let capability_flags = discovery_capability_flags(runtime.capability_flags);' \
    '}' \
    'fn discovery_capability_flags(runtime_flags: Vec<String>) -> Vec<String> {' \
    '  let _ = flags::BOOT_STATUS; let _ = flags::CONTROL_DIAGNOSTICS; runtime_flags' \
    '}' \
    > "$tmp/start-ready-env-node-legacy/src/daemon/control/server.rs"
  printf '%s\n' \
    'fn ready_runtime_discovery() -> anyhow::Result<server::ControlRuntimeDiscovery> {' \
    '  let config = DaemonConfig::load(&default_config_path())?;' \
    '  let node_id = std::env::var("EASYNET_NODE_ID").ok();' \
    '  let mut capability_flags = Vec::new();' \
    '  if matches!(config.mode(), DaemonMode::Device | DaemonMode::Both) {' \
    '    capability_flags.push(flags::PAIRED_USER_RUNTIME_SIGNER.to_string());' \
    '  }' \
    '  Ok(server::ControlRuntimeDiscovery { invocation_endpoint: resolved_local_uds_path_with_env_override(), daemon_identity: DaemonIdentity { mode: config.mode().as_str().to_string(), realm: config.realm().to_string(), node_id }, capability_flags })' \
    '}' \
    'fn ready_discovery_uses_paired_credentials_node_id_not_env() {}' \
    'fn ready_discovery_rejects_credentials_realm_mismatch() {}' \
    > "$tmp/start-ready-env-node-legacy/src/bin/easynet-daemon.rs"
  printf '%s\n' \
    'impl DaemonDiscoverySnapshot {' \
    '  pub fn identity(&self) -> Option<&DaemonIdentity> { None }' \
    '  pub fn has_capability_flag(&self, flag: &str) -> bool { false }' \
    '}' \
    > "$tmp/start-ready-env-node-legacy/src/daemon/boot/lifecycle/discovery.rs"
  printf '%s\n' \
    'pub enum RuntimeLifecycleError { StartRefusedIdentityMismatch, StartRefusedMissingDaemonIdentity, StartRefusedMissingRuntimeCapability }' \
    > "$tmp/start-ready-env-node-legacy/src/daemon/boot/lifecycle/errors.rs"
  printf '%s\n' \
    'fn validate_attach_capabilities(request: &RuntimeStartRequest, report: &RuntimeStatusReport) -> Result<(), RuntimeLifecycleError> {' \
    '  if report.daemon().has_capability_flag(crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER) { return Ok(()); }' \
    '  Err(RuntimeLifecycleError::StartRefusedMissingRuntimeCapability { mode: "device", capability: crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER })' \
    '}' \
    '#[cfg(test)] mod tests { fn start_preflight_refuses_device_attach_without_paired_user_signer_readiness() {} }' \
    > "$tmp/start-ready-env-node-legacy/src/daemon/boot/lifecycle/start.rs"
  if ( check_start_attach_user_signer_readiness_contract "$tmp/start-ready-env-node-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected ready discovery env-node fallback gate to fail"
  fi
  mkdir -p "$tmp/session-prelude-receipt-legacy/src/daemon/invocation/bidi/session_initiator"
  printf '%s\n' \
    'fn apply_federation_join_receipt(body_bytes: &[u8], hub_published_abilities: &HubPublishedAbilityStore) -> Result<(), tonic::Status> {' \
    '  if !body_bytes.is_empty() {' \
    '    if let Ok(body) = serde_json::from_slice::<crate::daemon::federation::client::ability_contract::JoinReceipt>(body_bytes) {' \
    '      hub_published_abilities.seed_from_snapshot(body.hub_abilities_revision, body.hub_published_abilities);' \
    '    }' \
    '  }' \
    '  Ok(())' \
    '}' \
    'fn federation_join_public_key_hex() {}' \
    > "$tmp/session-prelude-receipt-legacy/src/daemon/invocation/bidi/session_initiator/prelude.rs"
  printf '%s\n' \
    'fn apply_federation_heartbeat_receipt(body_bytes: &[u8], hub_published_abilities: &HubPublishedAbilityStore) -> Result<(), tonic::Status> {' \
    '  if let Ok(receipt) = serde_json::from_slice::<crate::daemon::federation::client::ability_contract::HeartbeatReceipt>(body_bytes) {' \
    '    let diff = receipt.hub_abilities_diff;' \
    '    if !diff.added.is_empty() || !diff.removed.is_empty() { hub_published_abilities.apply_diff(diff); }' \
    '  }' \
    '  Ok(())' \
    '}' \
    '#[cfg(test)] mod tests {}' \
    > "$tmp/session-prelude-receipt-legacy/src/daemon/invocation/bidi/session_initiator/heartbeat.rs"
  if ( check_session_prelude_receipt_contract "$tmp/session-prelude-receipt-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected session prelude receipt fallback gate to fail"
  fi
  mkdir -p "$tmp/device-settings-legacy/src/daemon/persistence" \
    "$tmp/device-settings-legacy/src/cli/commands"
  printf '%s\n' \
    '#[derive(Debug, Clone, Serialize, Deserialize, Default)]' \
    'pub struct DeviceSettings { pub session_bridge_exec_enabled: bool }' \
    'pub fn load_or_create_install_id() -> anyhow::Result<String> {' \
    '  let mut settings = load_device_settings();' \
    '  Ok(String::new())' \
    '}' \
    'pub fn load_device_settings() -> DeviceSettings {' \
    '  let path = device_settings_path();' \
    '  fs::read_to_string(&path).ok().and_then(|data| serde_json::from_str(&data).ok()).unwrap_or_default()' \
    '}' \
    '' \
    'pub fn save_device_settings(settings: &DeviceSettings) -> anyhow::Result<()> { Ok(()) }' \
    > "$tmp/device-settings-legacy/src/daemon/persistence/config.rs"
  printf 'fn run() { let settings = config::load_device_settings(); }\n' \
    > "$tmp/device-settings-legacy/src/cli/commands/config_cmd.rs"
  if ( check_device_settings_loader_contract "$tmp/device-settings-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected device settings default fallback gate to fail"
  fi
  mkdir -p "$tmp/mission-implicit-fallback/src/daemon/execution/mission" \
    "$tmp/mission-implicit-fallback/src/eal/parser" \
    "$tmp/mission-implicit-fallback/src/eal/runtime"
  printf '%s\n' \
    'struct ImplicitAgentFallback;' \
    'fn find_implicit_agent_fallback(ir: &MissionIr) -> anyhow::Result<Option<ImplicitAgentFallback>> {' \
    '  let snapshot = AgentAggregateRepository::load_snapshot()?;' \
    '  Ok(None)' \
    '}' \
    > "$tmp/mission-implicit-fallback/src/daemon/execution/mission/orchestration.rs"
  printf 'No implicit agent fallback is allowed.\n' \
    > "$tmp/mission-implicit-fallback/src/eal/parser/mod.rs"
  printf 'No implicit agent fallback is allowed.\n' \
    > "$tmp/mission-implicit-fallback/src/eal/runtime/ir.rs"
  if ( check_mission_traditional_target_conflict_contract "$tmp/mission-implicit-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected Mission implicit fallback naming gate to fail"
  fi
  mkdir -p "$tmp/mission-meta-identity-legacy/src/daemon/execution/mission"
  printf '%s\n' \
    '#[derive(Debug, Clone, Default, Serialize, Deserialize)]' \
    'pub struct MissionRunMeta {' \
    '  #[serde(default)]' \
    '  pub trace_id: String,' \
    '}' \
    'fn pre_trace_id_meta_still_deserializes() {}' \
    > "$tmp/mission-meta-identity-legacy/src/daemon/execution/mission/orchestration.rs"
  printf '%s\n' \
    '#[derive(Debug, Clone, Default, Serialize, Deserialize)]' \
    'pub struct RunMeta {' \
    '  #[serde(default, skip_serializing_if = "String::is_empty")]' \
    '  pub invocation_id: String,' \
    '}' \
    > "$tmp/mission-meta-identity-legacy/src/daemon/execution/mission/run_store.rs"
  printf '%s\n' \
    'pub(crate) fn deserialize_non_empty_string() {}' \
    'fn required_identity_rejects_empty_string() {}' \
    'const MESSAGE: &str = "runtime identity fact must be a non-empty string";' \
    > "$tmp/mission-meta-identity-legacy/src/daemon/execution/mission/persisted_identity.rs"
  if ( check_mission_runtime_meta_identity_schema_contract "$tmp/mission-meta-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected mission runtime meta identity legacy default gate to fail"
  fi
  mkdir -p "$tmp/sdk-runtime-failure-legacy/sdk/go" \
    "$tmp/sdk-runtime-failure-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-runtime-failure-legacy/sdk/python/tests"
  printf '%s\n' \
    'func runtimeFailureCode(code string, fallback ErrorCode) ErrorCode {' \
    '  if code == "" { return fallback }' \
    '  return ErrProtocolMismatch' \
    '}' \
    > "$tmp/sdk-runtime-failure-legacy/sdk/go/errors.go"
  printf '%s\n' \
    'func directAxonFailure(errorValue *axonpb.Error, stage string) map[string]any {' \
    '  code := runtimeFailureCode(errorValue.GetCode(), ErrAdmissionDenied)' \
    '  if code == "" || code == ErrGeneric { code = ErrAdmissionDenied }' \
    '  return nil' \
    '}' \
    'func directErrorStage() {}' \
    > "$tmp/sdk-runtime-failure-legacy/sdk/go/direct_runtime.go"
  printf '%s\n' \
    'def canonical_failure_code(code=None):' \
    '    if code:' \
    '        return ErrorCode.PROTOCOL_MISMATCH' \
    '    return ErrorCode.ADMISSION_DENIED' \
    '' \
    'def canonical_terminal_state_code(state): pass' \
    > "$tmp/sdk-runtime-failure-legacy/sdk/python/easynet_sdk/errors.py"
  printf '%s\n' \
    'def _response_error_code(code):' \
    '    if code:' \
    '        return canonical_failure_code(code)' \
    '    return ErrorCode.ADMISSION_DENIED' \
    '' \
    'def _failure_code_value(code): pass' \
    > "$tmp/sdk-runtime-failure-legacy/sdk/python/easynet_sdk/direct_runtime.py"
  touch "$tmp/sdk-runtime-failure-legacy/sdk/go/errors_test.go" \
    "$tmp/sdk-runtime-failure-legacy/sdk/go/direct_runtime_codec_test.go" \
    "$tmp/sdk-runtime-failure-legacy/sdk/python/tests/test_errors.py" \
    "$tmp/sdk-runtime-failure-legacy/sdk/python/tests/test_direct_runtime.py"
  if ( check_sdk_runtime_failure_code_contract "$tmp/sdk-runtime-failure-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK runtime failure code fallback gate to fail"
  fi
  mkdir -p "$tmp/sdk-direct-runtime-state-fallback/sdk/go" \
    "$tmp/sdk-direct-runtime-state-fallback/sdk/python/easynet_sdk" \
    "$tmp/sdk-direct-runtime-state-fallback/sdk/python/tests"
  printf '%s\n' \
    'func directStateName(state axonpb.InvocationState) string {' \
    '  switch state {' \
    '  case axonpb.InvocationState_INVOCATION_STATE_COMPLETED:' \
    '    return "Completed"' \
    '  default:' \
    '    return "Unspecified"' \
    '  }' \
    '}' \
    'func directInvokeResponseJSON(response *axonpb.InvokeResponse) { stateName := directStateName(response.GetState()); _ = stateName }' \
    'func directStreamChunkJSON(chunk *axonpb.InvokeStreamChunk) { stateName := directStateName(chunk.GetState()); _ = stateName }' \
    'func directReceipt(receipt *axonpb.InvocationReceipt) { stateName := directStateName(receipt.GetState()); _ = stateName }' \
    > "$tmp/sdk-direct-runtime-state-fallback/sdk/go/direct_runtime.go"
  printf 'func TestDirectRuntimeUnaryProjectsUnspecifiedInvocationState(t *testing.T) {}\n' \
    > "$tmp/sdk-direct-runtime-state-fallback/sdk/go/direct_runtime_codec_test.go"
  printf '%s\n' \
    'def _state_name(value: int) -> str:' \
    '    return {5: "Completed"}.get(value, "Unspecified")' \
    '' \
    'def _invoke_response_json(response):' \
    '    terminal_state = _state_name(response.state)' \
    '' \
    'def _stream_chunk_json(chunk):' \
    '    return {"state": _state_name(chunk.state)}' \
    '' \
    'def _canonical_receipt_projection(receipt):' \
    '    return {"state": _state_name(receipt.state)}' \
    > "$tmp/sdk-direct-runtime-state-fallback/sdk/python/easynet_sdk/direct_runtime.py"
  printf 'def test_direct_runtime_projects_unspecified_invocation_state(): pass\n' \
    > "$tmp/sdk-direct-runtime-state-fallback/sdk/python/tests/test_direct_runtime.py"
  if ( check_sdk_direct_runtime_state_projection_contract "$tmp/sdk-direct-runtime-state-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected SDK direct runtime state fallback gate to fail"
  fi
  mkdir -p "$tmp/sdk-runtime-stage-fallback/sdk/go" \
    "$tmp/sdk-runtime-stage-fallback/sdk/python/easynet_sdk" \
    "$tmp/sdk-runtime-stage-fallback/sdk/python/tests"
  printf '%s\n' \
    'func runtimeFailureCode(code string) ErrorCode {' \
    '  if code == "" { return ErrProtocolMismatch }' \
    '  return ErrProtocolMismatch' \
    '}' \
    'func isCanonicalExtensionErrorCode(code string) bool { return false }' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/go/errors.go"
  printf '%s\n' \
    'func directAxonFailure(errorValue *axonpb.Error, stage string) map[string]any {' \
    '  code := runtimeFailureCode(errorValue.GetCode())' \
    '  return map[string]any{"code": string(code), "stage": stage}' \
    '}' \
    'func directErrorStage(stage axonpb.ErrorStage, fallback string) string {' \
    '  return fallback' \
    '}' \
    'func directResponseFailure(errorValue *axonpb.Error, terminalState string, stage string) map[string]any {' \
    '  return directAxonFailure(errorValue, directErrorStage(errorValue.GetStage(), stage))' \
    '}' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/go/direct_runtime.go"
  printf '%s\n' \
    'func TestRuntimeFailureCodePreservesDomainCodesAndRejectsLegacyAliases(t *testing.T) {' \
    '  _ = ErrProtocolMismatch' \
    '}' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/go/errors_test.go"
  printf '%s\n' \
    'func TestDirectAxonFailureProjectsMissingErrorCodeToProtocolMismatch(t *testing.T) {}' \
    'func TestDirectErrorStageUsesCanonicalProviderProjection(t *testing.T) {}' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/go/direct_runtime_codec_test.go"
  printf '%s\n' \
    'def canonical_failure_code(code=None):' \
    '    return ErrorCode.PROTOCOL_MISMATCH' \
    '' \
    'def canonical_terminal_state_code(state): pass' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/python/easynet_sdk/errors.py"
  printf '%s\n' \
    'def _response_error_code(code):' \
    '    return canonical_failure_code(code)' \
    '' \
    'def _failure_code_value(code): pass' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/python/easynet_sdk/direct_runtime.py"
  printf '%s\n' \
    'def test_runtime_failure_code_preserves_domain_codes_and_rejects_legacy_aliases(): pass' \
    '"   ": ErrorCode.PROTOCOL_MISMATCH' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/python/tests/test_errors.py"
  printf '%s\n' \
    'self.assertEqual(_response_error_code(""), ErrorCode.PROTOCOL_MISMATCH)' \
    > "$tmp/sdk-runtime-stage-fallback/sdk/python/tests/test_direct_runtime.py"
  if ( check_sdk_runtime_failure_code_contract "$tmp/sdk-runtime-stage-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected SDK runtime error stage fallback gate to fail"
  fi
  mkdir -p "$tmp/sdk-direct-runtime-not-found-legacy/sdk/go" \
    "$tmp/sdk-direct-runtime-not-found-legacy/sdk/python/easynet_sdk" \
    "$tmp/sdk-direct-runtime-not-found-legacy/sdk/python/tests"
  printf '%s\n' \
    'func directRuntimeGRPCError() error {' \
    '  switch statusValue.Code() {' \
    '  case codes.NotFound:' \
    '    code, retry, retryable = ErrAbilityNotFound, RetryNever, false' \
    '  }' \
    '  return nil' \
    '}' \
    'func directRuntimeError() {}' \
    > "$tmp/sdk-direct-runtime-not-found-legacy/sdk/go/direct_runtime.go"
  printf 'func TestDirectRuntimeGRPCErrorProjectsProviderNotFoundAsDescriptorNotFound(t *testing.T) {}\n' \
    > "$tmp/sdk-direct-runtime-not-found-legacy/sdk/go/direct_runtime_test.go"
  printf '%s\n' \
    'def _grpc_error(error, *, endpoint):' \
    '    mapping = {' \
    '        grpc.StatusCode.NOT_FOUND: (ErrorCode.ABILITY_NOT_FOUND, RetryHint.NEVER, False),' \
    '    }' \
    '    return mapping' \
    '' \
    'def _direct_error(): pass' \
    > "$tmp/sdk-direct-runtime-not-found-legacy/sdk/python/easynet_sdk/direct_runtime.py"
  printf 'def test_direct_runtime_grpc_not_found_projects_descriptor_not_found(): pass\n' \
    > "$tmp/sdk-direct-runtime-not-found-legacy/sdk/python/tests/test_direct_runtime.py"
  if ( check_sdk_direct_runtime_descriptor_not_found_contract "$tmp/sdk-direct-runtime-not-found-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected SDK direct runtime NOT_FOUND ability projection gate to fail"
  fi
  mkdir -p "$tmp/sdk-root-product-named/sdk/go" \
    "$tmp/sdk-root-product-named/sdk/python/easynet_sdk"
  printf '// Package easynet provides the Go binding for the canonical EasyNet runtime SDK.\npackage easynet\n' \
    > "$tmp/sdk-root-product-named/sdk/go/doc.go"
  printf '"""Product-neutral EasyNet runtime SDK."""\n' \
    > "$tmp/sdk-root-product-named/sdk/python/easynet_sdk/__init__.py"
  if ( check_sdk_root_runtime_description_contract "$tmp/sdk-root-product-named" ) >/dev/null 2>&1; then
    fail "self-test expected SDK root product-named runtime description gate to fail"
  fi
  mkdir -p "$tmp/cli-mcp-local-bridge/src/daemon/ability/builtins/integrations/mcp" \
    "$tmp/cli-mcp-local-bridge/src/support/async_bridge"
  cat >"$tmp/cli-mcp-local-bridge/src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs" <<'EOF'
fn run_blocking<F: std::future::Future<Output = T>, T>(fut: F) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(_) => tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut),
    }
}
EOF
  printf 'pub fn try_run_blocking() {}\n' \
    > "$tmp/cli-mcp-local-bridge/src/support/async_bridge/mod.rs"
  if ( check_mcp_reflection_async_bridge_contract "$tmp/cli-mcp-local-bridge" ) >/dev/null 2>&1; then
    fail "self-test expected MCP local async bridge gate to fail"
  fi
  mkdir -p "$tmp/cli-credentials-ok-fallback/src/cli/presentation"
  printf '%s\n' \
    'fn write_runtime_status() {' \
    '  let creds = crate::daemon::persistence::config::load_credentials().ok();' \
    '}' \
    > "$tmp/cli-credentials-ok-fallback/src/cli/presentation/banner.rs"
  if ( check_cli_credentials_optional_read_contract "$tmp/cli-credentials-ok-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected credentials .ok fallback gate to fail"
  fi
  mkdir -p "$tmp/cli-user-ura-ok-fallback/src/cli/commands"
  printf '%s\n' \
    'fn render_pairing(creds: Credentials) {' \
    '  let user_ura = creds.user_ura().ok();' \
    '  let _ = user_ura;' \
    '}' \
    > "$tmp/cli-user-ura-ok-fallback/src/cli/commands/status.rs"
  if ( check_cli_credentials_optional_read_contract "$tmp/cli-user-ura-ok-fallback" ) >/dev/null 2>&1; then
    fail "self-test expected user_ura .ok fallback gate to fail"
  fi
  mkdir -p "$tmp/credentials-user-binding-legacy/src/daemon/persistence"
  cat >"$tmp/credentials-user-binding-legacy/src/daemon/persistence/config.rs" <<'EOF'
struct Credentials { user_id: Option<String> }
impl Credentials {
  fn join_receipt_hash(&self) -> Option<&str> { Some("hash") }
  fn username_slug(&self) -> anyhow::Result<&str> { Ok("alice") }
  fn user_id(&self) -> anyhow::Result<&str> { Ok("00000000-0000-0000-0000-000000000000") }
  fn validate_complete(&self) -> anyhow::Result<()> {
    if self.join_receipt_hash().is_none() {
      self.username_slug()?;
      self.user_id()?;
    }
    Ok(())
  }
}
#[test] fn save_credentials_accepts_federation_join_receipt_without_user_binding() {}
EOF
  if ( check_credentials_user_binding_validation_contract "$tmp/credentials-user-binding-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected credentials user-binding validation gate to fail"
  fi
  mkdir -p "$tmp/target-gate-credential-legacy/src/daemon/invocation/admission"
  cat >"$tmp/target-gate-credential-legacy/src/daemon/invocation/admission/target_gate.rs" <<'EOF'
struct LocalCredentialIdentity { realm: String, user_id: String }
fn load_local_credential_identity() -> Option<LocalCredentialIdentity> {
  crate::daemon::persistence::config::load_credentials()
    .ok()
    .and_then(|creds| {
      let user_id = creds.user_id().ok()?.to_string();
      Some(LocalCredentialIdentity { realm: creds.realm, user_id })
    })
}
EOF
  if ( check_target_gate_credential_state_contract "$tmp/target-gate-credential-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected target gate credential state gate to fail"
  fi
  mkdir -p "$tmp/local-device-identity-legacy/src/daemon/identity"
  cat >"$tmp/local-device-identity-legacy/src/daemon/identity/local_invocation.rs" <<'EOF'
pub(crate) const UNPAIRED_LOCAL_REALM: &str = "default";
pub(crate) const UNPAIRED_LOCAL_DEVICE_ID: &str = "local";

pub(crate) fn local_device_ura() -> String {
    if let Some(ura) = persisted_local_device_ura() {
        return ura;
    }
    crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|creds| crate::core::ura::device_ura(&creds.realm, &creds.node_id))
        .unwrap_or_else(|| {
            crate::core::ura::device_ura(UNPAIRED_LOCAL_REALM, UNPAIRED_LOCAL_DEVICE_ID)
        })
}

fn persisted_local_device_ura() -> Option<String> { None }
EOF
  if ( check_daemon_local_device_identity_contract "$tmp/local-device-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected local device identity default/local fallback gate to fail"
  fi
  mkdir -p "$tmp/filesystem-resource-owner-legacy/src/daemon/resources/files"
  cat >"$tmp/filesystem-resource-owner-legacy/src/daemon/resources/files/mod.rs" <<'EOF'
fn resource_ref_value() -> Value {
    let (realm, device_id) = crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|c| (c.realm, c.node_id))
        .unwrap_or_else(|| {
            (
                crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string(),
                crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_DEVICE_ID.to_string(),
            )
        });
    json!({"owner_ura": crate::core::ura::device_ura(&realm, &device_id)})
}

fn map_local_path_to_virtual_resource() {}
EOF
  if ( check_filesystem_resource_owner_contract "$tmp/filesystem-resource-owner-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected filesystem ResourceRef default/local owner gate to fail"
  fi
  mkdir -p "$tmp/federation-probe-local-identity-legacy/src/daemon/ability/builtins/integrations"
  cat >"$tmp/federation-probe-local-identity-legacy/src/daemon/ability/builtins/integrations/federation_probe.rs" <<'EOF'
pub(crate) struct LocalIdentity {
    node_id: String,
    tenant_id: String,
    paired: bool,
}

pub(crate) fn local_identity() -> LocalIdentity {
    match crate::daemon::persistence::config::load_credentials() {
        Ok(c) => LocalIdentity {
            node_id: c.node_id,
            tenant_id: c.realm,
            paired: true,
        },
        Err(_) => LocalIdentity {
            node_id: crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_DEVICE_ID.to_string(),
            tenant_id: crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string(),
            paired: false,
        },
    }
}

pub(crate) fn collect_device_view() {
    let nodes = vec![local_identity()];
}
EOF
  if ( check_federation_probe_local_identity_contract "$tmp/federation-probe-local-identity-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected federation probe default/local identity gate to fail"
  fi
  mkdir -p "$tmp/cli-device-directory-alias-legacy/src/cli/commands"
  cat >"$tmp/cli-device-directory-alias-legacy/src/cli/commands/devices.rs" <<'EOF'
fn device_platform_info(n: &Value) {
    let os = n.get("os").and_then(|v| v.as_str());
    let arch = n.get("arch").and_then(|v| v.as_str());
}

fn device_last_active(n: &Value) {
    let last_seen = n.get("last_seen_unix_ms")
        .or_else(|| n.get("last_heartbeat_unix_ms"));
}
EOF
  if ( CLI_ROOT="$tmp/cli-device-directory-alias-legacy"; check_cli_device_directory_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected CLI device directory alias gate to fail"
  fi
  mkdir -p "$tmp/ready-capability-mode-derived/src/bin" \
    "$tmp/ready-capability-mode-derived/src/daemon/boot/invocation"
  cat >"$tmp/ready-capability-mode-derived/src/bin/easynet-daemon.rs" <<'EOF'
fn ready_runtime_discovery() -> anyhow::Result<server::ControlRuntimeDiscovery> {
    let config = DaemonConfig::load(&default_config_path())?;
    let daemon_identity = ready_daemon_identity(&config)?;
    let mut capability_flags = Vec::new();
    if matches!(config.mode(), DaemonMode::Device | DaemonMode::Both) {
        capability_flags.push(flags::PAIRED_USER_RUNTIME_SIGNER.to_string());
    }
    Ok(server::ControlRuntimeDiscovery {
        invocation_endpoint: resolved_local_uds_path_with_env_override(),
        daemon_identity,
        capability_flags,
    })
}
EOF
  cat >"$tmp/ready-capability-mode-derived/src/daemon/boot/invocation/mod.rs" <<'EOF'
pub struct SessionShutdown;
pub fn start_daemon_invocation_transport() -> anyhow::Result<SessionShutdown> {
    register_paired_user_runtime_signer()?;
    Ok(SessionShutdown)
}
EOF
  if ( check_ready_capability_proof_contract "$tmp/ready-capability-mode-derived" ) >/dev/null 2>&1; then
    fail "self-test expected mode-derived ready capability gate to fail"
  fi
  mkdir -p "$tmp/cli-discover-candidate-legacy/src/cli/commands" \
    "$tmp/cli-discover-candidate-legacy/tests"
  cat >"$tmp/cli-discover-candidate-legacy/src/cli/commands/discover.rs" <<'EOF'
pub struct DiscoverReport {
  pub skipped_unparseable: usize,
  pub diagnostics: Vec<DiscoverDiagnostic>,
}
pub struct DiscoverDiagnostic { pub code: &'static str }
pub struct Candidate;
impl Candidate {
  fn from_ladder_row(row: &Value, tokens: &[String]) -> anyhow::Result<Option<Self>> {
    let scope = row.get("scope_matched").and_then(Value::as_str).unwrap_or("device");
    let callable = row.get("callable").and_then(Value::as_bool);
    Ok(None)
  }
}
EOF
  printf 'assert_eq!(weather.skipped_unparseable, 0);\n' \
    > "$tmp/cli-discover-candidate-legacy/tests/seven_axes_w1_discover_e2e.rs"
  if ( CLI_ROOT="$tmp/cli-discover-candidate-legacy"; check_cli_discover_candidate_projection_contract ) >/dev/null 2>&1; then
    fail "self-test expected CLI discover candidate projection fallback gate to fail"
  fi
  mkdir -p "$tmp/daemon-config-mode-default-legacy/src/daemon/persistence"
  cat >"$tmp/daemon-config-mode-default-legacy/src/daemon/persistence/daemon_config.rs" <<'EOF'
fn sync_existing_device_config_toml(raw: &str, creds: &Credentials) -> anyhow::Result<String> {
    use toml_edit::{value, DocumentMut, Item, Table};
    let mut doc: DocumentMut = raw.parse()?;
    let daemon_item = doc
        .as_table_mut()
        .entry("daemon")
        .or_insert_with(|| Item::Table(Table::new()));
    let daemon_table = daemon_item.as_table_mut().unwrap();
    let mode = daemon_table
        .get("mode")
        .and_then(|item| item.as_str())
        .unwrap_or("device");
    if mode != "device" {
        return Ok(raw.to_string());
    }
    Ok(raw.to_string())
}

#[cfg(test)]
mod tests {
    fn ensure_minimal_device_config_writes_default_file_and_syncs_device_fields() {}
}
EOF
  if ( CLI_ROOT="$tmp/daemon-config-mode-default-legacy"; check_daemon_config_mode_required_contract ) >/dev/null 2>&1; then
    fail "self-test expected daemon config mode default gate to fail"
  fi
  mkdir -p "$tmp/chat-session-index-default-legacy/src/daemon/persistence"
  cat >"$tmp/chat-session-index-default-legacy/src/daemon/persistence/chat_sessions.rs" <<'EOF'
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    #[serde(default)]
    pub latest: String,
    #[serde(default)]
    pub lifelong: String,
    #[serde(default)]
    pub sessions: Vec<SessionDescriptor>,
}

pub fn load_index(agent: &str) -> anyhow::Result<SessionIndex> {
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SessionIndex::default())
}

#[cfg(test)]
mod tests {
    fn index_without_lifelong_field_deserializes() {
        let idx: SessionIndex = serde_json::from_str(raw).expect("back-compat parse");
    }
}
EOF
  if ( CLI_ROOT="$tmp/chat-session-index-default-legacy"; check_chat_session_index_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected chat session index default gate to fail"
  fi
  mkdir -p "$tmp/local-agents-schema-legacy/src/daemon/persistence"
  cat >"$tmp/local-agents-schema-legacy/src/daemon/persistence/local_agents.rs" <<'EOF'
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalAgentsFile {
    #[serde(default)]
    pub host_device_agent_ura: String,
    #[serde(default)]
    pub hosted_agents: Vec<HostedAgentEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostedAgentEntry {
    pub profile: String,
    pub name: String,
    pub agent_ura: String,
    pub signing_authority: String,
    pub first_seen_at: String,
}

pub fn load() -> anyhow::Result<LocalAgentsFile> {
    return Ok(LocalAgentsFile::default());
}

#[cfg(test)]
mod tests {
    fn deserialize_tolerates_unknown_fields_for_forward_compat() {
        let f: LocalAgentsFile = serde_json::from_str(json).unwrap();
    }
}
EOF
  if ( CLI_ROOT="$tmp/local-agents-schema-legacy"; check_local_agents_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected local agents schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/profile-store-schema-legacy/src/cli/commands"
  cat >"$tmp/profile-store-schema-legacy/src/cli/commands/profile.rs" <<'EOF'
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileAccountSessionState { Authenticated, LoggedOut }

impl Default for ProfileAccountSessionState {
    fn default() -> Self { Self::LoggedOut }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileEntry {
    pub profile_name: String,
    pub realm_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_id: Option<String>,
    pub issuer: String,
    #[serde(default)]
    pub account_session: ProfileAccountSessionState,
    #[serde(default)]
    pub device_membership: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileStore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileEntry>,
}

pub(crate) fn load_store() -> anyhow::Result<ProfileStore> {
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
        return Ok(ProfileStore::default());
    }
}

#[cfg(test)]
mod tests {
    fn missing_profile_store_is_fresh_install_empty_state() {}
}
EOF
  if ( CLI_ROOT="$tmp/profile-store-schema-legacy"; check_profile_store_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected profile store schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/auth-session-owner-fact-legacy/src/cli/commands"
  cat >"$tmp/auth-session-owner-fact-legacy/src/cli/commands/auth.rs" <<'EOF'
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub hub_url: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Deserialize)]
struct AuthResp {
    token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    user: Option<UserResp>,
}

#[derive(Deserialize)]
struct UserResp {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

fn refresh_session(session: &mut AuthSession) -> anyhow::Result<()> {
    let auth: AuthResp = http_post_json(&url, &serde_json::json!({}))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test] fn missing_auth_session_is_logged_out_state() {}
}
EOF
  if ( CLI_ROOT="$tmp/auth-session-owner-fact-legacy"; check_auth_session_owner_fact_contract ) >/dev/null 2>&1; then
    fail "self-test expected auth session owner fact compatibility gate to fail"
  fi
  mkdir -p "$tmp/resources-schema-legacy/src/daemon/persistence"
  cat >"$tmp/resources-schema-legacy/src/daemon/persistence/resources.rs" <<'EOF'
/// Resource type taxonomy — RFC-005 v3.2. The wire form is a
/// lowercase string (forward-compat: a future deployment that
/// invents `gpu` lands without a schema migration), but every
/// known v1 type is enumerated here.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourcesFile {
    #[serde(default)]
    pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceEntry {
    pub resource_ura: String,
    #[serde(default)]
    pub owner_agent: String,
    pub kind: ResourceType,
    pub binding: ResourceBinding,
    pub hardware_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub metadata: Value,
    pub first_seen_at: String,
}

pub fn load() -> anyhow::Result<ResourcesFile> {
    return Ok(ResourcesFile::default());
}

#[cfg(test)]
mod tests {
    fn round_trip_through_json_preserves_fields() {}
}
EOF
  if ( CLI_ROOT="$tmp/resources-schema-legacy"; check_resources_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected resources schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/agent-spec-schema-legacy/src/core/agent"
  cat >"$tmp/agent-spec-schema-legacy/src/core/agent/spec.rs" <<'EOF'
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    pub name: String,
    pub runtime: RuntimeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    fn unknown_top_level_keys_are_ignored_for_forward_compat() {
        AgentSpec::from_toml_str(src).expect("unknown keys must be tolerated for forward compat");
    }

    fn schema_version_absent_is_rejected() {}
    fn schema_version_unknown_value_is_rejected() {}
}
EOF
  if ( CLI_ROOT="$tmp/agent-spec-schema-legacy"; check_agent_spec_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected agent spec schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/control-discovery-schema-legacy/src/daemon/control"
  cat >"$tmp/control-discovery-schema-legacy/src/daemon/control/discovery.rs" <<'EOF'
/// Contents of `~/.easynet/control.json`. The layout is frozen as
/// of PR-DAEMON; adding a field later must use `#[serde(default)]`
/// so old libs ignore it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlDiscovery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_endpoint: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_identity: Option<DaemonIdentity>,
    pub pid: u32,
    pub daemon_version: String,
    pub supported_ipc_versions: IpcVersionRange,
    #[serde(default)]
    pub capability_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonIdentity {
    pub mode: String,
    pub realm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcVersionRange {
    pub min: u16,
    pub max: u16,
}

#[cfg(test)]
mod tests {
    fn malformed_control_json_is_a_hard_error_not_silent_none() {}
    fn read_missing_file_returns_none_not_error() {}
}
EOF
  if ( CLI_ROOT="$tmp/control-discovery-schema-legacy"; check_control_discovery_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected control discovery schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/control-frame-schema-legacy/src/daemon/control"
  cat >"$tmp/control-frame-schema-legacy/src/daemon/control/frames.rs" <<'EOF'
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IncomingFrame {
    Subscribe {
        subscription_id: String,
        ability: String,
        #[serde(default)]
        args: Value,
    },
    Cancel { subscription_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutgoingFrame {
    Frame {
        subscription_id: String,
        frame: Value,
    },
    Terminal {
        subscription_id: String,
        reason: String,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    fn retired_product_incoming_variant_fails_to_parse() {}
}
EOF
  if ( CLI_ROOT="$tmp/control-frame-schema-legacy"; check_control_frame_schema_contract ) >/dev/null 2>&1; then
    fail "self-test expected control frame schema compatibility gate to fail"
  fi
  mkdir -p "$tmp/remote-subject-provenance-legacy/src/daemon/invocation/routing" \
    "$tmp/remote-subject-provenance-legacy/src/ffi/invocation"
  cat >"$tmp/remote-subject-provenance-legacy/src/daemon/invocation/routing/remote_invoke.rs" <<'EOF'
/// Named subject derivation for a remote invocation tuple.
///
/// Public compatibility may still offer ergonomic subject omission, but the
/// selected subject must be materialized under one of these labels before
/// dispatch so RF-8 cannot regress to silent callee/descriptor substitution.
pub(crate) enum RemoteInvocationSubject {
    Explicit(String),
    TargetOwnedSystem(String),
}
EOF
  printf '%s\n' \
    'RemoteInvocationSubject::TargetOwnedSystem(target.as_str().to_string());' \
    > "$tmp/remote-subject-provenance-legacy/src/ffi/invocation/mod.rs"
  if ( CLI_ROOT="$tmp/remote-subject-provenance-legacy"; check_remote_invocation_subject_provenance_contract ) >/dev/null 2>&1; then
    fail "self-test expected remote invocation subject provenance gate to fail"
  fi
  mkdir -p "$tmp/python-sdk-bytecode-index/sdk/python/easynet_sdk/__pycache__"
  (
    cd "$tmp/python-sdk-bytecode-index"
    git init -q
    printf 'tracked bytecode fixture\n' \
      > sdk/python/easynet_sdk/__pycache__/runtime.cpython-312.pyc
    git add sdk/python/easynet_sdk/__pycache__/runtime.cpython-312.pyc
  )
  if ( CLI_ROOT="$tmp/python-sdk-bytecode-index"; check_python_sdk_bytecode_index_contract ) >/dev/null 2>&1; then
    fail "self-test expected tracked Python SDK bytecode gate to fail"
  fi
  mkdir -p "$tmp/bidi-reverse-unary-terminal-state-legacy/src/daemon/invocation/bidi"
  cat >"$tmp/bidi-reverse-unary-terminal-state-legacy/src/daemon/invocation/bidi/session_escalation.rs" <<'EOF'
fn reverse_unary_reply(result: axon_sdk::pb::axon::v1::ReverseDispatchResult) -> EscalationReply {
    let state = result
                .terminal_receipt
                .as_ref()
                .map(|receipt| receipt.state)
                .unwrap_or(axon_sdk::pb::axon::v1::InvocationState::Completed as i32);
    EscalationReply::Canonical(Box::new(axon_sdk::pb::axon::v1::InvokeResponse {
        state,
        ..axon_sdk::pb::axon::v1::InvokeResponse::default()
    }))
}
EOF
  if ( check_bidi_reverse_unary_terminal_state_contract "$tmp/bidi-reverse-unary-terminal-state-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected bidi reverse unary terminal state gate to fail"
  fi
  mkdir -p "$tmp/terminal-lifecycle-args-legacy/src/daemon/ability/builtins/device_control/terminal"
  cat >"$tmp/terminal-lifecycle-args-legacy/src/daemon/ability/builtins/device_control/terminal/lifecycle.rs" <<'EOF'
/// Validation policy: drop unknown fields silently (forward
/// compatibility — a future schema addition mustn't break old callers).
fn parse_create_spec(args: &Value) -> anyhow::Result<PtyCreateSpec> {
    let cols = args.get("cols");
    let command = args.get("command").and_then(Value::as_str).map(str::to_string);
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    Ok(PtyCreateSpec {})
}

fn list_handler(args: Value) -> anyhow::Result<Value> {
    if !args.is_object() { anyhow::bail!("args must be an object"); }
    Ok(json!({}))
}

fn close_handler(args: Value) -> anyhow::Result<Value> {
    let id = args.get("session_id").and_then(Value::as_str);
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    fn parse_create_spec_drops_unknown_fields_silently() {
        parse_create_spec(&json!({"future_field_we_dont_know": true}))
            .expect("unknown fields must be tolerated");
    }
}
EOF
  if ( check_terminal_lifecycle_args_contract "$tmp/terminal-lifecycle-args-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected terminal lifecycle args compatibility gate to fail"
  fi
  mkdir -p "$tmp/session-failure-wire-facts-legacy/src/daemon/invocation/bidi/state"
  cat >"$tmp/session-failure-wire-facts-legacy/src/daemon/invocation/bidi/state/session_failure.rs" <<'EOF'
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SessionFailure {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub stage: i32,
    #[serde(default)]
    pub security_class: i32,
}
EOF
  if ( check_session_failure_wire_facts_contract "$tmp/session-failure-wire-facts-legacy" ) >/dev/null 2>&1; then
    fail "self-test expected session failure wire facts gate to fail"
  fi
  mkdir -p "$tmp/ffi-init-connect-error-legacy/src/ffi/client" "$tmp/ffi-init-connect-error-legacy/src/ffi"
  cat >"$tmp/ffi-init-connect-error-legacy/src/ffi/client/ipc.rs" <<'EOF'
pub async fn connect(path: &std::path::Path) -> anyhow::Result<IpcClient> {
    Ok(IpcClient {})
}
EOF
  cat >"$tmp/ffi-init-connect-error-legacy/src/ffi/mod.rs" <<'EOF'
pub unsafe extern "C" fn runtime_init() -> i32 {
    let msg = "FFI client: IPC version negotiation failed";
    if msg.contains("version negotiation failed") {
        return ERR_VERSION_INCOMPATIBLE;
    }
    ERR_DAEMON_DOWN
}
EOF
  if ( CLI_ROOT="$tmp/ffi-init-connect-error-legacy"; check_ffi_init_typed_connect_error_contract ) >/dev/null 2>&1; then
    fail "self-test expected FFI init typed connect error gate to fail"
  fi
  check_mcp_reflection_async_bridge_contract
  check_runtime_session_projection_accessor_contract
  check_ffi_runtime_sizing_policy_contract
  check_ffi_init_typed_connect_error_contract
  check_failure_code_default_policy_contract
  check_bidi_dispatch_default_code_policy_contract
  check_bidi_reverse_unary_terminal_state_contract
  check_cabi_bidi_cancel_reason_contract
  check_terminal_lifecycle_args_contract
  check_session_failure_wire_facts_contract
  check_active_source_contract
  check_sdk_root_runtime_description_contract
  check_go_sdk_public_ura_alias_contract
  check_go_sdk_runtime_resource_namespace_contract
  check_python_sdk_runtime_addressing_kind_contract
  check_advertise_agent_ingress_contract
  check_agent_start_model_intent_contract
  check_invocation_history_get_key_contract
  check_invocation_history_ledger_ura_contract
  check_core_ura_realm_projection_contract
  check_resolve_key_request_dto_contract
  check_invocation_history_filter_scope_contract
  check_cli_invocation_history_read_model_contract
  check_local_runtime_state_read_subject_contract
  check_runtime_state_kind_required_contract
  check_federation_realm_resolver_contract
  check_daemon_config_mode_required_contract
  check_chat_session_index_schema_contract
  check_local_agents_schema_contract
  check_profile_store_schema_contract
  check_auth_session_owner_fact_contract
  check_resources_schema_contract
  check_agent_spec_schema_contract
  check_control_discovery_schema_contract
  check_control_frame_schema_contract
  check_sdk_history_authority_subject_contract
  check_sdk_descriptor_resolution_error_vocabulary_contract
  check_sdk_ability_descriptor_not_found_vocabulary_contract
  check_sdk_runtime_identity_signer_not_found_contract
  check_sdk_easynet_provider_identity_alias_contract
  check_sdk_python_transport_stream_event_projection_contract
  check_sdk_python_invocation_result_adapter_projection_contract
  check_sdk_runtime_failure_code_contract
  check_sdk_direct_runtime_state_projection_contract
  check_sdk_direct_runtime_descriptor_not_found_contract
  check_principal_lifecycle_cli_schema_contract
  check_principal_lifecycle_store_idempotency_schema_contract
  check_auth_agents_backend_shape_contract
  check_pages_identity_credentials_contract
  check_cli_credentials_optional_read_contract
  check_credentials_user_binding_validation_contract
  check_target_gate_credential_state_contract
  check_api_key_cli_identity_contract
  check_api_key_store_schema_contract
  check_local_api_key_cache_contract
  check_runtime_trust_revoke_credentials_contract
  check_runtime_trust_user_key_inventory_scope_contract
  check_runtime_trust_user_key_write_scope_contract
  check_device_trust_sync_caller_classification_contract
  check_product_e2e_invocation_history_exact_scope_contract
  check_observe_health_contract_projection_contract
  check_admission_owner_credentials_contract
  check_shared_local_device_owner_projection_contract
  check_node_session_authority_subject_contract
  check_runtime_authority_metadata_key_neutrality_contract
  check_admission_authority_raw_wire_strict_contract
  check_admission_authority_ability_projection_contract
  check_peer_envelope_signer_subject_profile_contract
  check_local_ability_target_subject_policy_contract
  check_session_prelude_credentials_contract
  check_session_prelude_receipt_contract
  check_device_settings_loader_contract
  check_mission_traditional_target_conflict_contract
  check_mission_runtime_meta_identity_schema_contract
  check_mission_terminal_receipt_projection_contract
  check_edge_adapter_policy_contract
  check_sdk_product_neutrality_contract
  check_python_sdk_bytecode_index_contract
  check_daemon_tuple_route_contract
  check_remote_invocation_subject_provenance_contract
  check_daemon_runtime_route_inventory_contract
  check_daemon_local_device_identity_contract
  check_filesystem_resource_owner_contract
  check_federation_probe_local_identity_contract
  check_ready_capability_proof_contract
  check_daemon_local_runtime_identity_contract
  check_kernel_runtime_session_read_model_contract
  check_daemon_runtime_session_binding_contract
  check_daemon_runtime_discuss_binding_contract
  check_daemon_runtime_tenant_store_binding_contract
  check_schedule_store_current_schema_contract
  check_route_resolver_descriptor_ref_selector_contract
  check_namespace_resolver_authority_projection_contract
  check_daemon_invocation_service_descriptor_ref_route_projection_contract
  check_ffi_descriptor_runtime_owner_contract
  check_ffi_descriptor_probe_not_found_vocabulary_contract
  check_cli_discover_candidate_projection_contract
  check_ffi_invocation_json_projection_contract
  check_ffi_last_error_typed_tls_contract
  check_canonical_ability_catalog_projection_contract
  check_daemon_runtime_assembly_contract
  check_catalog_exact_runtime_key_contract
  check_federation_directory_device_projection_contract
  check_cli_device_directory_projection_contract
  check_plugin_sidecar_helper_matrix_contract
  check_retired_browser_mock_surface_contract
  check_ability_deploy_product_neutrality_contract
  check_ability_manifest_exec_absence_contract
  check_runtime_wire_target_state_contract
  check_invocation_wire_callee_target_contract
  check_local_session_descriptor_ref_test_authority_contract
  check_local_daemon_loopback_explicit_subject_contract
  check_sdk_directory_projection_fail_closed_contract
  check_sdk_principal_projection_fail_closed_contract
  check_runtime_owner_signer_custody_contract
  check_remote_invocation_signer_first_contract
  check_daemon_runtime_identity_vocabulary_contract
  check_key_custody_boundary_contract
  check_daemon_mission_eal_boundary_contract
  check_product_identity_boundary_contract
  check_ura_vocabulary_contract
  check_axon_protocol_pack_ura_vector_contract
  check_axon_normative_ura_document_contract
  check_axon_proto_ura_vocabulary_contract
  check_axon_sdk_product_neutral_ura_error_contract
  check_axon_active_ura_source_test_contract
  check_active_ura_transport_classification_contract "$ROOT/src" "$ROOT/tests" "$ROOT/include"
  ( AXON_ROOT="$tmp/axon-schema-good"; check_schema_source_derivation_contract )
  ( AXON_ROOT="$tmp/axon-benchmark-good"; check_axon_benchmark_baseline_contract )
  check_axon_product_protocol_boundary_contract
  check_axon_plain_proof_public_boundary_contract
  check_axon_rust_local_fast_signer_boundary_contract
  check_axon_process_local_signer_fallback_contract
  check_cli_rust_local_fast_signer_boundary_contract
  check_cli_signed_submission_boundary_contract
  check_receipt_proof_fact_contract
  check_java_sdk_runtime_receipt_projection_contract
  check_node_sdk_runtime_receipt_projection_contract
  check_swift_sdk_runtime_receipt_projection_contract
  check_sdk_runtime_receipt_type_state_binding_contract
  check_start_attach_user_signer_readiness_contract
  echo "canonical-runtime-convergence-v2 self-test ok"
  exit 0
fi

check_lifecycle_evidence_freshness_contract
check_manifest_contract
check_mcp_reflection_async_bridge_contract
check_runtime_session_projection_accessor_contract
check_ffi_runtime_sizing_policy_contract
check_ffi_init_typed_connect_error_contract
check_failure_code_default_policy_contract
check_bidi_dispatch_default_code_policy_contract
check_bidi_reverse_unary_terminal_state_contract
check_cabi_bidi_cancel_reason_contract
check_session_failure_wire_facts_contract
check_active_source_contract
check_sdk_root_runtime_description_contract
check_go_sdk_public_ura_alias_contract
check_go_sdk_runtime_resource_namespace_contract
check_python_sdk_runtime_addressing_kind_contract
check_advertise_agent_ingress_contract
check_agent_start_model_intent_contract
check_invocation_history_get_key_contract
check_invocation_history_ledger_ura_contract
check_core_ura_realm_projection_contract
check_resolve_key_request_dto_contract
check_invocation_history_filter_scope_contract
check_cli_invocation_history_read_model_contract
check_local_runtime_state_read_subject_contract
check_runtime_state_kind_required_contract
check_federation_realm_resolver_contract
check_profile_store_schema_contract
check_auth_session_owner_fact_contract
check_resources_schema_contract
check_agent_spec_schema_contract
check_control_discovery_schema_contract
check_control_frame_schema_contract
check_sdk_history_authority_subject_contract
check_sdk_descriptor_resolution_error_vocabulary_contract
check_sdk_ability_descriptor_not_found_vocabulary_contract
check_sdk_runtime_identity_signer_not_found_contract
check_sdk_easynet_provider_identity_alias_contract
check_sdk_python_transport_stream_event_projection_contract
check_sdk_python_invocation_result_adapter_projection_contract
check_sdk_runtime_failure_code_contract
check_sdk_direct_runtime_state_projection_contract
check_sdk_direct_runtime_descriptor_not_found_contract
check_principal_lifecycle_cli_schema_contract
check_principal_lifecycle_store_idempotency_schema_contract
check_auth_agents_backend_shape_contract
check_pages_identity_credentials_contract
check_cli_credentials_optional_read_contract
check_credentials_user_binding_validation_contract
check_target_gate_credential_state_contract
check_api_key_cli_identity_contract
check_api_key_store_schema_contract
check_local_api_key_cache_contract
check_runtime_trust_revoke_credentials_contract
check_runtime_trust_user_key_inventory_scope_contract
check_runtime_trust_user_key_write_scope_contract
check_device_trust_sync_caller_classification_contract
check_product_e2e_invocation_history_exact_scope_contract
check_observe_health_contract_projection_contract
check_admission_owner_credentials_contract
check_shared_local_device_owner_projection_contract
check_node_session_authority_subject_contract
check_runtime_authority_metadata_key_neutrality_contract
check_admission_authority_raw_wire_strict_contract
check_admission_authority_ability_projection_contract
check_peer_envelope_signer_subject_profile_contract
check_local_ability_target_subject_policy_contract
check_session_prelude_credentials_contract
check_start_attach_user_signer_readiness_contract
check_session_prelude_receipt_contract
check_device_settings_loader_contract
check_mission_traditional_target_conflict_contract
check_mission_runtime_meta_identity_schema_contract
check_mission_terminal_receipt_projection_contract
check_edge_adapter_policy_contract
check_sdk_product_neutrality_contract
check_python_sdk_bytecode_index_contract
check_daemon_tuple_route_contract
check_remote_invocation_subject_provenance_contract
check_daemon_runtime_route_inventory_contract
check_daemon_local_device_identity_contract
check_filesystem_resource_owner_contract
check_federation_probe_local_identity_contract
check_ready_capability_proof_contract
check_daemon_local_runtime_identity_contract
check_kernel_runtime_session_read_model_contract
check_daemon_runtime_session_binding_contract
check_daemon_runtime_discuss_binding_contract
check_daemon_runtime_tenant_store_binding_contract
check_schedule_store_current_schema_contract
check_retired_federation_directory_v1_stream_contract
check_route_resolver_descriptor_ref_selector_contract
check_namespace_resolver_authority_projection_contract
check_daemon_invocation_service_descriptor_ref_route_projection_contract
check_ffi_descriptor_runtime_owner_contract
check_ffi_descriptor_probe_not_found_vocabulary_contract
check_cli_discover_candidate_projection_contract
check_ffi_invocation_json_projection_contract
check_ffi_last_error_typed_tls_contract
check_canonical_ability_catalog_projection_contract
check_daemon_runtime_assembly_contract
check_catalog_exact_runtime_key_contract
check_federation_directory_device_projection_contract
check_cli_device_directory_projection_contract
check_plugin_sidecar_helper_matrix_contract
check_retired_browser_mock_surface_contract
check_ability_deploy_product_neutrality_contract
check_ability_manifest_exec_absence_contract
check_runtime_wire_target_state_contract
check_invocation_wire_callee_target_contract
check_local_session_descriptor_ref_test_authority_contract
check_local_daemon_loopback_explicit_subject_contract
check_sdk_directory_projection_fail_closed_contract
check_sdk_runtime_recovery_report_fail_closed_contract
check_sdk_principal_projection_fail_closed_contract
check_runtime_owner_signer_custody_contract
check_remote_invocation_signer_first_contract
check_daemon_runtime_identity_vocabulary_contract
check_key_custody_boundary_contract
check_daemon_mission_eal_boundary_contract
check_product_identity_boundary_contract
check_ura_vocabulary_contract
check_axon_protocol_pack_ura_vector_contract
check_axon_normative_ura_document_contract
check_axon_proto_ura_vocabulary_contract
check_axon_sdk_product_neutral_ura_error_contract
check_axon_active_ura_source_test_contract
check_active_ura_transport_classification_contract "$ROOT/src" "$ROOT/tests" "$ROOT/include"
check_schema_source_derivation_contract
check_axon_benchmark_baseline_contract
check_axon_product_protocol_boundary_contract
check_axon_plain_proof_public_boundary_contract
check_axon_rust_local_fast_signer_boundary_contract
check_axon_process_local_signer_fallback_contract
check_cli_rust_local_fast_signer_boundary_contract
check_cli_signed_submission_boundary_contract
check_receipt_proof_fact_contract
check_java_sdk_runtime_receipt_projection_contract
check_node_sdk_runtime_receipt_projection_contract
check_swift_sdk_runtime_receipt_projection_contract
check_sdk_runtime_receipt_type_state_binding_contract
echo "canonical-runtime-convergence-v2: OK"
