#!/usr/bin/env bash
#
# Contract tests for scripts/check-openai-model-ability-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-openai-model-ability-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli" "$sandbox/src/runtime/system_ability_catalog" "$sandbox/src/runtime/system_abilities/integrations" "$sandbox/docs" "$sandbox/ability-descriptors/system"
    cp "$REPO_ROOT/src/cli/llm_api.rs" "$sandbox/src/cli/llm_api.rs"
    cp "$REPO_ROOT/src/runtime/system_abilities/integrations/openai_compat.rs" "$sandbox/src/runtime/system_abilities/integrations/openai_compat.rs"
    cp "$REPO_ROOT/src/runtime/system_ability_catalog/catalog_metadata.rs" "$sandbox/src/runtime/system_ability_catalog/catalog_metadata.rs"
    cp "$REPO_ROOT/docs/PAGES_AND_LLM_API.md" "$sandbox/docs/PAGES_AND_LLM_API.md"
    cp "$REPO_ROOT/ability-descriptors/system/openai.chat_completions.ability.toml" "$sandbox/ability-descriptors/system/openai.chat_completions.ability.toml"
    cp "$REPO_ROOT/ability-descriptors/system/openai.list_models.ability.toml" "$sandbox/ability-descriptors/system/openai.list_models.ability.toml"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_OPENAI_MODEL_ABILITY_URA_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: OpenAI model Ability URA contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/value_name = "ABILITY_URA"/value_name = "MODEL"/' "$SB/src/cli/llm_api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired model placeholder should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/crate::runtime::system_abilities::integrations::openai_compat::validate_chat_model_id\(&m\)\?;\n        //' "$SB/src/cli/llm_api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing explicit model validation should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/easynet:\/\/\/r\/easynet\.run\/ability\/alice\.codex\.chat/codex/g' "$SB/docs/PAGES_AND_LLM_API.md"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired doc model id should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/input_schema\.properties\.request\.properties\.model/input_schema.properties.request.properties.retired_model/' "$SB/ability-descriptors/system/openai.chat_completions.ability.toml"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing descriptor model property should exit 1 (got $rc)"

echo "test_check_openai_model_ability_ura.sh: all cases passed"
