#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
PYTHON_BIN="${PYTHON_BIN:-python3}"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

ISSUER="src/support/platform/local_invoke.rs"
TARGETS=(
  "src/cli/commands/discover.rs"
  "src/cli/commands/doctor.rs"
  "src/cli/commands/groups/device.rs"
  "src/cli/commands/status.rs"
  "src/cli/commands/invocation_watch.rs"
  "src/cli/commands/user_signing_identity.rs"
)

OPERATIONAL_TARGETS=(
  "src/cli/commands/groups/mcp.rs"
)

GOVERNANCE_TARGETS=(
  "src/cli/commands/groups/invocation.rs"
)

CATALOGUE_TARGETS=(
  "src/cli/daemon_client/ability_catalog.rs"
  "src/cli/commands/ability_record.rs"
  "src/daemon/ability/catalog/profiles/mcp.rs"
)

[[ -f "$ISSUER" ]] || fail "missing $ISSUER"

if ! rg -n 'struct LocalRuntimeStateReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime-state reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeCatalogueReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime catalogue reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeGovernanceReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime governance reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeOperationalReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime operational reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeStateReadAttachment' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must own explicit runtime attachment state"
fi

if ! rg -n 'RuntimeStateReadSubject::new' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must delegate subject construction to core identity"
fi

if ! rg -n '/// Invoke a canonical local target with public-ingress tuple facts\.' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer section terminator is missing"
fi

runtime_state_issuer_section="$(
  sed -n '/pub struct LocalRuntimeStateReadIssuer/,/\/\/\/ Invoke a canonical local target with public-ingress tuple facts\./p' "$ISSUER"
)"

if rg -n 'const RESOURCE_PATH:.*runtime-state/read|resource_dot_ura\(realm|struct LocalRuntimeStateReadSubject' "$ISSUER"; then
  fail "runtime-state read issuer must not own a duplicate subject grammar"
fi

if ! rg -n 'user-owned Resource URA' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer contract must describe user-owned Resource subjects"
fi

if rg -n 'issuer binds every read to the daemon identity|daemon identity published by control discovery' "$ISSUER"; then
  fail "runtime-state read issuer contract must not describe daemon identity subjects"
fi

if ! rg -n 'persistence::config::load_credentials\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must derive subject ownership from paired credentials"
fi

if ! rg -n '\.user_id\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must require a paired user id"
fi

if rg -n 'from_credentials_file' "$ISSUER"; then
  fail "runtime-state read issuer must not issue subjects from credentials alone"
fi

if ! rg -n 'trait RuntimeStateReadSignerCustody' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must have an explicit signer-custody proof seam"
fi

if ! rg -n 'prove_runtime_caller_signer_custody' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must prove live caller signer custody before issuing a read subject"
fi

if ! rg -n 'from_runtime_attachment_file' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must issue subjects from active runtime attachment, not raw credentials"
fi

if ! rg -n 'daemon Ready discovery' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must bind to daemon Ready discovery"
fi

if ! rg -n 'PAIRED_USER_RUNTIME_SIGNER' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must require the paired-user runtime signer capability"
fi

if ! rg -n 'identity\.realm\.trim\(\).*credentials\.realm_str\(\)\.trim\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must reject stale realm attachment"
fi

if ! rg -n 'identity\.node_id\.as_deref\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must reject stale node attachment"
fi

if printf '%s\n' "$runtime_state_issuer_section" \
  | rg -n 'local_invocation::local_daemon_ura\(\)|local_invocation::local_device_ura\(\)|UNPAIRED_LOCAL_REALM|UNPAIRED_LOCAL_DEVICE_ID'; then
  fail "runtime-state read issuer must not fall back to daemon/device/default subjects"
fi

if ! rg -n 'runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test user-owned resource subject projection"
fi

if ! rg -n 'runtime_state_read_subject_rejects_missing_user_id_before_device_fallback' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing user id as fail-closed"
fi

if printf '%s\n' "$runtime_state_issuer_section" | rg -n 'fn from_credentials\('; then
  fail "runtime-state read subject must not keep a credentials-only constructor"
fi

if ! sed -n '/fn runtime_state_read_subject_rejects_missing_user_id_before_device_fallback/,/^    }/p' "$ISSUER" \
  | rg -n 'from_runtime_attachment\(' >/dev/null; then
  fail "runtime-state read missing-user test must exercise the runtime attachment constructor"
fi

if ! rg -n 'runtime_state_read_subject_requires_ready_signer_capability' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing Ready signer capability"
fi

if ! rg -n 'runtime_state_read_subject_rejects_stale_runtime_attachment' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test stale runtime attachment rejection"
fi

