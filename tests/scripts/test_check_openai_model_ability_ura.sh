#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-openai-model-ability-ura.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-openai-model-ability-ura.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

descriptor_path() {
    local root="$1"
    local ability="$2"
    local paths count
    paths="$(find "$root/ability-descriptors/system" -type f -name "${ability}.ability.toml" -print | sort)"
    count="$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')"
    [[ "$count" == "1" ]] || fail "expected exactly one descriptor for $ability, found $count"
    printf '%s\n' "$paths"
}

copy_descriptor() {
    local sandbox="$1"
    local ability="$2"
    local source rel
    source="$(descriptor_path "$REPO_ROOT" "$ability")"
    rel="${source#$REPO_ROOT/}"
    mkdir -p "$sandbox/$(dirname "$rel")"
    cp "$source" "$sandbox/$rel"
}

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli/commands" "$sandbox/src/daemon/ability/catalog" "$sandbox/src/daemon/ability/builtins/integrations" "$sandbox/docs" "$sandbox/ability-descriptors/system"
    cp "$REPO_ROOT/src/cli/commands/llm_api.rs" "$sandbox/src/cli/commands/llm_api.rs"
    cp "$REPO_ROOT/src/daemon/ability/builtins/integrations/openai_compat.rs" "$sandbox/src/daemon/ability/builtins/integrations/openai_compat.rs"
    cp "$REPO_ROOT/src/daemon/ability/catalog/catalog_metadata.rs" "$sandbox/src/daemon/ability/catalog/catalog_metadata.rs"
    cp "$REPO_ROOT/docs/PAGES_AND_LLM_API.md" "$sandbox/docs/PAGES_AND_LLM_API.md"
    copy_descriptor "$sandbox" openai.chat_completions
    copy_descriptor "$sandbox" openai.list_models
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
perl -0pi -e 's/value_name = "ABILITY_URA"/value_name = "MODEL"/' "$SB/src/cli/commands/llm_api.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired model placeholder should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/crate::daemon::ability::builtins::integrations::openai_compat::validate_chat_model_id\(&m\)\?;\n        //' "$SB/src/cli/commands/llm_api.rs"
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
CHAT_TOML="$(descriptor_path "$SB" openai.chat_completions)"
perl -0pi -e 's/input_schema\.properties\.request\.properties\.model/input_schema.properties.request.properties.retired_model/' "$CHAT_TOML"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing descriptor model property should exit 1 (got $rc)"

echo "test_check_openai_model_ability_ura.sh: all cases passed"
