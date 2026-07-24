#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

ISSUER="src/support/platform/local_invoke.rs"
TARGETS=(
  "src/cli/commands/ability_record.rs"
  "src/cli/commands/discover.rs"
  "src/cli/commands/doctor.rs"
  "src/cli/commands/groups/mcp.rs"
  "src/cli/commands/groups/device.rs"
  "src/cli/commands/status.rs"
  "src/cli/daemon_client/ability_catalog.rs"
  "src/cli/commands/groups/invocation.rs"
  "src/cli/commands/invocation_watch.rs"
  "src/cli/commands/user_signing_identity.rs"
  "src/daemon/ability/catalog/profiles/mcp.rs"
)

[[ -f "$ISSUER" ]] || fail "missing $ISSUER"

if ! rg -n 'struct LocalRuntimeStateReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime-state reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeStateReadSubject' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must own an explicit read-subject value object"
fi

if ! rg -n 'runtime-state/read' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must bind a dedicated runtime-state resource subject"
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

if rg -n 'local_invocation::local_daemon_ura\(\)|local_invocation::local_device_ura\(\)|UNPAIRED_LOCAL_REALM|UNPAIRED_LOCAL_DEVICE_ID' "$ISSUER"; then
  fail "runtime-state read issuer must not fall back to daemon/device/default subjects"
fi

if ! rg -n 'runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test user-owned resource subject projection"
fi

if ! rg -n 'runtime_state_read_subject_rejects_missing_user_id_before_device_fallback' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing user id as fail-closed"
fi

if ! rg -n 'runtime_state_read_subject_requires_ready_signer_capability' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing Ready signer capability"
fi

if ! rg -n 'runtime_state_read_subject_rejects_stale_daemon_identity' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test stale daemon identity rejection"
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

AGENT_GATEWAY="src/cli/daemon_client/agent_gateway.rs"
AGENT_VIEW="src/cli/daemon_client/agent_view.rs"
AGENT_PUBLISH="src/cli/commands/agent/publish.rs"
LLM_API="src/cli/commands/llm_api.rs"
SKILL_CLI="src/cli/commands/skill.rs"
API_KEY_CLI="src/cli/commands/api_key_cli.rs"

[[ -f "$AGENT_GATEWAY" ]] || fail "missing $AGENT_GATEWAY"
[[ -f "$AGENT_VIEW" ]] || fail "missing $AGENT_VIEW"
[[ -f "$AGENT_PUBLISH" ]] || fail "missing $AGENT_PUBLISH"
[[ -f "$LLM_API" ]] || fail "missing $LLM_API"
[[ -f "$SKILL_CLI" ]] || fail "missing $SKILL_CLI"
[[ -f "$API_KEY_CLI" ]] || fail "missing $API_KEY_CLI"

if ! rg -n 'trait AgentStateReadGateway' "$AGENT_GATEWAY" >/dev/null; then
  fail "agent.list must have a dedicated AgentStateReadGateway"
fi

if ! rg -n 'LocalRuntimeStateReadIssuer::invoke' "$AGENT_GATEWAY" >/dev/null; then
  fail "production AgentStateReadGateway must use LocalRuntimeStateReadIssuer"
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

if ! rg -n '\binvoke_local_ability\s*\(\s*"openai\.chat_completions"' "$LLM_API" >/dev/null; then
  fail "llm-api chat completions must remain on the action invoke path"
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

if ! rg -n -U 'LocalDaemonSystemAbilityIssuer::invoke_root_for_subject\(\s*ability,\s*args,\s*&subject_ura' "$SKILL_CLI" >/dev/null; then
  fail "skill mutations must use explicit local daemon system issuer subject"
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

echo "check-runtime-state-read-subject-boundary: ok"
