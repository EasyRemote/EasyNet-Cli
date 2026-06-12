#!/usr/bin/env bash
#
# Guard OpenAI-compatible model ids to canonical chat Ability URAs.

set -euo pipefail

ROOT="${CHECK_OPENAI_MODEL_ABILITY_URA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-openai-model-ability-ura: $*" >&2
    exit 1
}

LLM_RS="src/facade/cli/llm_api.rs"
OPENAI_RS="src/runtime/agents/openai_compat_ability.rs"
RUNTIME_AGENTS_RS="src/runtime/agents/catalog_metadata.rs"  # descriptor source moved here in the T4.5 mod.rs split
DOC_MD="docs/PAGES_AND_LLM_API.md"
CHAT_TOML="abilities/system/openai.chat_completions.ability.toml"
MODELS_TOML="abilities/system/openai.list_models.ability.toml"

for file in "$LLM_RS" "$OPENAI_RS" "$RUNTIME_AGENTS_RS" "$DOC_MD" "$CHAT_TOML" "$MODELS_TOML"; do
    [[ -f "$file" ]] || fail "missing $file"
done

grep -q 'value_name = "ABILITY_URA"' "$LLM_RS" \
    || fail "llm-api --model must advertise ABILITY_URA, not a bare model name"

grep -q 'validate_chat_model_id(&m)' "$LLM_RS" \
    || fail "llm-api must validate explicit --model before invoking the adapter"

grep -q 'pub(crate) fn validate_chat_model_id' "$OPENAI_RS" \
    || fail "OpenAI adapter must expose one canonical model-id validator"

grep -q 'model must be a valid canonical Ability URA' "$OPENAI_RS" \
    || fail "OpenAI adapter must reject non-URA model ids"

grep -q 'canonical agent-owned chat Ability URA' "$RUNTIME_AGENTS_RS" \
    || fail "runtime descriptor source must document Ability-URA model ids"

grep -q 'input_schema.properties.request.properties.model' "$CHAT_TOML" \
    || fail "chat_completions descriptor must document request.model"

grep -q 'Canonical agent-owned chat Ability URA' "$CHAT_TOML" \
    || fail "chat_completions descriptor must describe model as Ability URA"

grep -q 'canonical agent-owned chat Ability URA' "$MODELS_TOML" \
    || fail "list_models descriptor must say Model.id is the Ability URA"

bad="$(
    grep -nE 'model.?[:=].?codex|model=.codex|--model <name>|model\\":\\"codex|model='\''codex|\"model\": \"codex\"|model=codex|Model name|Model name|bare model name' \
        "$LLM_RS" "$DOC_MD" "$CHAT_TOML" "$MODELS_TOML" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "OpenAI model surface still advertises retired bare model ids:
$bad"
fi

echo "check-openai-model-ability-ura: ok"