if rg -n 'active daemon identity|stale daemon identity' "$ISSUER"; then
  fail "runtime-state read subject tests must describe stale runtime attachment, not daemon identity"
fi

if ! rg -n 'runtime_state_read_subject_rejects_missing_live_signer_custody' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test live signer custody rejection"
fi

for target in "${TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'LocalRuntimeStateReadIssuer::invoke' "$target" >/dev/null; then
    fail "$target must enter local runtime through LocalRuntimeStateReadIssuer"
  fi
  if rg -n '\binvoke_local_ability\s*\(' "$target"; then
    fail "$target must not use generic invoke_local_ability for runtime-state reads"
  fi
done

for target in "${OPERATIONAL_TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'LocalRuntimeOperationalReadIssuer::observe_health' "$target" >/dev/null; then
    fail "$target must enter local runtime operational health through LocalRuntimeOperationalReadIssuer"
  fi
  if rg -n '\binvoke_local_ability\s*\(' "$target"; then
    fail "$target must not use generic invoke_local_ability for runtime operational reads"
  fi
done

"$PYTHON_BIN" - "$ISSUER" <<'PY'
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
match = re.search(
    r"impl LocalRuntimeOperationalReadIssuer \{(?P<body>.*?)\n\}",
    text,
    re.S,
)
if not match:
    raise SystemExit("LocalRuntimeOperationalReadIssuer impl is missing")
body = match.group("body")
if "pub fn invoke(" in body or "pub fn invoke_timeout(" in body:
    raise SystemExit(
        "LocalRuntimeOperationalReadIssuer must expose observe_health methods, not generic invoke methods"
    )
for required in ("pub fn observe_health(", "pub fn observe_health_timeout("):
    if required not in body:
        raise SystemExit(f"LocalRuntimeOperationalReadIssuer missing {required}")
PY

for target in "${GOVERNANCE_TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'LocalRuntimeGovernanceReadIssuer::invoke' "$target" >/dev/null; then
    fail "$target must enter local runtime through LocalRuntimeGovernanceReadIssuer"
  fi
  if rg -n '\binvoke_local_ability\s*\(' "$target"; then
    fail "$target must not use generic invoke_local_ability for runtime governance reads"
  fi
done

for target in "${CATALOGUE_TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'LocalRuntimeCatalogueReadIssuer::invoke' "$target" >/dev/null; then
    fail "$target must enter local runtime catalogue through LocalRuntimeCatalogueReadIssuer"
  fi
  if rg -n '\binvoke_local_ability\s*\(' "$target"; then
    fail "$target must not use generic invoke_local_ability for runtime catalogue reads"
  fi
done

AGENT_GATEWAY="src/cli/daemon_client/agent_gateway.rs"
AGENT_VIEW="src/cli/daemon_client/agent_view.rs"
AGENT_PUBLISH="src/cli/commands/agent/publish.rs"
LLM_API="src/cli/commands/llm_api.rs"
SKILL_CLI="src/cli/commands/skill.rs"
API_KEY_CLI="src/cli/commands/api_key_cli.rs"
ABILITY_GROUP="src/cli/commands/groups/ability.rs"

[[ -f "$AGENT_GATEWAY" ]] || fail "missing $AGENT_GATEWAY"
[[ -f "$AGENT_VIEW" ]] || fail "missing $AGENT_VIEW"
[[ -f "$AGENT_PUBLISH" ]] || fail "missing $AGENT_PUBLISH"
[[ -f "$LLM_API" ]] || fail "missing $LLM_API"
[[ -f "$SKILL_CLI" ]] || fail "missing $SKILL_CLI"
[[ -f "$API_KEY_CLI" ]] || fail "missing $API_KEY_CLI"
[[ -f "$ABILITY_GROUP" ]] || fail "missing $ABILITY_GROUP"

if ! rg -n 'trait AgentStateReadGateway' "$AGENT_GATEWAY" >/dev/null; then
  fail "agent.list must have a dedicated AgentStateReadGateway"
fi

if ! rg -n 'LocalRuntimeStateReadIssuer::invoke' "$AGENT_GATEWAY" >/dev/null; then
  fail "production AgentStateReadGateway must use LocalRuntimeStateReadIssuer"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity' "$AGENT_GATEWAY" >/dev/null; then
  fail "production AgentCommandGateway must delegate local daemon identity subject policy to the system issuer"
fi

if rg -n 'LocalDaemonSystemAbilityIssuer::local_daemon_identity_subject_ura|let subject_ura\s*=' "$AGENT_GATEWAY"; then
  fail "production AgentCommandGateway must not derive local daemon subject in the product gateway"
fi

if rg -n '\binvoke_local_ability\s*\(' "$AGENT_GATEWAY"; then
  fail "agent command gateway must not use generic invoke_local_ability"
