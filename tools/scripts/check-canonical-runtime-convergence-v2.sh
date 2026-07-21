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

  "$PYTHON_BIN" - "$identity" "$config" "$daemon" "$smoke" <<'PY'
import sys
from pathlib import Path

identity = Path(sys.argv[1]).read_text()
config = Path(sys.argv[2]).read_text() if Path(sys.argv[2]).exists() else ""
daemon = Path(sys.argv[3]).read_text() if Path(sys.argv[3]).exists() else ""
smoke = Path(sys.argv[4]).read_text() if Path(sys.argv[4]).exists() else ""

if "pub fn from_env() -> Self" in identity:
    raise SystemExit("pages_identity_retains_infallible_from_env")
if "pub fn try_from_env() -> anyhow::Result<Self>" not in identity:
    raise SystemExit("pages_identity_missing_fallible_env_resolver")
for retired in (
    "load_credentials()\n                    .ok()",
    "load_credentials().ok()",
    'parse::<u16>().ok()',
    ".and_then(|s| s.parse::<u16>().ok())",
):
    if retired in identity:
        raise SystemExit(f"pages_identity_retired_fallback:{retired}")
for required in (
    "load_credentials_optional()?",
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
):
    if test not in identity and test not in config:
        raise SystemExit(f"missing_pages_identity_credentials_test:{test}")
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

check_daemon_runtime_route_inventory_contract() {
  bash "$ROOT/tools/scripts/check-architecture-convergence.sh" >/dev/null
}

check_daemon_runtime_assembly_contract() {
  local cli_root="${CLI_ROOT:-$ROOT}"
  local runtime_binding="$cli_root/src/daemon/invocation/dispatch/deps.rs"
  local invocation_service="$cli_root/src/daemon/invocation/dispatch/daemon_invocation_service.rs"

  if rg -n 'CanonicalOnly|pub fn with_local_runtime\s*\(' \
    "$runtime_binding" "$invocation_service"; then
    fail "daemon Invocation transport retains a bare LocalRuntime construction path"
  fi
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
    "java": "run.easynet.daemon.provider.easynet.pluginexec",
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
        "sdk/java/src/main/java/run/easynet/daemon/provider/easynet/pluginexec/SidecarRuntime.java",
        "sdk/java/src/main/java/run/easynet/daemon/provider/easynet/pluginexec/SidecarInvocation.java",
        "sdk/java/src/test/java/run/easynet/daemon/provider/easynet/pluginexec/SidecarRuntimeTest.java",
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
if "SidecarRuntime.serve" not in text or "run.easynet.daemon.provider.easynet.pluginexec" not in text:
    raise SystemExit("plugin_java_template_missing_provider_helper")
if "serveExecPlugin" not in text:
    raise SystemExit("plugin_node_template_missing_provider_helper")
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

diagnostics = ffi.split("fn runtime_meta_descriptor_catalog_entries", 1)[1].split(
    "fn descriptor_catalog_entry_from_descriptor", 1
)[0]
if ".bind(invocation)" not in diagnostics or ".invoke(signed)" not in diagnostics:
    raise SystemExit("diagnostics_bypasses_session_invocation_authority")
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
  check_active_source_contract
  check_go_sdk_public_ura_alias_contract
  check_advertise_agent_ingress_contract
  check_agent_start_model_intent_contract
  check_invocation_history_get_key_contract
  check_principal_lifecycle_cli_schema_contract
  check_auth_agents_backend_shape_contract
  check_pages_identity_credentials_contract
  check_local_api_key_cache_contract
  check_runtime_trust_revoke_credentials_contract
  check_admission_owner_credentials_contract
  check_device_settings_loader_contract
  check_mission_traditional_target_conflict_contract
  check_edge_adapter_policy_contract
  check_sdk_product_neutrality_contract
  check_daemon_tuple_route_contract
  check_daemon_runtime_route_inventory_contract
  check_daemon_runtime_assembly_contract
  check_plugin_sidecar_helper_matrix_contract
  check_key_custody_boundary_contract
  check_daemon_mission_eal_boundary_contract
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
  echo "canonical-runtime-convergence-v2 self-test ok"
  exit 0
fi

check_lifecycle_evidence_freshness_contract
check_manifest_contract
check_active_source_contract
check_go_sdk_public_ura_alias_contract
check_advertise_agent_ingress_contract
check_agent_start_model_intent_contract
check_invocation_history_get_key_contract
check_principal_lifecycle_cli_schema_contract
check_auth_agents_backend_shape_contract
check_pages_identity_credentials_contract
check_local_api_key_cache_contract
check_runtime_trust_revoke_credentials_contract
check_admission_owner_credentials_contract
check_device_settings_loader_contract
check_mission_traditional_target_conflict_contract
check_edge_adapter_policy_contract
check_sdk_product_neutrality_contract
check_daemon_tuple_route_contract
check_daemon_runtime_route_inventory_contract
check_daemon_runtime_assembly_contract
check_plugin_sidecar_helper_matrix_contract
check_key_custody_boundary_contract
check_daemon_mission_eal_boundary_contract
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
echo "canonical-runtime-convergence-v2: OK"