fi

if ! rg -n 'AgentStateReadGateway' "$AGENT_VIEW" >/dev/null; then
  fail "agent view must depend on AgentStateReadGateway, not AgentCommandGateway"
fi

if rg -n 'AgentCommandGateway|agent_command_gateway|\.invoke\s*\(\s*"agent\.list"' "$AGENT_VIEW"; then
  fail "agent view must not read agent.list through the command gateway"
fi

if rg -n '\.invoke\s*\(\s*"(agent\.list|meta\.list_abilities)"' "$AGENT_PUBLISH"; then
  fail "agent publish read projections must not use the command gateway"
fi

if ! rg -n 'LocalRuntimeStateReadIssuer::invoke\("openai\.list_models"' "$LLM_API" >/dev/null; then
  fail "llm-api model catalogue discovery must use LocalRuntimeStateReadIssuer"
fi

if rg -n '\binvoke_local_ability\s*\(\s*"openai\.list_models"' "$LLM_API"; then
  fail "llm-api model catalogue discovery must not use generic invoke_local_ability"
fi

if rg -n '\binvoke_local_ability\s*\(' "$LLM_API"; then
  fail "llm-api must not use generic invoke_local_ability"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity' "$LLM_API" >/dev/null; then
  fail "llm-api chat completions must delegate local daemon identity subject policy to the system issuer"
fi

if ! rg -n -F 'invoke_openai_chat_completions(adapter_args)' "$LLM_API" >/dev/null; then
  fail "llm-api chat completions must remain on the explicit action issuer path"
fi

if ! rg -n -U 'LocalRuntimeStateReadIssuer::invoke\(\s*"skill\.list"' "$SKILL_CLI" >/dev/null; then
  fail "skill.list must use LocalRuntimeStateReadIssuer"
fi

if rg -n -U '\binvoke_local_ability\s*\(\s*"skill\.list"' "$SKILL_CLI"; then
  fail "skill.list must not use generic invoke_local_ability"
fi

for action in skill.install skill.upgrade skill.remove; do
  if ! rg -n -U "invoke_daemon_skill_mutation\\s*\\(\\s*\"$action\"" "$SKILL_CLI" >/dev/null; then
    fail "$action must remain on the explicit action issuer path"
  fi
done

if rg -n -U '\binvoke_local_ability\s*\(' "$SKILL_CLI"; then
  fail "skill CLI must not use generic invoke_local_ability"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity\(ability, args\)' "$SKILL_CLI" >/dev/null; then
  fail "skill mutations must delegate local daemon identity subject policy to the system issuer"
fi

if ! rg -n -U 'LocalRuntimeStateReadIssuer::invoke\(&ability,\s*(serde_json::)?json!\(\{\}\)' "$API_KEY_CLI" >/dev/null; then
  fail "api-key list must use LocalRuntimeStateReadIssuer"
fi

if rg -n -U '\binvoke_local_ability\s*\(&ability,\s*(serde_json::)?json!\(\{\}\)' "$API_KEY_CLI"; then
  fail "api-key list must not use generic invoke_local_ability"
fi

if rg -n -U '\binvoke_local_ability\s*\(' "$API_KEY_CLI"; then
  fail "api-key CLI must not use generic invoke_local_ability"
fi

if ! rg -n -U 'LocalDaemonSystemAbilityIssuer::invoke_root_for_subject\(\s*ability,\s*args,\s*&principal\.subject_ura' "$API_KEY_CLI" >/dev/null; then
  fail "api-key mutations must use explicit local daemon system issuer subject"
fi

if ! rg -n -U 'invoke_api_key_manage\(&principal,\s*&ability,\s*args\)' "$API_KEY_CLI" >/dev/null; then
  fail "api-key create must remain on the explicit action issuer path"
fi

if ! rg -n -U 'invoke_api_key_manage\(&principal,\s*&ability,\s*(serde_json::)?json!\(\{\s*"id_prefix"' "$API_KEY_CLI" >/dev/null; then
  fail "api-key revoke must remain on the explicit action issuer path"
fi

if rg -n -U '\binvoke_local_ability\s*\(' "$ABILITY_GROUP"; then
  fail "ability CLI must not use generic invoke_local_ability"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity\("ability\.uninstall", args\)' "$ABILITY_GROUP" >/dev/null; then
  fail "ability.uninstall must delegate local daemon identity subject policy to the system issuer"
fi

if ! rg -n -F 'invoke_ability_uninstall(ability_uninstall_payload(&args))' "$ABILITY_GROUP" >/dev/null; then
  fail "ability.uninstall must remain on the explicit action issuer path"
fi

echo "check-runtime-state-read-subject-boundary: ok"
